use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use pastelet::{
    AppState,
    config::Config,
    db, routes,
    services::{paste, pdf::MAX_PDF_BYTES},
};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

const PDF: &[u8] = include_bytes!("fixtures/two-pages.pdf");

struct Harness {
    app: Router,
    state: AppState,
}
impl Harness {
    async fn new() -> Self {
        Self::at("sqlite::memory:".into(), 1).await
    }
    async fn at(database_url: String, connections: u32) -> Self {
        let config = Config {
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            database_url,
            base_url: "https://paste.example".into(),
            allowed_origins: Vec::new(),
            max_content_bytes: 1024 * 1024,
            database_max_connections: connections,
            cleanup_interval: Duration::from_secs(60),
            api_token: None,
            rust_log: "error".into(),
        };
        let state = AppState {
            pool: db::connect(&config).await.expect("database"),
            config: Arc::new(config),
        };
        Self {
            app: routes::router(state.clone()),
            state,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        bytes: Vec<u8>,
        headers: &[(&str, &str)],
    ) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::from(bytes)).expect("request"))
            .await
            .expect("response")
    }
    async fn upload(&self, data: &[u8], fields: &[(&str, &str)]) -> Response {
        let mut body = b"--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"../../original.pdf\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
        for (name, value) in fields {
            body.extend_from_slice(format!("--boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        }
        body.extend_from_slice(b"--boundary--\r\n");
        self.request(
            "POST",
            "/paste/pdf",
            body,
            &[
                ("content-type", "multipart/form-data; boundary=boundary"),
                ("origin", "https://paste.example"),
            ],
        )
        .await
    }
    async fn create(&self, fields: &[(&str, &str)]) -> String {
        let response = self.upload(PDF, fields).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let value: serde_json::Value =
            serde_json::from_slice(&bytes(response).await).expect("JSON");
        let id = value["id"].as_str().expect("id");
        assert_eq!(id.len(), 22);
        id.into()
    }
    async fn get(&self, path: &str, cookie: &str, html: bool) -> Response {
        self.request(
            "GET",
            path,
            vec![],
            &[
                ("cookie", cookie),
                (
                    "accept",
                    if html {
                        "text/html"
                    } else {
                        "application/json"
                    },
                ),
            ],
        )
        .await
    }
    async fn form(&self, path: &str, fields: &[(&str, &str)], cookie: &str) -> Response {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields.iter().copied())
            .finish();
        self.request(
            "POST",
            path,
            body.into_bytes(),
            &[
                ("content-type", "application/x-www-form-urlencoded"),
                ("cookie", cookie),
                ("origin", "https://paste.example"),
                ("accept", "text/html"),
            ],
        )
        .await
    }
    async fn token(&self, id: &str, cookie: &str) -> String {
        let response = self.get(&format!("/{id}"), cookie, true).await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(bytes(response).await).expect("HTML");
        assert!(!html.contains("data-pdf="));
        html.split("name=\"token\" value=\"")
            .nth(1)
            .expect("token")
            .split('"')
            .next()
            .expect("value")
            .into()
    }
    async fn stored(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pdf_files")
            .fetch_one(&self.state.pool)
            .await
            .expect("count")
    }
}
async fn bytes(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 20 * 1024 * 1024)
        .await
        .expect("body")
        .to_vec()
}

#[tokio::test]
async fn pdf_round_trip_headers_title_and_no_text_operations() {
    let harness = Harness::new().await;
    let id = harness
        .create(&[("title", "报告"), ("expires_in", "never")])
        .await;
    assert!(paste::valid_id(&id));
    let storage: String = sqlx::query_scalar("SELECT storage_name FROM pdf_files")
        .fetch_one(&harness.state.pool)
        .await
        .expect("storage name");
    assert_eq!(storage, format!("{id}.pdf"));
    for path in [format!("/raw/{id}"), format!("/download/{id}")] {
        let response = harness.get(&path, "", false).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/pdf");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(
            response.headers()["content-disposition"]
                .to_str()
                .expect("header")
                .contains("filename*=UTF-8''%E6%8A%A5%E5%91%8A%2E%70%64%66")
        );
        assert_eq!(bytes(response).await, PDF);
    }
    let html = String::from_utf8(bytes(harness.get(&format!("/{id}"), "", true).await).await)
        .expect("HTML");
    assert!(html.contains("id=\"pdf-viewer\""));
    assert!(html.contains(&format!("data-share-url=\"https://paste.example/{id}\"")));
    assert!(html.contains(&STANDARD.encode(PDF)));
    for hidden in [
        "copy-paste",
        "view-toggle",
        "pastelet-source",
        "aria-label=\"Fork\"",
    ] {
        assert!(!html.contains(hidden));
    }
    assert_eq!(
        harness
            .get(&format!("/api/paste/{id}"), "", false)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        harness
            .get(&format!("/fork/{id}"), "", false)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(harness.stored().await, 1);
}

