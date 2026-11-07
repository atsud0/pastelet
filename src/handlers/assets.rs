use axum::{http::header, response::IntoResponse};

use crate::{
    error::AppError,
    services::{compute, render},
};

include!(concat!(env!("OUT_DIR"), "/pdf_assets.rs"));

pub async fn pdf_asset(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    let (_, bytes) = PDF_ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .ok_or(AppError::NotFound)?;
    let mime = if path.ends_with(".mjs") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".ttf") {
        "font/ttf"
    } else {
        "application/octet-stream"
    };
    Ok(([(header::CONTENT_TYPE, mime)], *bytes).into_response())
}

pub async fn pdf_editor_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/pdf-editor.js"),
    )
}

pub async fn pdf_viewer_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/pdf-viewer.mjs"),
    )
}

pub async fn stylesheet() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/app.css"),
    )
}

pub async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/app.js"),
    )
}

pub async fn htmx() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/vendor/htmx-2.0.8.min.js"),
    )
}

pub async fn editor_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/editor.js"),
    )
}

pub async fn syntax_stylesheet() -> Result<impl IntoResponse, AppError> {
    let css = compute(render::syntax_css).await?;
    Ok(([(header::CONTENT_TYPE, "text/css; charset=utf-8")], css))
}
