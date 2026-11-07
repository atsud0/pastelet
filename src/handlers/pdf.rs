use crate::{
    AppState,
    error::AppError,
    models::Paste,
    services::{access, paste::CreatePaste, pdf},
};
use axum::{
    Json,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::collections::HashMap;

fn multipart_error(error: axum::extract::multipart::MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge
    } else {
        AppError::BadRequest("Invalid PDF upload")
    }
}

pub async fn create(
    State(state): State<AppState>,
    mut request: Request,
) -> Result<Response, AppError> {
    access::same_origin(&state, request.headers())?;
    DefaultBodyLimit::max(pdf::MAX_PDF_BYTES + 16 * 1024).apply(&mut request);
    let mut multipart = Multipart::from_request(request, &state)
        .await
        .map_err(|_| AppError::BadRequest("Expected multipart upload"))?;
    let mut fields = HashMap::new();
    let mut file = None;
    let mut filename = None;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        let name = field
            .name()
            .ok_or(AppError::BadRequest("Missing field name"))?
            .to_owned();
        if !["file", "title", "expires_in", "password", "burn_after_read"].contains(&name.as_str())
        {
            return Err(AppError::BadRequest("Unknown upload field"));
        }
        if fields.contains_key(&name) || name == "file" && file.is_some() {
            return Err(AppError::BadRequest("Duplicate upload field"));
        }
        let is_file = name == "file";
        if is_file {
            filename = field.file_name().map(str::to_owned);
        }
        let limit = if is_file { pdf::MAX_PDF_BYTES } else { 4096 };
        let mut data = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
            if data.len().saturating_add(chunk.len()) > limit {
                return Err(AppError::PayloadTooLarge);
            }
            data.extend_from_slice(&chunk);
        }
        if is_file {
            pdf::validate(&data)?;
            file = Some(data);
        } else {
            fields.insert(
                name,
                String::from_utf8(data)
                    .map_err(|_| AppError::BadRequest("Invalid UTF-8 metadata"))?,
            );
        }
    }
    let data = file.ok_or(AppError::BadRequest("PDF file is required"))?;
    let burn_after_read = match fields.remove("burn_after_read").as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        _ => return Err(AppError::BadRequest("Invalid burn setting")),
    };
    let title = fields
        .remove("title")
        .filter(|value| !value.trim().is_empty())
        .or_else(|| filename.map(|name| name.chars().take(200).collect()));
    let record = pdf::create(
        &state,
        CreatePaste {
            content: String::new(),
            title,
            language: None,
            expires_in: fields.remove("expires_in"),
            password: fields.remove("password"),
            burn_after_read,
        },
        data,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"id": record.id, "url": format!("/{}", record.id)})),
    )
        .into_response())
}

pub async fn file_response(
    state: &AppState,
    record: &Paste,
    attachment: bool,
) -> Result<Response, AppError> {
    let data = pdf::data(state, &record.id).await?;
    let name = pdf::download_name(record.title.as_deref(), &record.id);
    let encoded: String = name.bytes().map(|byte| format!("%{byte:02X}")).collect();
    let mode = if attachment { "attachment" } else { "inline" };
    let disposition = format!(
        "{mode}; filename=\"{}.pdf\"; filename*=UTF-8''{encoded}",
        record.id
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox; frame-ancestors 'none'",
            ),
        ],
        data,
    )
        .into_response())
}
