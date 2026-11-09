use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
    response::Response,
};
use pastelet::{
    AppState,
    config::Config,
    db, routes,
    services::{access, cleanup, paste},
};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

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
            max_content_bytes: 1048576,
            database_max_connections: connections,
            cleanup_interval: Duration::from_secs(60),
            api_token: None,
            rust_log: "error".into(),
        };
        let state = AppState {
            pool: db::connect(&config).await.expect("db"),
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
        fields: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/x-www-form-urlencoded");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields.iter().copied())
            .finish();
        self.app
            .clone()
            .oneshot(request.body(Body::from(body)).expect("request"))
            .await
            .expect("response")
    }

    async fn create(&self, password: &str, burn: bool) -> String {
        let response = self
            .request(
                Method::POST,
                "/paste",
                &[
                    ("title", "Secret title"),
                    ("content", "private-payload"),
                    ("password", password),
                    ("burn_after_read", if burn { "true" } else { "false" }),
                ],
                &[],
            )
            .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        response.headers()["location"]
            .to_str()
            .expect("location")
            .trim_start_matches('/')
            .into()
    }

    async fn unlock(&self, id: &str, password: &str, target: &str) -> Response {
        self.request(
            Method::POST,
            &format!("/unlock/{id}"),
            &[("password", password), ("target", target)],
            &[("accept", "text/html"), ("origin", "https://paste.example")],
        )
        .await
    }

    async fn confirm_token(&self, id: &str, cookie: &str) -> String {
        let response = self
            .request(
                Method::GET,
                &format!("/{id}"),
                &[],
                &[("accept", "text/html"), ("cookie", cookie)],
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = text(response).await;
        assert!(!html.contains("private-payload"));
        assert!(!html.contains("Secret title"));
        html.split("name=\"token\" value=\"")
            .nth(1)
            .expect("confirmation token")
            .split('"')
            .next()
            .expect("value")
            .into()
    }

    async fn consume(&self, id: &str, token: &str, cookie: &str, target: &str) -> Response {
        self.request(
            Method::POST,
            &format!("/read/{id}"),
            &[("token", token), ("target", target)],
            &[
                ("accept", "text/html"),
                ("cookie", cookie),
                ("origin", "https://paste.example"),
            ],
        )
        .await
    }

    async fn exists(&self, id: &str) -> bool {
        db::find_live_paste(&self.state.pool, id, paste::unix_now().expect("clock"))
            .await
            .expect("query")
            .is_some()
    }
}

async fn text(response: Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf8")
}

fn cookie(response: &Response) -> String {
    response.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .into()
}

