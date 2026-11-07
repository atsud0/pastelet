use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
    response::Response,
};
use rpaste::{
    AppState,
    config::Config,
    db, routes,
    services::{access, paste},
};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: AppState,
}

impl Harness {
    async fn new(limit: usize, token: Option<&str>) -> Self {
        Self::at(limit, token, "sqlite::memory:".into(), 1).await
    }

    async fn at(limit: usize, token: Option<&str>, database_url: String, connections: u32) -> Self {
        let config = Config {
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            database_url,
            base_url: "https://paste.example".into(),
            max_content_bytes: limit,
            database_max_connections: connections,
            cleanup_interval: Duration::from_secs(60),
            api_token: token.map(str::to_owned),
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
        method: Method,
        path: &str,
        body: impl Into<Body>,
        headers: &[(&str, &str)],
    ) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.app
            .clone()
            .oneshot(request.body(body.into()).expect("request"))
            .await
            .expect("response")
    }

    async fn create(&self, value: Value) -> String {
        let response = self
            .request(
                Method::POST,
                "/api/paste",
                value.to_string(),
                &[("content-type", "application/json")],
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body(response).await["id"].as_str().expect("id").to_owned()
    }

    async fn read(&self, id: &str, headers: &[(&str, &str)]) -> Response {
        self.request(
            Method::GET,
            &format!("/api/paste/{id}"),
            Body::empty(),
            headers,
        )
        .await
    }
}

async fn body(response: Response) -> Value {
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["cache-control"], "no-store");
    serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("JSON")
}

async fn error(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let value = body(response).await;
    assert_eq!(value["error"]["code"], code);
    assert!(value["error"]["message"].is_string());
    assert_eq!(value.as_object().expect("object").len(), 1);
}

