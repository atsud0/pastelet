use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::IntoResponse,
};
use pastelet::{AppState, config::Config, db, error::AppError, models::Paste, routes};
use tower::ServiceExt;

fn config(database_url: String) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().expect("socket address"),
        database_url,
        base_url: "http://localhost:3000".into(),
        allowed_origins: Vec::new(),
        max_content_bytes: 1048576,
        database_max_connections: 2,
        cleanup_interval: Duration::from_secs(60),
        api_token: None,
        rust_log: "pastelet=info".into(),
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
        ("Ab12Cd34Ef56Gh78Ij90K", "hello", 0, Some(200)),
        ("Ab12Cd34Ef56Gh78Ij90Klm", "hello", 0, Some(200)),
        ("Ab12Cd34Ef56Gh78Ij90K-", "hello", 0, Some(200)),
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
async fn upgrade_preserves_legacy_content_tokens_and_pdf_with_foreign_keys_enabled() {
    use pastelet::services::{access, password};
    let directory = tempfile::tempdir().expect("directory");
    let migrations = directory.path().join("migrations");
    std::fs::create_dir(&migrations).expect("migrations directory");
    for name in [
        "0001_create_pastes.sql",
        "0002_paste_tokens.sql",
        "0003_pdf_files.sql",
    ] {
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .join(name),
            migrations.join(name),
        )
        .expect("copy old migration");
    }
    let old = sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .expect("old migrator");
    let database = directory.path().join("upgrade.db");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&database)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = sqlx::SqlitePool::connect_with(options)
        .await
        .expect("old database");
    old.run(&pool).await.expect("old schema");
    let hash = password::hash("secret".into()).await.expect("hash");
    sqlx::query("INSERT INTO pastes (id,title,content,language,password_hash,created_at,expires_at) VALUES ('OldText1','Legacy','old content','text',?,100,9223372036854775807)")
        .bind(&hash).execute(&pool).await.expect("old text");
    sqlx::query("INSERT INTO pastes (id,title,content,language,burn_after_read,created_at) VALUES ('OldPdf01','Legacy PDF','PDF','pdf',1,100)")
        .execute(&pool).await.expect("old PDF");
    let pdf = include_bytes!("fixtures/two-pages.pdf");
    sqlx::query("INSERT INTO pdf_files VALUES ('OldPdf01','OldPdf01.pdf',?)")
        .bind(pdf.as_slice())
        .execute(&pool)
        .await
        .expect("attachment");
    let access_token = "A".repeat(43);
    let confirm_token = "B".repeat(43);
    for (id, token, kind) in [
        ("OldText1", &access_token, "access"),
        ("OldPdf01", &confirm_token, "confirm"),
    ] {
        sqlx::query("INSERT INTO paste_tokens VALUES (?,?,?,?)")
            .bind(access::token_hash(token).expect("hash"))
            .bind(id)
            .bind(kind)
            .bind(i64::MAX)
            .execute(&pool)
            .await
            .expect("old token");
    }
    std::fs::write(
        migrations.join("0004_expand_paste_ids.sql"),
        format!(
            "{}\nINSERT INTO missing_migration_test_table VALUES (1);",
            include_str!("../migrations/0004_expand_paste_ids.sql")
        ),
    )
    .expect("failure fixture");
    let failing = sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .expect("failing migrator");
    assert!(failing.run(&pool).await.is_err());
    pool.close().await;
    let config = config(format!("sqlite://{}", database.display()));
    let pool = db::connect(&config).await.expect("upgrade after rollback");
    db::MIGRATOR.run(&pool).await.expect("repeat migration");
    let stored: Paste = sqlx::query_as("SELECT * FROM pastes WHERE id='OldText1'")
        .fetch_one(&pool)
        .await
        .expect("legacy text");
    assert_eq!(stored.password_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(stored.title.as_deref(), Some("Legacy"));
    assert_eq!(stored.created_at, 100);
    assert_eq!(stored.expires_at, Some(i64::MAX));
    let attachment: (String, Vec<u8>) =
        sqlx::query_as("SELECT storage_name,data FROM pdf_files WHERE paste_id='OldPdf01'")
            .fetch_one(&pool)
            .await
            .expect("legacy PDF");
    assert_eq!(attachment, ("OldPdf01.pdf".into(), pdf.to_vec()));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM paste_tokens")
            .fetch_one(&pool)
            .await
            .expect("tokens"),
        2
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("FK check")
            .is_empty()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("FK setting"),
        1
    );
    let app = routes::router(AppState {
        config: Arc::new(config),
        pool: pool.clone(),
    });
    for path in [
        "/OldText1",
        "/raw/OldText1",
        "/download/OldText1",
        "/fork/OldText1",
        "/api/paste/OldText1",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("x-paste-password", "secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/raw/OldText1")
                .header("cookie", format!("pastelet_access_OldText1={access_token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("cookie response");
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/read/OldPdf01")
                .header("origin", "http://localhost:3000")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("token={confirm_token}&target=download")))
                .expect("burn request"),
        )
        .await
        .expect("burn response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 4096)
            .await
            .expect("PDF bytes")
            .as_ref(),
        pdf
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pdf_files")
            .fetch_one(&pool)
            .await
            .expect("PDF deleted"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM paste_tokens WHERE paste_id='OldPdf01'")
            .fetch_one(&pool)
            .await
            .expect("token deleted"),
        0
    );
    sqlx::query("INSERT INTO pastes (id,content,language,created_at) VALUES ('Ab12Cd34Ef56Gh78Ij90Kl','new content','text',100)").execute(&pool).await.expect("new ID accepted");
    sqlx::query("DELETE FROM pastes WHERE id='OldText1'")
        .execute(&pool)
        .await
        .expect("delete old text");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM paste_tokens")
            .fetch_one(&pool)
            .await
            .expect("tokens deleted"),
        0
    );
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