#[tokio::test]
async fn password_unlock_guards_every_path_and_uses_hashed_expiring_tokens() {
    let harness = Harness::new().await;
    let id = harness.create("correct horse", false).await;
    let stored = db::find_live_paste(&harness.state.pool, &id, 0)
        .await
        .expect("db")
        .expect("paste");
    assert!(
        stored
            .password_hash
            .expect("hash")
            .starts_with("$argon2id$")
    );
    for prefix in ["", "/raw", "/download", "/fork"] {
        let path = format!("{prefix}/{id}");
        for headers in [
            vec![],
            vec![("x-paste-password", "wrong")],
            vec![("cookie", "pastelet_access_fake=fake")],
        ] {
            let response = harness.request(Method::GET, &path, &[], &headers).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(!text(response).await.contains("private-payload"));
        }
        let response = harness
            .request(Method::GET, &path, &[], &[("accept", "text/html")])
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            text(response)
                .await
                .contains(&format!("action=\"/unlock/{id}\""))
        );
    }
    let wrong = harness.unlock(&id, "wrong", "view").await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert!(!wrong.headers().contains_key("set-cookie"));
    let response = harness.unlock(&id, "correct horse", "raw").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], format!("/raw/{id}"));
    let set_cookie = response.headers()["set-cookie"].to_str().expect("header");
    for flag in ["HttpOnly", "SameSite=Strict", "Secure", "Max-Age=3600"] {
        assert!(set_cookie.contains(flag));
    }
    assert!(!set_cookie.contains("correct horse"));
    let cookie = cookie(&response);
    let token = cookie.split_once('=').expect("pair").1;
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM paste_tokens WHERE paste_id = ?")
            .bind(&id)
            .fetch_one(&harness.state.pool)
            .await
            .expect("token");
    assert_eq!(stored, access::token_hash(token).expect("hash"));
    assert_ne!(stored, token.as_bytes());
    for prefix in ["", "/raw", "/download", "/fork"] {
        let response = harness
            .request(
                Method::GET,
                &format!("{prefix}/{id}"),
                &[],
                &[("cookie", &cookie)],
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(text(response).await.contains("private-payload"));
    }
    let by_header = harness
        .request(
            Method::GET,
            &format!("/raw/{id}"),
            &[],
            &[("x-paste-password", "correct horse")],
        )
        .await;
    assert_eq!(text(by_header).await, "private-payload");
    sqlx::query("UPDATE paste_tokens SET expires_at = 1")
        .execute(&harness.state.pool)
        .await
        .expect("expire token");
    assert_eq!(
        harness
            .request(
                Method::GET,
                &format!("/raw/{id}"),
                &[],
                &[("cookie", &cookie)]
            )
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn native_confirmation_with_fresh_browser_headers_reads_once() {
    let harness = Harness::new().await;
    let id = harness.create("", true).await;
    let page = harness
        .request(
            Method::GET,
            &format!("/{id}"),
            &[],
            &[("accept", "text/html")],
        )
        .await;
    let origin = match page.headers()["referrer-policy"].to_str().expect("policy") {
        "no-referrer" => "null",
        "same-origin" => "https://paste.example",
        policy => panic!("unexpected referrer policy: {policy}"),
    };
    let html = text(page).await;
    let token = html
        .split("name=\"token\" value=\"")
        .nth(1)
        .expect("token")
        .split('"')
        .next()
        .expect("value");
    let response = harness
        .request(
            Method::POST,
            &format!("/read/{id}"),
            &[("token", token), ("target", "view")],
            &[
                ("accept", "text/html"),
                ("origin", origin),
                ("sec-fetch-site", "same-origin"),
                ("sec-fetch-mode", "navigate"),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(text(response).await.contains("private-payload"));
    assert!(!harness.exists(&id).await);
    assert_eq!(
        harness.consume(&id, token, "", "view").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn opaque_origin_still_cannot_unlock_or_burn() {
    let harness = Harness::new().await;
    let id = harness.create("", true).await;
    let token = harness.confirm_token(&id, "").await;
    let response = harness
        .request(
            Method::POST,
            &format!("/read/{id}"),
            &[("token", &token)],
            &[("origin", "null"), ("sec-fetch-site", "same-origin")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(text(response).await.contains("Cross-site request rejected"));
    assert!(harness.exists(&id).await);
    let protected = harness.create("secret", true).await;
    let response = harness
        .request(
            Method::POST,
            &format!("/unlock/{protected}"),
            &[("password", "secret")],
            &[("origin", "null")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(!response.headers().contains_key("set-cookie"));
    assert!(harness.exists(&protected).await);
}

#[tokio::test]
async fn previews_and_invalid_confirmations_never_burn() {
    let harness = Harness::new().await;
    let id = harness.create("", true).await;
    for prefix in ["", "/raw", "/download", "/fork"] {
        for method in [Method::GET, Method::HEAD] {
            let response = harness
                .request(
                    method,
                    &format!("{prefix}/{id}"),
                    &[],
                    &[("accept", "text/html"), ("user-agent", "link-preview")],
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(!text(response).await.contains("private-payload"));
            assert!(harness.exists(&id).await);
        }
    }
    assert_eq!(
        harness
            .request(Method::GET, &format!("/raw/{id}"), &[], &[])
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        harness.consume(&id, "invalid", "", "view").await.status(),
        StatusCode::BAD_REQUEST
    );
    let token = harness.confirm_token(&id, "").await;
    let cross_site = harness
        .request(
            Method::POST,
            &format!("/read/{id}"),
            &[("token", &token)],
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(cross_site.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        harness
            .consume(&id, &token, "", "//evil.example")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(harness.exists(&id).await);
    let response = harness.consume(&id, &token, "", "view").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let html = text(response).await;
    assert!(html.contains("private-payload"));
    assert!(html.contains("data-burned=\"true\""));
    assert!(html.contains("id=\"burned-fork\""));
    assert!(!harness.exists(&id).await);
    assert_eq!(
        harness.consume(&id, &token, "", "view").await.status(),
        StatusCode::NOT_FOUND
    );
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM paste_tokens")
        .fetch_one(&harness.state.pool)
        .await
        .expect("count");
    assert_eq!(tokens, 0);
}

#[tokio::test]
async fn password_and_burn_require_unlock_then_confirmation() {
    let harness = Harness::new().await;
    let id = harness.create("password", true).await;
    let locked = harness
        .request(
            Method::GET,
            &format!("/{id}"),
            &[],
            &[("accept", "text/html")],
        )
        .await;
    assert_eq!(locked.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(locked.headers()["referrer-policy"], "same-origin");
    let wrong = harness.unlock(&id, "wrong", "view").await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert!(harness.exists(&id).await);
    let unlocked = harness.unlock(&id, "password", "view").await;
    assert_eq!(unlocked.status(), StatusCode::SEE_OTHER);
    let cookie = cookie(&unlocked);
    assert!(harness.exists(&id).await);
    let token = harness.confirm_token(&id, &cookie).await;
    assert_eq!(
        harness.consume(&id, &token, "", "view").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(harness.exists(&id).await);
    assert_eq!(
        harness.consume(&id, &token, &cookie, "view").await.status(),
        StatusCode::OK
    );
    assert!(!harness.exists(&id).await);
}

#[tokio::test]
async fn simultaneous_readers_have_exactly_one_winner_on_multiple_connections() {
    let directory = tempfile::tempdir().expect("directory");
    let harness = Harness::at(
        format!("sqlite://{}", directory.path().join("race.db").display()),
        5,
    )
    .await;
    let id = harness.create("", true).await;
    let first_token = harness.confirm_token(&id, "").await;
    let second_token = harness.confirm_token(&id, "").await;
    let third_token = harness.confirm_token(&id, "").await;
    let (first, second, third) = tokio::join!(
        harness.consume(&id, &first_token, "", "view"),
        harness.consume(&id, &second_token, "", "raw"),
        harness.consume(&id, &third_token, "", "download")
    );
    let mut winners = 0;
    for response in [first, second, third] {
        if response.status() == StatusCode::OK {
            winners += 1;
            assert!(text(response).await.contains("private-payload"));
        } else {
            assert!([StatusCode::NOT_FOUND, StatusCode::BAD_REQUEST].contains(&response.status()));
            assert!(!text(response).await.contains("private-payload"));
        }
    }
    assert_eq!(winners, 1);
    assert!(!harness.exists(&id).await);
}

#[tokio::test]
async fn burned_download_and_view_snapshot_keep_language_filename() {
    let harness = Harness::new().await;
    for (language, extension, target) in [("markdown", "md", "view"), ("HTML", "html", "download")]
    {
        let id = harness.create("", true).await;
        sqlx::query("UPDATE pastes SET language = ? WHERE id = ?")
            .bind(language)
            .bind(&id)
            .execute(&harness.state.pool)
            .await
            .expect("language");
        let token = harness.confirm_token(&id, "").await;
        let response = harness.consume(&id, &token, "", target).await;
        assert_eq!(response.status(), StatusCode::OK);
        if target == "view" {
            assert!(
                text(response)
                    .await
                    .contains(&format!("data-filename=\"{id}.{extension}\""))
            );
        } else {
            assert_eq!(
                response.headers()["content-disposition"],
                format!("attachment; filename=\"{id}.{extension}\"")
            );
            assert_eq!(text(response).await, "private-payload");
        }
        assert!(!harness.exists(&id).await);
    }
}

#[tokio::test]
async fn all_confirmed_destinations_work_and_snapshot_fork_does_not_insert() {
    let harness = Harness::new().await;
    for target in ["raw", "download", "fork"] {
        let id = harness.create("", true).await;
        let token = harness.confirm_token(&id, "").await;
        let response = harness.consume(&id, &token, "", target).await;
        assert_eq!(response.status(), StatusCode::OK);
        if target == "download" {
            assert!(
                response.headers()["content-disposition"]
                    .to_str()
                    .expect("header")
                    .starts_with("attachment")
            );
        }
        assert!(text(response).await.contains("private-payload"));
        assert!(!harness.exists(&id).await);
    }
    let response = harness
        .request(
            Method::POST,
            "/fork",
            &[
                ("content", "<script>hi</script>"),
                ("language", "text"),
                ("title", "Original"),
            ],
            &[],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = text(response).await;
    assert!(!html.contains("<script>hi"));
    assert!(html.contains("value=\"1d\" selected"));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pastes")
        .fetch_one(&harness.state.pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn expired_tokens_and_pastes_are_removed_without_touching_live_records() {
    let harness = Harness::new().await;
    let expired = harness.create("", true).await;
    let live = harness.create("", true).await;
    let _ = harness.confirm_token(&expired, "").await;
    let token = harness.confirm_token(&live, "").await;
    sqlx::query("UPDATE pastes SET created_at = 1, expires_at = 2 WHERE id = ?")
        .bind(&expired)
        .execute(&harness.state.pool)
        .await
        .expect("expire");
    sqlx::query("UPDATE paste_tokens SET expires_at = 1 WHERE paste_id = ?")
        .bind(&live)
        .execute(&harness.state.pool)
        .await
        .expect("expire token");
    assert_eq!(
        harness.consume(&live, &token, "", "view").await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        harness.consume(&expired, &token, "", "view").await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(!harness.exists(&expired).await);
    assert_eq!(
        cleanup::sweep(&harness.state.pool).await.expect("cleanup"),
        0
    );
    assert!(harness.exists(&live).await);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM paste_tokens")
        .fetch_one(&harness.state.pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
    assert_eq!(
        cleanup::sweep(&harness.state.pool)
            .await
            .expect("idempotent cleanup"),
        0
    );
}

#[tokio::test]
async fn tokens_are_bound_to_paste_and_purpose_and_cannot_extend_paste_lifetime() {
    let harness = Harness::new().await;
    let first = harness.create("password", true).await;
    let second = harness.create("password", true).await;
    let expiry = paste::unix_now().expect("clock") + 60;
    sqlx::query("UPDATE pastes SET expires_at = ? WHERE id = ?")
        .bind(expiry)
        .bind(&first)
        .execute(&harness.state.pool)
        .await
        .expect("expiry");
    let unlocked = harness.unlock(&first, "password", "view").await;
    let first_cookie = cookie(&unlocked);
    let access_token = first_cookie.split_once('=').expect("cookie").1;
    let confirmation = harness.confirm_token(&first, &first_cookie).await;
    let wrong_paste_cookie = format!("{}={access_token}", access::cookie_name(&second));
    assert_eq!(
        harness
            .request(
                Method::GET,
                &format!("/raw/{second}"),
                &[],
                &[("cookie", &wrong_paste_cookie)]
            )
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let wrong_kind_cookie = format!("{}={confirmation}", access::cookie_name(&first));
    assert_eq!(
        harness
            .request(
                Method::GET,
                &format!("/raw/{first}"),
                &[],
                &[("cookie", &wrong_kind_cookie)]
            )
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        harness
            .consume(&first, access_token, &first_cookie, "view")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let second_unlocked = harness.unlock(&second, "password", "view").await;
    assert_eq!(
        harness
            .consume(&second, &confirmation, &cookie(&second_unlocked), "view")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let expiry_values: Vec<i64> =
        sqlx::query_scalar("SELECT expires_at FROM paste_tokens WHERE paste_id = ?")
            .bind(&first)
            .fetch_all(&harness.state.pool)
            .await
            .expect("tokens");
    assert_eq!(expiry_values.len(), 2);
    assert!(expiry_values.iter().all(|value| *value == expiry));
    assert!(harness.exists(&first).await && harness.exists(&second).await);
}

#[tokio::test]
async fn failed_delete_never_returns_prepared_content_and_can_be_retried() {
    let harness = Harness::new().await;
    let id = harness.create("", true).await;
    let token = harness.confirm_token(&id, "").await;
    sqlx::query("CREATE TRIGGER reject_delete BEFORE DELETE ON pastes BEGIN SELECT RAISE(ABORT, 'test failure'); END")
        .execute(&harness.state.pool).await.expect("trigger");
    let response = harness.consume(&id, &token, "", "view").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!text(response).await.contains("private-payload"));
    assert!(harness.exists(&id).await);
    sqlx::query("DROP TRIGGER reject_delete")
        .execute(&harness.state.pool)
        .await
        .expect("remove trigger");
    assert_eq!(
        harness.consume(&id, &token, "", "view").await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn alternate_origins_unlock_and_burn_with_transport_appropriate_cookies() {
    let mut harness = Harness::new().await;
    let origins = [
        "https://paste.example",
        "http://100.64.0.10:8007",
        "http://my-node:8007",
        "https://my-node.tail-example.ts.net",
    ];
    Arc::make_mut(&mut harness.state.config).allowed_origins = origins[1..]
        .iter()
        .map(|origin| (*origin).to_owned())
        .collect();
    harness.app = routes::router(harness.state.clone());
    for origin in origins {
        let id = harness.create("secret", true).await;
        let response = harness
            .request(
                Method::POST,
                &format!("/unlock/{id}"),
                &[("password", "secret")],
                &[
                    ("origin", origin),
                    ("sec-fetch-site", "same-origin"),
                    ("host", "internal-proxy:3000"),
                    ("x-forwarded-proto", "http"),
                ],
            )
            .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let header = response.headers()["set-cookie"].to_str().expect("cookie");
        assert_eq!(header.contains("; Secure"), origin.starts_with("https://"));
        assert!(header.contains("HttpOnly; SameSite=Strict"));
        assert!(!header.contains("Domain="));
        let cookie = cookie(&response);
        let token = harness.confirm_token(&id, &cookie).await;
        let response = harness
            .request(
                Method::POST,
                &format!("/read/{id}"),
                &[("token", &token)],
                &[("origin", origin), ("cookie", &cookie)],
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(text(response).await.contains("private-payload"));
        assert!(!harness.exists(&id).await);
    }
    let fallback = access::cookie(
        &harness.state,
        "Ab12Cd34",
        "token",
        &axum::http::HeaderMap::new(),
    )
    .expect("cookie");
    assert!(fallback.contains("; Secure"));
}

#[tokio::test]
async fn origin_allowlist_does_not_trust_proxy_headers_or_cross_site_forms() {
    let mut harness = Harness::new().await;
    Arc::make_mut(&mut harness.state.config).allowed_origins = vec!["http://my-node:8007".into()];
    harness.app = routes::router(harness.state.clone());
    let id = harness.create("secret", false).await;
    for origin in [
        "https://evil.example",
        "http://my-node:8008",
        "https://my-node:8007",
        "http://my-node:8007.evil.example",
        "http://my-node:8007/path",
        "null",
        "http://my-node:8007 https://paste.example",
    ] {
        let response = harness
            .request(
                Method::POST,
                &format!("/unlock/{id}"),
                &[("password", "secret")],
                &[
                    ("origin", origin),
                    ("host", "my-node:8007"),
                    ("x-forwarded-host", "my-node:8007"),
                    ("x-forwarded-proto", "http"),
                ],
            )
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{origin}");
        assert!(!response.headers().contains_key("set-cookie"));
    }
    let response = harness
        .request(
            Method::POST,
            &format!("/unlock/{id}"),
            &[("password", "secret")],
            &[
                ("origin", "http://my-node:8007"),
                ("sec-fetch-site", "cross-site"),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let mut headers = axum::http::HeaderMap::new();
    headers.append("origin", "https://paste.example".parse().expect("origin"));
    headers.append("origin", "http://my-node:8007".parse().expect("origin"));
    assert!(access::same_origin(&harness.state, &headers).is_err());
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM paste_tokens")
        .fetch_one(&harness.state.pool)
        .await
        .expect("tokens");
    assert_eq!(tokens, 0);
}

#[tokio::test]
async fn unlock_persists_across_restart_and_cross_site_unlock_is_rejected() {
    let directory = tempfile::tempdir().expect("directory");
    let url = format!("sqlite://{}", directory.path().join("persist.db").display());
    let harness = Harness::at(url.clone(), 2).await;
    let id = harness.create("password", false).await;
    let blocked = harness
        .request(
            Method::POST,
            &format!("/unlock/{id}"),
            &[("password", "password")],
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(blocked.status(), StatusCode::BAD_REQUEST);
    assert!(!blocked.headers().contains_key("set-cookie"));
    let unlocked = harness.unlock(&id, "password", "view").await;
    let cookie = cookie(&unlocked);
    harness.state.pool.close().await;
    let reopened = Harness::at(url, 2).await;
    assert_eq!(
        reopened
            .request(
                Method::GET,
                &format!("/raw/{id}"),
                &[],
                &[("cookie", &cookie)]
            )
            .await
            .status(),
        StatusCode::OK
    );
    reopened.state.pool.close().await;
}

#[tokio::test]
async fn cleanup_runs_periodically_and_stops_cleanly() {
    let harness = Harness::new().await;
    let never = harness.create("", false).await;
    sqlx::query("UPDATE pastes SET expires_at = NULL WHERE id = ?")
        .bind(&never)
        .execute(&harness.state.pool)
        .await
        .expect("never");
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(cleanup::run(
        harness.state.pool.clone(),
        Duration::from_millis(10),
        receiver,
    ));
    for _ in 0..2 {
        let id = harness.create("", false).await;
        sqlx::query("UPDATE pastes SET created_at = 1, expires_at = 2 WHERE id = ?")
            .bind(&id)
            .execute(&harness.state.pool)
            .await
            .expect("expire");
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pastes WHERE id = ?")
                    .bind(&id)
                    .fetch_one(&harness.state.pool)
                    .await
                    .expect("count");
                if count == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("periodic cleanup");
    }
    sender.send(true).expect("stop");
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("shutdown deadline")
        .expect("task");
    assert!(harness.exists(&never).await);
}
