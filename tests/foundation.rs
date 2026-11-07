use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::IntoResponse,
};
use rpaste::{AppState, config::Config, db, error::AppError, models::Paste, routes};
use tower::ServiceExt;

fn config(database_url: String) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().expect("socket address"),
        database_url,
        base_url: "http://localhost:3000".into(),
        max_content_bytes: 1048576,
        database_max_connections: 2,
        cleanup_interval: Duration::from_secs(60),
        api_token: None,
        rust_log: "rpaste=info".into(),
    }
}

#[tokio::test]
async fn migration_is_repeatable_and_persistent() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("nested/pastes.db");
    let config = config(format!("sqlite://{}", path.display()));
    let pool = db::connect(&config).await.expect("connect and migrate");
    db::MIGRATOR.run(&pool).await.expect("repeat migration");
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .expect("journal mode");
    assert_eq!(journal, "wal");
    sqlx::query("INSERT INTO pastes (id, content, language, created_at) VALUES ('Ab12Cd34', 'hello', 'text', 100)")
        .execute(&pool).await.expect("insert paste");
    pool.close().await;
    let reopened = db::connect(&config).await.expect("reopen database");
    let paste: Paste = sqlx::query_as("SELECT * FROM pastes WHERE id = ?")
        .bind("Ab12Cd34")
        .fetch_one(&reopened)
        .await
        .expect("stored paste");
    assert_eq!(paste.content, "hello");
    assert!(!paste.burn_after_read);
    assert!(paste.password_hash.is_none());
    assert!(!paste.is_expired_at(i64::MAX));
    reopened.close().await;
}

#[tokio::test]
async fn schema_rejects_invalid_records() {
    let pool = db::connect(&config("sqlite::memory:".into()))
        .await
        .expect("database");
    for (id, content, burn, expires) in [
        ("short", "hello", 0, Some(200)),
        ("Ab12Cd!4", "hello", 0, Some(200)),
        ("Ab12Cd34", "", 0, Some(200)),
        ("Ab12Cd34", "hello", 2, Some(200)),
        ("Ab12Cd34", "hello", 0, Some(100)),
        ("Ab12Cd34", "hello", 0, Some(99)),
    ] {
        let result = sqlx::query("INSERT INTO pastes (id, content, language, burn_after_read, created_at, expires_at) VALUES (?, ?, 'text', ?, 100, ?)")
            .bind(id).bind(content).bind(burn).bind(expires).execute(&pool).await;
        assert!(result.is_err(), "invalid record must be rejected");
    }
    pool.close().await;
}

#[tokio::test]
async fn router_renders_template_and_returns_structured_404() {
    let config = config("sqlite::memory:".into());
    let pool = db::connect(&config).await.expect("database");
    let app = routes::router(AppState {
        config: Arc::new(config),
        pool: pool.clone(),
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri("/missing")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 4096).await.expect("body");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).expect("JSON"),
        serde_json::json!({"error":{"code":"not_found","message":"Not found"}})
    );
    pool.close().await;
}

#[tokio::test]
async fn errors_preserve_status_and_hide_internal_details() {
    for (error, status, code) in [
        (
            AppError::BadRequest("Invalid language"),
            StatusCode::BAD_REQUEST,
            "bad_request",
        ),
        (
            AppError::Unauthorized,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (
            AppError::PayloadTooLarge,
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
        ),
        (
            AppError::Database(sqlx::Error::Protocol("private database detail".into())),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
        ),
    ] {
        let response = error.into_response();
        assert_eq!(response.status(), status);
        let body = to_bytes(response.into_body(), 4096).await.expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(body["error"]["code"], code);
        assert!(!body.to_string().contains("private database detail"));
    }
}
