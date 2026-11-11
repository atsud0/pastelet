use super::pages::{self, PageAccess, ReadTarget};
use askama::Template;
use axum::{
    Form,
    extract::{FromRequestParts, Path, State, rejection::FormRejection},
    http::{HeaderMap, StatusCode, header, request::Parts},
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::{
    AppState, db,
    error::AppError,
    models::Paste,
    services::{
        access, compute,
        language::{self, LanguageOption},
        password,
        paste::{self, CreatePaste},
        render,
    },
};

#[derive(Template)]
#[template(path = "create.html")]
struct CreateTemplate {
    max_pdf_bytes: usize,
    title: String,
    content: String,
    language: String,
    languages: Vec<LanguageOption>,
    expires_in: String,
    error: String,
    burn_after_read: bool,
}

#[derive(Template)]
#[template(path = "view.html")]
struct ViewTemplate {
    share_url: String,
    is_pdf: bool,
    pdf_base64: String,
    download_name: String,
    id: String,
    title: String,
    language: String,
    source_html: String,
    markdown_html: String,
    is_markdown: bool,
    raw_base64: String,
    content: String,
    burned: bool,
}

pub struct PasteId(String);

impl<S: Send + Sync> FromRequestParts<S> for PasteId {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::NotFound)?;
        if !paste::valid_id(&id) {
            return Err(AppError::NotFound);
        }
        Ok(Self(id))
    }
}

pub async fn index() -> Result<Html<String>, AppError> {
    create_form(
        String::new(),
        String::new(),
        "auto".into(),
        "1d".into(),
        String::new(),
        false,
    )
    .await
}

async fn create_form(
    title: String,
    content: String,
    language: String,
    expires_in: String,
    error: String,
    burn_after_read: bool,
) -> Result<Html<String>, AppError> {
    compute(move || {
        Ok(Html(
            CreateTemplate {
                max_pdf_bytes: crate::services::pdf::MAX_PDF_BYTES,
                title,
                content,
                language,
                languages: language::options(),
                expires_in,
                error,
                burn_after_read,
            }
            .render()?,
        ))
    })
    .await
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<CreatePaste>, FormRejection>,
) -> Result<Response, AppError> {
    let input = form.map(|Form(input)| input).map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PayloadTooLarge
        } else {
            AppError::BadRequest("Invalid form")
        }
    });
    let retained = input.as_ref().ok().cloned();
    let result = match input {
        Ok(input) => paste::create(&state, input).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(paste) if pages::is_htmx(&headers) => {
            Ok(([("hx-redirect", format!("/{}", paste.id))], "").into_response())
        }
        Ok(paste) => Ok(Redirect::to(&format!("/{}", paste.id)).into_response()),
        Err(error) if pages::wants_html(&headers) => {
            let message = match &error {
                AppError::BadRequest(message) => *message,
                AppError::PayloadTooLarge => "Content too large",
                _ => "Unavailable",
            };
            let status = error.into_response().status();
            if pages::is_htmx(&headers) {
                return Ok(
                    (status, Html(pages::FeedbackTemplate { message }.render()?)).into_response(),
                );
            }
            if let Some(input) = retained {
                return Ok((
                    status,
                    create_form(
                        input.title.unwrap_or_default(),
                        input.content,
                        input.language.unwrap_or_else(|| "auto".into()),
                        input.expires_in.unwrap_or_else(|| "1d".into()),
                        message.into(),
                        input.burn_after_read,
                    )
                    .await?,
                )
                    .into_response());
            }
            Ok(pages::error_response(status))
        }
        Err(error) => Err(error),
    }
}

pub async fn view(
    State(state): State<AppState>,
    PasteId(id): PasteId,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    show(&state, &id, &headers, ReadTarget::View).await
}

async fn view_response(state: &AppState, paste: Paste) -> Result<Response, AppError> {
    let share_url = state.config.share_url(&paste.id);
    let is_pdf = paste.language == crate::services::pdf::PDF_LANGUAGE;
    let pdf_base64 = if is_pdf {
        STANDARD.encode(crate::services::pdf::data(state, &paste.id).await?)
    } else {
        String::new()
    };
    let download_name = if is_pdf {
        crate::services::pdf::download_name(paste.title.as_deref(), &paste.id)
    } else {
        language::download_filename(&paste.id, &paste.language)
    };
    compute(move || {
        let source_html = if is_pdf {
            String::new()
        } else {
            render::source(&paste.content, &paste.language)?
        };
        let is_markdown = paste.language == "markdown";
        let markdown_html = if is_markdown {
            render::markdown(&paste.content)?
        } else {
            String::new()
        };
        Ok(Html(
            ViewTemplate {
                share_url,
                is_pdf,
                pdf_base64,
                download_name,
                title: paste.title.unwrap_or_else(|| "Untitled".into()),
                id: paste.id,
                language: paste.language,
                source_html,
                markdown_html,
                is_markdown,
                raw_base64: STANDARD.encode(paste.content.as_bytes()),
                content: paste.content,
                burned: paste.burn_after_read,
            }
            .render()?,
        ))
    })
    .await
    .map(IntoResponse::into_response)
}