#[tokio::test]
async fn json_round_trip_defaults_links_and_exact_public_fields() {
    let harness = Harness::new(1048576, None).await;
    let content = "fn main() {\r\n    println!(\"中文 <script>\");\r\n}\r\n";
    let response = harness
        .request(
            Method::POST,
            "/api/paste",
            json!({"content":content}).to_string(),
            &[
                ("content-type", "application/json; charset=utf-8"),
                ("host", "attacker.example"),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let location = response.headers()["location"]
        .to_str()
        .expect("location")
        .to_owned();
    let created = body(response).await;
    let id = created["id"].as_str().expect("id");
    assert!(paste::valid_id(id));
    assert_eq!(created.as_object().expect("object").len(), 4);
    assert_eq!(created["url"], format!("https://paste.example/{id}"));
    assert_eq!(created["url"], location);
    assert_eq!(
        created["raw_url"],
        format!("https://paste.example/raw/{id}")
    );
    let response = harness.read(id, &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let read = body(response).await;
    assert_eq!(read.as_object().expect("object").len(), 7);
    assert_eq!(read["content"], content);
    assert_eq!(read["language"], "Rust");
    assert_eq!(read["title"], Value::Null);
    assert_eq!(read["burn_after_read"], false);
    assert_eq!(
        read["expires_at"].as_i64(),
        Some(read["created_at"].as_i64().expect("time") + 86400)
    );
    assert_eq!(read["expires_at"], created["expires_at"]);
    assert_eq!(harness.read(id, &[]).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn plain_text_preserves_bytes_and_parses_all_query_options() {
    let harness = Harness::new(1048576, None).await;
    let content = "\u{feff}\r\n# 标题\r\n\r\n";
    let response = harness.request(Method::POST, "/api/paste?title=Hello%20world&language=md&expires_in=never&password=demo%2Bpass&burn_after_read=true", content, &[("content-type", "text/plain; charset=UTF-8")]).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body(response).await;
    assert_eq!(created["expires_at"], Value::Null);
    let id = created["id"].as_str().expect("id");
    let read = body(harness.read(id, &[("x-paste-password", "demo+pass")]).await).await;
    assert_eq!(read["content"], content);
    assert_eq!(read["title"], "Hello world");
    assert_eq!(read["language"], "markdown");
    assert_eq!(read["burn_after_read"], true);
    error(
        harness.read(id, &[]).await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
}

#[tokio::test]
async fn every_expiration_is_supported_in_both_modes() {
    let harness = Harness::new(1024, None).await;
    for (expiration, seconds) in [
        ("10m", Some(600)),
        ("1h", Some(3600)),
        ("1d", Some(86400)),
        ("1w", Some(604800)),
        ("1m", Some(2592000)),
        ("never", None),
    ] {
        for plain in [false, true] {
            let id = if plain {
                let response = harness
                    .request(
                        Method::POST,
                        &format!("/api/paste?expires_in={expiration}&language=text"),
                        "text",
                        &[("content-type", "text/plain")],
                    )
                    .await;
                assert_eq!(response.status(), StatusCode::CREATED);
                body(response).await["id"].as_str().expect("id").to_owned()
            } else {
                harness
                    .create(json!({"content":"text", "expires_in":expiration}))
                    .await
            };
            let read = body(harness.read(&id, &[]).await).await;
            assert_eq!(
                read["expires_at"].as_i64(),
                seconds.map(|seconds| read["created_at"].as_i64().expect("time") + seconds)
            );
        }
    }
}

#[tokio::test]
async fn invalid_requests_are_json_errors_even_when_accepting_html() {
    let harness = Harness::new(1024, None).await;
    for input in [
        r#"{"content":""}"#,
        r#"{"content":"x","expires_in":"2h"}"#,
        r#"{"content":"x","language":"no-such-language"}"#,
        r#"{"content":"x","burn_after_read":null}"#,
        "{",
        "[]",
        r#"{"content":"x","content":"y"}"#,
    ] {
        error(
            harness
                .request(
                    Method::POST,
                    "/api/paste",
                    input,
                    &[
                        ("content-type", "application/json"),
                        ("accept", "text/html"),
                    ],
                )
                .await,
            StatusCode::BAD_REQUEST,
            "bad_request",
        )
        .await;
    }
    for path in [
        "/api/paste?burn_after_read=yes",
        "/api/paste?unknown=1",
        "/api/paste?title=a&title=b",
        "/api/paste?expires_in=2h",
    ] {
        error(
            harness
                .request(
                    Method::POST,
                    path,
                    "text",
                    &[("content-type", "text/plain")],
                )
                .await,
            StatusCode::BAD_REQUEST,
            "bad_request",
        )
        .await;
    }
    for headers in [
        vec![],
        vec![("content-type", "application/xml")],
        vec![("content-type", "text/plain; charset=latin1")],
    ] {
        error(
            harness
                .request(Method::POST, "/api/paste", "text", &headers)
                .await,
            StatusCode::BAD_REQUEST,
            "bad_request",
        )
        .await;
    }
    error(
        harness
            .request(
                Method::POST,
                "/api/paste",
                vec![255],
                &[("content-type", "text/plain")],
            )
            .await,
        StatusCode::BAD_REQUEST,
        "bad_request",
    )
    .await;
    error(
        harness
            .request(
                Method::POST,
                "/api/paste?language=text",
                r#"{"content":"x"}"#,
                &[("content-type", "application/json")],
            )
            .await,
        StatusCode::BAD_REQUEST,
        "bad_request",
    )
    .await;
    for id in ["not-an-id", "Ab12Cd34", "%FF"] {
        error(
            harness.read(id, &[("accept", "text/html")]).await,
            StatusCode::NOT_FOUND,
            "not_found",
        )
        .await;
    }
}

#[tokio::test]
async fn content_limits_cover_bytes_and_json_escape_expansion() {
    let harness = Harness::new(12, None).await;
    for (media, payload) in [
        ("text/plain", "中".repeat(4)),
        (
            "application/json",
            format!(r#"{{"content":"{}"}}"#, "\\u0061".repeat(12)),
        ),
    ] {
        assert_eq!(
            harness
                .request(
                    Method::POST,
                    "/api/paste",
                    payload,
                    &[("content-type", media)]
                )
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    for (media, payload) in [
        ("text/plain", "中".repeat(5)),
        (
            "application/json",
            json!({"content":"中".repeat(5)}).to_string(),
        ),
        ("application/json", " ".repeat(20000)),
    ] {
        error(
            harness
                .request(
                    Method::POST,
                    "/api/paste",
                    payload,
                    &[("content-type", media)],
                )
                .await,
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
        )
        .await;
    }
}

#[tokio::test]
async fn optional_token_protects_only_api_writes_in_both_modes() {
    let harness = Harness::new(1024, Some("test-secret")).await;
    for (media, payload) in [
        ("application/json", r#"{"content":"x"}"#),
        ("text/plain", "x"),
    ] {
        for authorization in [
            None,
            Some("Bearer wrong"),
            Some("Basic test-secret"),
            Some("Bearer test-secret "),
        ] {
            let mut headers = vec![("content-type", media)];
            if let Some(value) = authorization {
                headers.push(("authorization", value));
            }
            error(
                harness
                    .request(Method::POST, "/api/paste", payload, &headers)
                    .await,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            )
            .await;
        }
        let response = harness
            .request(
                Method::POST,
                "/api/paste",
                payload,
                &[
                    ("content-type", media),
                    ("authorization", "bearer test-secret"),
                ],
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let created = body(response).await;
        assert_eq!(
            harness
                .read(created["id"].as_str().expect("id"), &[])
                .await
                .status(),
            StatusCode::OK
        );
    }
    error(
        harness
            .request(
                Method::POST,
                "/api/paste",
                "x",
                &[
                    ("content-type", "text/plain"),
                    ("authorization", "Bearer test-secret"),
                    ("authorization", "Bearer test-secret"),
                ],
            )
            .await,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    let web = harness
        .request(
            Method::POST,
            "/paste",
            "content=web&language=text",
            &[("content-type", "application/x-www-form-urlencoded")],
        )
        .await;
    assert_eq!(web.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn default_limit_accepts_full_size_plain_and_escaped_json() {
    let limit = 1024 * 1024;
    let harness = Harness::new(limit, None).await;
    let response = harness
        .request(
            Method::POST,
            "/api/paste?language=text",
            "a".repeat(limit),
            &[("content-type", "text/plain")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let payload = format!(
        r#"{{"content":"{}","language":"text"}}"#,
        "\\u0061".repeat(limit)
    );
    let response = harness
        .request(
            Method::POST,
            "/api/paste",
            payload,
            &[("content-type", "application/json")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body(response).await;
    let read = body(harness.read(created["id"].as_str().expect("id"), &[]).await).await;
    assert_eq!(read["content"].as_str().expect("content").len(), limit);
    error(
        harness
            .request(
                Method::POST,
                "/api/paste",
                json!({"content": "a".repeat(limit + 1), "language": "text"}).to_string(),
                &[("content-type", "application/json")],
            )
            .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
    )
    .await;
    error(
        harness
            .request(
                Method::POST,
                "/api/paste",
                "a".repeat(limit + 1),
                &[("content-type", "text/plain")],
            )
            .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
    )
    .await;
}

#[tokio::test]
async fn failed_burn_delete_never_leaks_prepared_json() {
    let harness = Harness::new(1024, None).await;
    let id = harness
        .create(json!({"content":"private-marker", "burn_after_read":true}))
        .await;
    sqlx::query("CREATE TRIGGER fail_api_delete BEFORE DELETE ON pastes BEGIN SELECT RAISE(ABORT, 'test failure'); END")
        .execute(&harness.state.pool).await.expect("trigger");
    let response = harness.read(&id, &[]).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let value = body(response).await;
    assert_eq!(
        value,
        json!({"error":{"code":"internal_error", "message":"Internal error"}})
    );
    assert!(paste::metadata(&harness.state, &id).await.is_ok());
    sqlx::query("DROP TRIGGER fail_api_delete")
        .execute(&harness.state.pool)
        .await
        .expect("drop trigger");
    assert_eq!(
        body(harness.read(&id, &[]).await).await["content"],
        "private-marker"
    );
}

#[tokio::test]
async fn password_is_required_before_burning_and_cookies_do_not_authorize_api() {
    let harness = Harness::new(1024, None).await;
    let id = harness
        .create(json!({"content":"secret", "password":"demo", "burn_after_read":true}))
        .await;
    let metadata = paste::metadata(&harness.state, &id)
        .await
        .expect("metadata");
    assert!(
        metadata
            .password_hash
            .as_deref()
            .expect("hash")
            .starts_with("$argon2id$")
    );
    let token = access::issue(&harness.state, &metadata, "access")
        .await
        .expect("token");
    let cookie = format!("{}={token}", access::cookie_name(&id));
    for headers in [
        vec![],
        vec![("x-paste-password", "wrong")],
        vec![("cookie", cookie.as_str())],
    ] {
        error(
            harness.read(&id, &headers).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    assert!(paste::metadata(&harness.state, &id).await.is_ok());
    let response = harness.read(&id, &[("x-paste-password", "demo")]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await["content"], "secret");
    error(
        harness.read(&id, &[("x-paste-password", "demo")]).await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    assert!(
        !db::valid_token(
            &harness.state.pool,
            &id,
            &access::token_hash(&token).expect("hash"),
            "access",
            paste::unix_now().expect("time")
        )
        .await
        .expect("token removed")
    );
}

#[tokio::test]
async fn expired_pastes_are_404_and_head_or_wrong_methods_never_burn() {
    let harness = Harness::new(1024, None).await;
    let id = harness
        .create(json!({"content":"secret", "password":"demo"}))
        .await;
    sqlx::query(
        "UPDATE pastes SET created_at = created_at - 2, expires_at = created_at - 1 WHERE id = ?",
    )
    .bind(&id)
    .execute(&harness.state.pool)
    .await
    .expect("expire");
    error(
        harness.read(&id, &[]).await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
    let burn = harness
        .create(json!({"content":"one time", "burn_after_read":true}))
        .await;
    let path = format!("/api/paste/{burn}");
    let head = harness
        .request(Method::HEAD, &path, Body::empty(), &[])
        .await;
    assert_eq!(head.status(), StatusCode::METHOD_NOT_ALLOWED);
    error(
        harness
            .request(Method::POST, &path, Body::empty(), &[])
            .await,
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    )
    .await;
    error(
        harness
            .request(Method::GET, "/api/paste", Body::empty(), &[])
            .await,
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    )
    .await;
    assert_eq!(harness.read(&burn, &[]).await.status(), StatusCode::OK);
    error(
        harness.read(&burn, &[]).await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
}

#[tokio::test]
async fn concurrent_api_reads_have_one_burn_winner_with_multiple_connections() {
    let directory = tempfile::tempdir().expect("directory");
    let harness = Harness::at(
        1024,
        None,
        format!("sqlite://{}", directory.path().join("api.db").display()),
        5,
    )
    .await;
    let id = harness
        .create(json!({"content":"once", "burn_after_read":true}))
        .await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let app = harness.app.clone();
        let path = format!("/api/paste/{id}");
        tasks.spawn(async move {
            app.oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response")
        });
    }
    let mut winners = 0;
    while let Some(result) = tasks.join_next().await {
        let response = result.expect("task");
        if response.status() == StatusCode::OK {
            winners += 1;
            assert_eq!(body(response).await["content"], "once");
        } else {
            error(response, StatusCode::NOT_FOUND, "not_found").await;
        }
    }
    assert_eq!(winners, 1);
}
