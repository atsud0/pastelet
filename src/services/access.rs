use axum::http::{HeaderMap, header};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use super::{password, paste};
use crate::{AppState, db, error::AppError, models::PasteMetadata};

pub const ACCESS_SECONDS: i64 = 3600;
const CONFIRM_SECONDS: i64 = 600;

pub fn token_hash(token: &str) -> Option<Vec<u8>> {
    if token.len() != 43
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    Some(Sha256::digest(token.as_bytes()).to_vec())
}

pub async fn issue(
    state: &AppState,
    metadata: &PasteMetadata,
    kind: &str,
) -> Result<String, AppError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AppError::Internal("OS randomness unavailable"))?;
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let now = paste::unix_now()?;
    let lifetime = if kind == "access" {
        ACCESS_SECONDS
    } else {
        CONFIRM_SECONDS
    };
    let expires_at = metadata.expires_at.unwrap_or(i64::MAX).min(now + lifetime);
    let hash = Sha256::digest(token.as_bytes());
    if !db::insert_token(&state.pool, &metadata.id, &hash, kind, expires_at, now).await? {
        return Err(AppError::NotFound);
    }
    Ok(token)
}

pub fn cookie_name(id: &str) -> String {
    format!("rpaste_access_{id}")
}

pub fn cookie(state: &AppState, id: &str, token: &str) -> String {
    let secure = if state.config.base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ACCESS_SECONDS}{secure}",
        cookie_name(id)
    )
}

pub async fn authorized(
    state: &AppState,
    metadata: &PasteMetadata,
    headers: &HeaderMap,
) -> Result<bool, AppError> {
    let Some(encoded) = &metadata.password_hash else {
        return Ok(true);
    };
    if let Some(value) = headers.get("x-paste-password") {
        let password = std::str::from_utf8(value.as_bytes()).map_err(|_| AppError::Unauthorized)?;
        return password::verify(password.to_owned(), encoded.clone()).await;
    }
    let name = cookie_name(&metadata.id);
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for part in value.split(';') {
            let Some((key, token)) = part.trim().split_once('=') else {
                continue;
            };
            if key == name
                && let Some(hash) = token_hash(token)
                && db::valid_token(
                    &state.pool,
                    &metadata.id,
                    &hash,
                    "access",
                    paste::unix_now()?,
                )
                .await?
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn same_origin(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|site| site == "cross-site")
    {
        return Err(AppError::BadRequest("Cross-site request rejected"));
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin
            .to_str()
            .ok()
            .and_then(|value| url::Url::parse(value).ok());
        let expected = url::Url::parse(&state.config.base_url)
            .map_err(|_| AppError::Internal("invalid base URL"))?;
        if origin.is_none_or(|origin| origin.origin() != expected.origin()) {
            return Err(AppError::BadRequest("Cross-site request rejected"));
        }
    }
    Ok(())
}