pub async fn raw(
    State(state): State<AppState>,
    PasteId(id): PasteId,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    show(&state, &id, &headers, ReadTarget::Raw).await
}

pub async fn download(
    State(state): State<AppState>,
    PasteId(id): PasteId,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    show(&state, &id, &headers, ReadTarget::Download).await
}

fn text_response(paste: Paste, attachment: bool) -> Response {
    let disposition = if attachment { "attachment" } else { "inline" };
    let filename = language::download_filename(&paste.id, &paste.language);
    let disposition = format!("{disposition}; filename=\"{filename}\"");
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox; frame-ancestors 'none'",
            ),
        ],
        paste.content,
    )
        .into_response()
}

pub async fn fork(
    State(state): State<AppState>,
    PasteId(id): PasteId,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    show(&state, &id, &headers, ReadTarget::Fork).await
}

async fn show(
    state: &AppState,
    id: &str,
    headers: &HeaderMap,
    target: ReadTarget,
) -> Result<Response, AppError> {
    match pages::gate(state, id, headers, target).await? {
        PageAccess::Ready(paste) => render_response(state, paste, target).await,
        PageAccess::Prompt(response) => Ok(response),
    }
}

async fn render_response(
    state: &AppState,
    paste: Paste,
    target: ReadTarget,
) -> Result<Response, AppError> {
    if paste.language == crate::services::pdf::PDF_LANGUAGE {
        return match target {
            ReadTarget::View => view_response(state, paste).await,
            ReadTarget::Raw => super::pdf::file_response(state, &paste, false).await,
            ReadTarget::Download => super::pdf::file_response(state, &paste, true).await,
            ReadTarget::Fork => Err(AppError::BadRequest("PDF fork is not supported")),
        };
    }
    match target {
        ReadTarget::View => view_response(state, paste).await,
        ReadTarget::Raw => Ok(text_response(paste, false)),
        ReadTarget::Download => Ok(text_response(paste, true)),
        ReadTarget::Fork => create_form(
            paste.title.unwrap_or_default(),
            paste.content,
            paste.language,
            "1d".into(),
            String::new(),
            false,
        )
        .await
        .map(IntoResponse::into_response),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlockForm {
    password: String,
    #[serde(default)]
    target: ReadTarget,
}

fn form_error(error: FormRejection) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge
    } else {
        AppError::BadRequest("Invalid form")
    }
}

pub async fn unlock(
    State(state): State<AppState>,
    PasteId(id): PasteId,
    headers: HeaderMap,
    form: Result<Form<UnlockForm>, FormRejection>,
) -> Result<Response, AppError> {
    access::same_origin(&state, &headers)?;
    let Form(input) = form.map_err(form_error)?;
    let metadata = paste::metadata(&state, &id).await?;
    if let Some(hash) = metadata.password_hash.as_ref()
        && !password::verify(input.password, hash.clone()).await?
    {
        if pages::wants_html(&headers) {
            return pages::unlock_page(&state, &id, input.target, "Incorrect password");
        }
        return Err(AppError::Unauthorized);
    }
    let token = access::issue(&state, &metadata, "access").await?;
    Ok((
        [(
            header::SET_COOKIE,
            access::cookie(&state, &id, &token, &headers)?,
        )],
        Redirect::to(&input.target.path(&id)),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmForm {
    token: String,
    #[serde(default)]
    target: ReadTarget,
}

pub async fn consume(
    State(state): State<AppState>,
    PasteId(id): PasteId,
    headers: HeaderMap,
    form: Result<Form<ConfirmForm>, FormRejection>,
) -> Result<Response, AppError> {
    access::same_origin(&state, &headers)?;
    let Form(input) = form.map_err(form_error)?;
    let paste = paste::authorized_read(&state, &id, &headers).await?;
    if !paste.burn_after_read {
        return Err(AppError::BadRequest("Confirmation not required"));
    }
    let hash =
        access::token_hash(&input.token).ok_or(AppError::BadRequest("Invalid confirmation"))?;
    if !db::valid_token(&state.pool, &id, &hash, "confirm", paste::unix_now()?).await? {
        paste::metadata(&state, &id).await?;
        return Err(AppError::BadRequest("Invalid confirmation"));
    }
    let response = render_response(&state, paste, input.target).await?;
    paste::consume(&state, &id, &input.token).await?;
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkForm {
    content: String,
    title: Option<String>,
    language: Option<String>,
}

pub async fn fork_snapshot(
    State(state): State<AppState>,
    form: Result<Form<ForkForm>, FormRejection>,
) -> Result<Html<String>, AppError> {
    let Form(input) = form.map_err(form_error)?;
    let input = CreatePaste {
        content: input.content,
        title: input.title,
        language: input.language,
        expires_in: None,
        password: None,
        burn_after_read: false,
    };
    paste::validate(&input, state.config.max_content_bytes)?;
    let (input, language) = compute(move || {
        let language = language::resolve(input.language.as_deref(), &input.content)?;
        Ok((input, language))
    })
    .await?;
    create_form(
        input.title.unwrap_or_default(),
        input.content,
        language,
        "1d".into(),
        String::new(),
        false,
    )
    .await
}
