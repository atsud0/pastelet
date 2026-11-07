use askama::Template;
use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};

use crate::{
    AppState, db,
    error::AppError,
    models::Paste,
    services::{access, paste},
};
use serde::Deserialize;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadTarget {
    #[default]
    View,
    Raw,
    Download,
    Fork,
}

impl ReadTarget {
    pub fn key(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Raw => "raw",
            Self::Download => "download",
            Self::Fork => "fork",
        }
    }
    pub fn path(self, id: &str) -> String {
        match self {
            Self::View => format!("/{id}"),
            _ => format!("/{}/{id}", self.key()),
        }
    }
}

pub enum PageAccess {
    Ready(Paste),
    Prompt(Response),
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
    label: &'static str,
}

#[derive(Template)]
#[template(path = "unlock.html")]
struct UnlockTemplate<'a> {
    id: &'a str,
    target: &'a str,
    error: &'a str,
}

#[derive(Template)]
#[template(path = "confirm.html")]
struct ConfirmTemplate<'a> {
    id: &'a str,
    target: &'a str,
    token: &'a str,
}

#[derive(Template)]
#[template(source = "<p class=\"form-error\">{{ message }}</p>", ext = "html")]
pub struct FeedbackTemplate<'a> {
    pub message: &'a str,
}

pub fn is_htmx(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        .is_some_and(|value| value == "true")
}

pub fn wants_html(headers: &HeaderMap) -> bool {
    is_htmx(headers)
        || headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|part| part.trim().starts_with("text/html"))
            })
}

pub fn error_response(status: StatusCode) -> Response {
    let label = match status {
        StatusCode::NOT_FOUND => "Not found",
        StatusCode::UNAUTHORIZED => "Locked",
        StatusCode::PAYLOAD_TOO_LARGE => "Too large",
        StatusCode::BAD_REQUEST => "Invalid",
        StatusCode::METHOD_NOT_ALLOWED => "Not allowed",
        _ => "Unavailable",
    };
    match (ErrorTemplate { label }).render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => AppError::Template(error).into_response(),
    }
}

pub fn unlock_page(id: &str, target: ReadTarget, error: &str) -> Result<Response, AppError> {
    Ok((
        StatusCode::UNAUTHORIZED,
        Html(
            UnlockTemplate {
                id,
                target: target.key(),
                error,
            }
            .render()?,
        ),
    )
        .into_response())
}

pub async fn gate(
    state: &AppState,
    id: &str,
    headers: &HeaderMap,
    target: ReadTarget,
) -> Result<PageAccess, AppError> {
    let metadata = paste::metadata(state, id).await?;
    if !access::authorized(state, &metadata, headers).await? {
        if wants_html(headers) {
            return Ok(PageAccess::Prompt(unlock_page(id, target, "")?));
        }
        return Err(AppError::Unauthorized);
    }
    if metadata.burn_after_read {
        if !wants_html(headers) {
            return Err(AppError::BadRequest("Confirmation required"));
        }
        let token = access::issue(state, &metadata, "confirm").await?;
        return Ok(PageAccess::Prompt(
            Html(
                ConfirmTemplate {
                    id,
                    target: target.key(),
                    token: &token,
                }
                .render()?,
            )
            .into_response(),
        ));
    }
    let paste = db::find_live_paste(&state.pool, id, paste::unix_now()?)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(PageAccess::Ready(paste))
}