#[tokio::test]
async fn upload_checks_header_size_fields_and_origin_without_orphans() {
    let harness = Harness::new().await;
    for invalid in [b"not PDF".as_slice(), b"\x89PNG\r\n", b" %PDF-1.7"] {
        assert_eq!(
            harness.upload(invalid, &[]).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    for fields in [
        vec![("title", "one"), ("title", "two")],
        vec![("unknown", "x")],
        vec![("burn_after_read", "yes")],
        vec![("expires_in", "2d")],
    ] {
        assert_eq!(
            harness.upload(PDF, &fields).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut large = PDF.to_vec();
    large.resize(MAX_PDF_BYTES + 1, b' ');
    assert_eq!(
        harness.upload(&large, &[]).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(harness.stored().await, 0);
    let parents: i64 = sqlx::query_scalar("SELECT count(*) FROM pastes")
        .fetch_one(&harness.state.pool)
        .await
        .expect("parents");
    assert_eq!(parents, 0);
    assert_eq!(
        harness
            .request(
                "POST",
                "/paste/pdf",
                vec![],
                &[("origin", "https://evil.example")]
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    large.pop();
    assert_eq!(
        harness
            .upload(&large, &[("title", "full.pdf")])
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(harness.stored().await, 1);
}

#[tokio::test]
async fn password_guards_every_pdf_path_before_burning_and_snapshot_survives_delete() {
    let harness = Harness::new().await;
    let id = harness
        .create(&[("password", "secret"), ("burn_after_read", "true")])
        .await;
    for path in [
        format!("/{id}"),
        format!("/raw/{id}"),
        format!("/download/{id}"),
        format!("/api/paste/{id}"),
    ] {
        let response = harness.get(&path, "", false).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!String::from_utf8_lossy(&bytes(response).await).contains(&STANDARD.encode(PDF)));
    }
    assert_eq!(
        harness
            .form(&format!("/unlock/{id}"), &[("password", "wrong")], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = harness
        .form(&format!("/unlock/{id}"), &[("password", "secret")], "")
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let token = harness.token(&id, &cookie).await;
    assert_eq!(harness.stored().await, 1);
    assert_eq!(
        harness
            .form(&format!("/read/{id}"), &[("token", &token)], "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = harness
        .form(&format!("/read/{id}"), &[("token", &token)], &cookie)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(bytes(response).await).expect("HTML");
    assert!(html.contains("data-burned=\"true\""));
    assert!(!html.contains("data-share-url="));
    assert!(html.contains(&STANDARD.encode(PDF)));
    assert_eq!(harness.stored().await, 0);
    assert_eq!(
        harness
            .get(&format!("/download/{id}"), &cookie, false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn previews_head_and_text_api_do_not_burn_pdf() {
    let harness = Harness::new().await;
    let id = harness.create(&[("burn_after_read", "true")]).await;
    for path in [
        format!("/{id}"),
        format!("/raw/{id}"),
        format!("/download/{id}"),
    ] {
        let response = harness.get(&path, "", true).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!String::from_utf8_lossy(&bytes(response).await).contains("data-pdf="));
        assert_eq!(
            harness
                .request("HEAD", &path, vec![], &[("accept", "text/html")])
                .await
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        harness
            .get(&format!("/api/paste/{id}"), "", false)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(harness.stored().await, 1);
}

#[tokio::test]
async fn expiration_deletes_blob_on_access_and_background_cleanup() {
    let harness = Harness::new().await;
    let id = harness.create(&[]).await;
    let expire = |id: String| {
        let pool = harness.state.pool.clone();
        async move {
            sqlx::query("UPDATE pastes SET created_at = 1, expires_at = 2 WHERE id = ?")
                .bind(id)
                .execute(&pool)
                .await
                .expect("expire");
        }
    };
    expire(id.clone()).await;
    assert_eq!(
        harness
            .get(&format!("/download/{id}"), "", false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(harness.stored().await, 0);
    let id = harness.create(&[]).await;
    expire(id).await;
    db::cleanup_expired(&harness.state.pool, paste::unix_now().expect("clock"))
        .await
        .expect("cleanup");
    assert_eq!(harness.stored().await, 0);
    let secure: i64 = sqlx::query_scalar("PRAGMA secure_delete")
        .fetch_one(&harness.state.pool)
        .await
        .expect("secure deletion");
    assert_eq!(secure, 1);
}

#[tokio::test]
async fn concurrent_pdf_confirmation_has_one_winner_and_failed_delete_leaks_nothing() {
    let directory = tempfile::tempdir().expect("directory");
    let harness = Harness::at(
        format!("sqlite://{}", directory.path().join("pdf.db").display()),
        4,
    )
    .await;
    let id = harness.create(&[("burn_after_read", "true")]).await;
    let token = harness.token(&id, "").await;
    let path = format!("/read/{id}");
    let fields = [("token", token.as_str())];
    let (first, second) = tokio::join!(
        harness.form(&path, &fields, ""),
        harness.form(&path, &fields, "")
    );
    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::NOT_FOUND)
            .count(),
        1
    );
    assert_eq!(harness.stored().await, 0);
    let id = harness.create(&[("burn_after_read", "true")]).await;
    let token = harness.token(&id, "").await;
    sqlx::query("CREATE TRIGGER refuse_delete BEFORE DELETE ON pastes BEGIN SELECT RAISE(ABORT, 'test'); END").execute(&harness.state.pool).await.expect("trigger");
    let response = harness
        .form(&format!("/read/{id}"), &[("token", &token)], "")
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!String::from_utf8_lossy(&bytes(response).await).contains(&STANDARD.encode(PDF)));
    assert_eq!(harness.stored().await, 1);
}

#[tokio::test]
async fn attachment_failure_rolls_back_parent_and_restart_preserves_pdf() {
    let directory = tempfile::tempdir().expect("directory");
    let database = format!("sqlite://{}", directory.path().join("pdf.db").display());
    let harness = Harness::at(database.clone(), 1).await;
    sqlx::query("CREATE TRIGGER refuse_insert BEFORE INSERT ON pdf_files BEGIN SELECT RAISE(ABORT, 'test'); END").execute(&harness.state.pool).await.expect("trigger");
    assert_eq!(
        harness.upload(PDF, &[]).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let parents: i64 = sqlx::query_scalar("SELECT count(*) FROM pastes")
        .fetch_one(&harness.state.pool)
        .await
        .expect("parents");
    assert_eq!(parents, 0);
    sqlx::query("DROP TRIGGER refuse_insert")
        .execute(&harness.state.pool)
        .await
        .expect("remove trigger");
    let id = harness.create(&[]).await;
    harness.state.pool.close().await;
    let reopened = Harness::at(database, 1).await;
    assert_eq!(
        bytes(reopened.get(&format!("/download/{id}"), "", false).await).await,
        PDF
    );
}

#[tokio::test]
async fn pdf_runtime_assets_are_embedded_and_paths_are_allowlisted() {
    let harness = Harness::new().await;
    for path in [
        "pdf.min.mjs",
        "pdf.worker.min.mjs",
        "cmaps/Adobe-GB1-UCS2.bcmap",
    ] {
        let response = harness
            .get(&format!("/static/pdfjs/{path}"), "", false)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!bytes(response).await.is_empty());
    }
    assert_eq!(
        harness
            .get("/static/pdfjs/../../Cargo.toml", "", false)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}
