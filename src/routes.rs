use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

use crate::{AppState, api, error::AppError, handlers};

pub fn router(state: AppState) -> Router {
    let form_limit = state
        .config
        .max_content_bytes
        .saturating_mul(3)
        .saturating_add(16 * 1024);
    Router::new()
        .route("/", get(handlers::index))
        .route("/paste", post(handlers::create))
        .route("/paste/pdf", post(handlers::pdf::create))
        .route("/api/paste", post(api::create))
        .route(
            "/api/paste/{id}",
            get(api::read).head(|| async { AppError::MethodNotAllowed }),
        )
        .route("/{id}", get(handlers::view))
        .route("/raw/{id}", get(handlers::raw))
        .route("/download/{id}", get(handlers::download))
        .route("/fork/{id}", get(handlers::fork))
        .route("/fork", post(handlers::paste::fork_snapshot))
        .route("/unlock/{id}", post(handlers::paste::unlock))
        .route("/read/{id}", post(handlers::paste::consume))
        .route("/static/app.css", get(handlers::assets::stylesheet))
        .route(
            "/static/syntax.css",
            get(handlers::assets::syntax_stylesheet),
        )
        .route("/static/app.js", get(handlers::assets::script))
        .route("/static/editor.js", get(handlers::assets::editor_script))
        .route(
            "/static/pdf-editor.js",
            get(handlers::assets::pdf_editor_script),
        )
        .route(
            "/static/pdf-viewer.mjs",
            get(handlers::assets::pdf_viewer_script),
        )
        .route("/static/pdfjs/{*path}", get(handlers::assets::pdf_asset))
        .route("/static/htmx.min.js", get(handlers::assets::htmx))
        .fallback(|| async { AppError::NotFound })
        .method_not_allowed_fallback(|| async { AppError::MethodNotAllowed })
        .layer(DefaultBodyLimit::max(form_limit))
        .layer(middleware::from_fn(response_headers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn response_headers(request: Request, next: Next) -> Response {
    let html = handlers::pages::wants_html(request.headers())
        && !["/api/", "/raw/", "/download/", "/static/"]
            .iter()
            .any(|prefix| request.uri().path().starts_with(prefix));
    let mut response = next.run(request).await;
    if html
        && (response.status().is_client_error() || response.status().is_server_error())
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value == "application/json")
    {
        let allow = response.headers().get(header::ALLOW).cloned();
        response = handlers::pages::error_response(response.status());
        if let Some(allow) = allow {
            response.headers_mut().insert(header::ALLOW, allow);
        }
    }
    let headers = response.headers_mut();
    headers.append(header::VARY, HeaderValue::from_static("Accept, HX-Request"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow, noarchive"),
    );
    headers.entry(header::CONTENT_SECURITY_POLICY).or_insert(HeaderValue::from_static(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; font-src 'self' blob:; style-src 'self'; img-src https: http: blob: data:; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    ));
    response
}
