use axum::{
    Json,
    body::Bytes,
    extract::{
        DefaultBodyLimit, FromRequest, Path, Query, Request, State, rejection::PathRejection,
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AppState, db,
    error::AppError,
    models::Paste,
    services::{password, paste},
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlainOptions {
    title: Option<String>,
    language: Option<String>,
    expires_in: Option<String>,
    password: Option<String>,
    #[serde(default)]
    burn_after_read: bool,
}

#[derive(Serialize)]
struct CreatedPaste {
    id: String,
    url: String,
    raw_url: String,
    expires_at: Option<i64>,
}

#[derive(Serialize)]
struct ReadPaste<'a> {
    id: &'a str,
    title: &'a Option<String>,
    content: &'a str,
    language: &'a str,
    created_at: i64,
    expires_at: Option<i64>,
    burn_after_read: bool,
}

fn authorize_write(headers: &HeaderMap, expected: Option<&str>) -> Result<(), AppError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(AppError::Unauthorized);
    }
    let (scheme, token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .ok_or(AppError::Unauthorized)?;
    let actual_hash = Sha256::digest(token.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    let difference = actual_hash
        .iter()
        .zip(expected_hash.iter())
        .fold(0_u8, |difference, (actual, expected)| {
            difference | (actual ^ expected)
        });
    if !scheme.eq_ignore_ascii_case("Bearer") || difference != 0 {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

fn parse_input(plain: bool, uri: &Uri, body: &[u8]) -> Result<paste::CreatePaste, AppError> {
    if !plain {
        if uri.query().is_some_and(|query| !query.is_empty()) {
            return Err(AppError::BadRequest(
                "JSON options belong in the request body",
            ));
        }
        return serde_json::from_slice(body).map_err(|_| AppError::BadRequest("Invalid JSON body"));
    }
    let Query(options) = Query::<PlainOptions>::try_from_uri(uri)
        .map_err(|_| AppError::BadRequest("Invalid query parameters"))?;
    let content = std::str::from_utf8(body)
        .map_err(|_| AppError::BadRequest("Content must be UTF-8"))?
        .to_owned();
    Ok(paste::CreatePaste {
        content,
        title: options.title,
        language: options.language,
        expires_in: options.expires_in,
        password: options.password,
        burn_after_read: options.burn_after_read,
    })
}

pub async fn create(
    State(state): State<AppState>,
    mut request: Request,
) -> Result<Response, AppError> {
    authorize_write(request.headers(), state.config.api_token.as_deref())?;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::BadRequest("Content-Type is required"))?;
    let mut parts = content_type.split(';');
    let media = parts.next().unwrap_or_default().trim();
    let plain = media.eq_ignore_ascii_case("text/plain");
    if !plain && !media.eq_ignore_ascii_case("application/json") {
        return Err(AppError::BadRequest("Use application/json or text/plain"));
    }
    for parameter in parts {
        if let Some((key, value)) = parameter.trim().split_once('=')
            && key.trim().eq_ignore_ascii_case("charset")
            && !value.trim().trim_matches('"').eq_ignore_ascii_case("utf-8")
        {
            return Err(AppError::BadRequest("Content must be UTF-8"));
        }
    }
    let limit = if plain {
        state.config.max_content_bytes
    } else {
        state
            .config
            .max_content_bytes
            .saturating_mul(6)
            .saturating_add(16 * 1024)
    };
    DefaultBodyLimit::max(limit).apply(&mut request);
    let uri = request.uri().clone();
    let body = Bytes::from_request(request, &state)
        .await
        .map_err(|error| {
            if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                AppError::PayloadTooLarge
            } else {
                AppError::BadRequest("Cannot read request body")
            }
        })?;
    let paste = paste::create(&state, parse_input(plain, &uri, &body)?).await?;
    let url = state.config.share_url(&paste.id);
    let result = CreatedPaste {
        raw_url: format!("{}/raw/{}", state.config.base_url, paste.id),
        id: paste.id,
        url: url.clone(),
        expires_at: paste.expires_at,
    };
    Ok((StatusCode::CREATED, [(header::LOCATION, url)], Json(result)).into_response())
}

pub async fn read(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Path(id) = path.map_err(|_| AppError::NotFound)?;
    let metadata = paste::metadata(&state, &id).await?;
    if let Some(encoded) = metadata.password_hash {
        if headers.get_all("x-paste-password").iter().count() != 1 {
            return Err(AppError::Unauthorized);
        }
        let supplied = headers
            .get("x-paste-password")
            .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
            .ok_or(AppError::Unauthorized)?;
        if !password::verify(supplied.to_owned(), encoded).await? {
            return Err(AppError::Unauthorized);
        }
    }
    let paste = db::find_live_paste(&state.pool, &id, paste::unix_now()?)
        .await?
        .ok_or(AppError::NotFound)?;
    let response = serialize_read(&paste)?;
    if paste.burn_after_read && !db::consume_api(&state.pool, &paste, paste::unix_now()?).await? {
        return Err(AppError::NotFound);
    }
    Ok(response)
}

fn serialize_read(paste: &Paste) -> Result<Response, AppError> {
    if paste.language == crate::services::pdf::PDF_LANGUAGE {
        return Err(AppError::BadRequest(
            "PDF content is available through its protected web page or download route",
        ));
    }
    let body = serde_json::to_vec(&ReadPaste {
        id: &paste.id,
        title: &paste.title,
        content: &paste.content,
        language: &paste.language,
        created_at: paste.created_at,
        expires_at: paste.expires_at,
        burn_after_read: paste.burn_after_read,
    })
    .map_err(|_| AppError::Internal("API response serialization failed"))?;
    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_modes_preserve_content_and_reject_invalid_options() {
        let uri: Uri = "/api/paste?title=A%20B&burn_after_read=true&expires_in=never"
            .parse()
            .expect("URI");
        let input = parse_input(true, &uri, b"\xef\xbb\xbfhello\r\n").expect("plain");
        assert_eq!(input.content, "\u{feff}hello\r\n");
        assert_eq!(input.title.as_deref(), Some("A B"));
        assert!(input.burn_after_read);
        for query in [
            "burn_after_read=1",
            "language=rs&language=py",
            "unknown=true",
        ] {
            let uri = format!("/api/paste?{query}").parse().expect("URI");
            assert!(parse_input(true, &uri, b"text").is_err());
        }
        let uri = "/api/paste".parse().expect("URI");
        assert!(parse_input(true, &uri, &[255]).is_err());
        for json in [
            r#"{}"#,
            r#"{"content":3}"#,
            r#"{"content":"x","burn_after_read":"true"}"#,
            r#"{"content":"x","extra":1}"#,
        ] {
            assert!(parse_input(false, &uri, json.as_bytes()).is_err());
        }
        let input = parse_input(false, &uri, br#"{"content":"hello"}"#).expect("JSON");
        assert!(!input.burn_after_read);
        assert!(input.language.is_none());
    }
}
