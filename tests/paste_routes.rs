use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rpaste::{AppState, config::Config, db, models::Paste, routes, services::paste::valid_id};
use sqlx::SqlitePool;
use tower::ServiceExt;

struct Harness {
    app: Router,
    pool: SqlitePool,
}

impl Harness {
    async fn new(limit: usize) -> Self {
        let config = Config {
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            database_url: "sqlite::memory:".into(),
            base_url: "http://localhost:3000".into(),
            max_content_bytes: limit,
            database_max_connections: 1,
            cleanup_interval: Duration::from_secs(60),
            api_token: None,
            rust_log: "rpaste=info".into(),
        };
        let pool = db::connect(&config).await.expect("database");
        let app = routes::router(AppState {
            config: Arc::new(config),
            pool: pool.clone(),
        });
        Self { app, pool }
    }

    async fn request(&self, method: Method, path: &str, body: String) -> Response {
        self.app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    async fn get(&self, path: &str) -> Response {
        self.request(Method::GET, path, String::new()).await
    }

    async fn browser(&self, method: Method, path: &str, body: &str, htmx: bool) -> Response {
        self.app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("accept", "text/html")
                    .header("hx-request", if htmx { "true" } else { "false" })
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body.to_owned()))
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    async fn post(&self, fields: &[(&str, &str)]) -> Response {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields.iter().copied())
            .finish();
        self.request(Method::POST, "/paste", encoded).await
    }

    async fn create(&self, fields: &[(&str, &str)]) -> String {
        let response = self.post(fields).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response.headers()["location"].to_str().expect("location");
        let id = location.strip_prefix('/').expect("local redirect");
        assert!(valid_id(id));
        id.to_owned()
    }

    async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pastes")
            .fetch_one(&self.pool)
            .await
            .expect("count")
    }

    async fn paste(&self, id: &str) -> Paste {
        sqlx::query_as("SELECT * FROM pastes WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("stored paste")
    }
}

async fn text(response: Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("UTF-8")
}

async fn assert_error(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body: serde_json::Value = serde_json::from_str(&text(response).await).expect("JSON error");
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn browser_errors_are_html_but_raw_errors_remain_json() {
    let harness = Harness::new(1024).await;
    for (method, path, status) in [
        (Method::GET, "/missing", StatusCode::NOT_FOUND),
        (Method::PUT, "/paste", StatusCode::METHOD_NOT_ALLOWED),
    ] {
        let response = harness.browser(method, path, "", false).await;
        assert_eq!(response.status(), status);
        assert_eq!(
            response.headers()["content-type"],
            "text/html; charset=utf-8"
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = text(response).await;
        assert!(body.contains("class=\"state-icon\""));
        assert!(body.contains(">Home</a>"));
        assert!(!body.contains("id=\"theme-picker\""));
    }
    assert_error(
        harness
            .browser(Method::GET, "/raw/abcdefgh", "", false)
            .await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
}

#[tokio::test]
async fn htmx_redirects_and_validation_fragments_keep_the_form_intact() {
    let harness = Harness::new(8).await;
    let response = harness
        .browser(Method::POST, "/paste", "content=hello", true)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(valid_id(
        response.headers()["hx-redirect"]
            .to_str()
            .expect("redirect")
            .trim_start_matches('/')
    ));
    assert!(!response.headers().contains_key("location"));
    for (body, status) in [
        ("content=", StatusCode::BAD_REQUEST),
        ("content=123456789", StatusCode::PAYLOAD_TOO_LARGE),
        ("content=hello&unknown=secret", StatusCode::BAD_REQUEST),
    ] {
        let response = harness.browser(Method::POST, "/paste", body, true).await;
        assert_eq!(response.status(), status);
        let fragment = text(response).await;
        assert!(fragment.starts_with("<p class=\"form-error\">"));
        assert!(!fragment.contains("<form"));
        assert!(!fragment.contains("<html"));
        assert!(!fragment.contains("secret"));
    }
    assert_eq!(harness.count().await, 1);
}

#[tokio::test]
async fn non_javascript_validation_preserves_and_escapes_input() {
    let harness = Harness::new(8).await;
    let response = harness.browser(Method::POST, "/paste",
        "content=%3Cscript%3Ebad%3C%2Fscript%3E&title=%22%3E%3Cimg%3E&language=text&expires_in=1w", false).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = text(response).await;
    assert!(
        body.contains("&#60;script&#62;bad&#60;/script&#62;")
            || body.contains("&lt;script&gt;bad&lt;/script&gt;")
    );
    assert!(!body.contains("<script>bad"));
    assert!(body.contains("value=\"text\" selected"));
    assert!(body.contains("value=\"1w\" selected"));
    assert!(body.contains("Content too large"));
    assert_eq!(harness.count().await, 0);
}

#[tokio::test]
async fn browser_gate_templates_never_return_or_delete_protected_content() {
    let harness = Harness::new(1024).await;
    for (password, burn, status, label) in [
        (Some("hash"), false, StatusCode::UNAUTHORIZED, "Locked"),
        (None, true, StatusCode::OK, "Burn"),
        (Some("hash"), true, StatusCode::UNAUTHORIZED, "Locked"),
    ] {
        let id = harness
            .create(&[("content", "private-payload"), ("title", "private-title")])
            .await;
        sqlx::query("UPDATE pastes SET password_hash = ?, burn_after_read = ? WHERE id = ?")
            .bind(password)
            .bind(burn)
            .bind(&id)
            .execute(&harness.pool)
            .await
            .expect("flags");
        for path in [format!("/{id}"), format!("/fork/{id}")] {
            let response = harness.browser(Method::GET, &path, "", false).await;
            assert_eq!(response.status(), status);
            let body = text(response).await;
            assert!(body.contains(&format!("<title>{label} · rpaste</title>")));
            assert!(body.contains("type=\"submit\""));
            assert!(!body.contains("private-payload"));
            assert!(!body.contains("private-title"));
            assert!(!body.contains("data-raw"));
        }
        assert_eq!(harness.paste(&id).await.content, "private-payload");
        sqlx::query("UPDATE pastes SET created_at = 1, expires_at = 2 WHERE id = ?")
            .bind(&id)
            .execute(&harness.pool)
            .await
            .expect("expire");
        assert_eq!(
            harness
                .browser(Method::GET, &format!("/{id}"), "", false)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn formal_ui_resources_and_privacy_settings_are_embedded() {
    let harness = Harness::new(1024).await;
    let body = text(harness.get("/").await).await;
    for expected in [
        "hx-post=\"/paste\"",
        "hx-history=\"false\"",
        "\"allowEval\":false",
        "\"historyCacheSize\":0",
        "id=\"theme-picker\"",
        "name=\"burn_after_read\"",
        "id=\"create-paste\"",
        "class=\"site-toolbar create-toolbar\"",
        "aria-controls=\"create-settings\"",
        "aria-controls=\"advanced-fields\"",
        "class=\"mobile-create\"",
        "aria-keyshortcuts=\"Control+Enter\"",
        "id=\"editor-gutter\"",
        "wrap=\"off\"",
        "src=\"/static/editor.js\"",
        "id=\"upload-button\"",
        "aria-label=\"上传文件\"",
        "id=\"upload-file\" type=\"file\" hidden",
        "id=\"upload-message\" role=\"status\"",
        "id=\"upload-replace\" class=\"control\" type=\"button\"",
        "松手即可上传",
    ] {
        assert!(body.contains(expected), "missing {expected}");
    }
    let htmx = harness.get("/static/htmx.min.js").await;
    assert_eq!(htmx.status(), StatusCode::OK);
    assert_eq!(
        htmx.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert!(text(htmx).await.contains("2.0.8"));
    let styles = include_str!("../static/styles.css");
    assert!(styles.contains("calc(100dvh - var(--toolbar-height) - 6rem - 2px)"));
    assert!(styles.contains(".site-toolbar { position: fixed;"));
    assert!(styles.contains(".create-submit { background: #9bb9ff;"));
    assert!(styles.contains("(max-width: 767px)"));
    for name in [
        "language",
        "expires_in",
        "password",
        "burn_after_read",
        "content",
        "title",
    ] {
        assert_eq!(body.matches(&format!("name=\"{name}\"")).count(), 1);
    }
    assert_eq!(body.matches("<form ").count(), 1);
    assert!(body.find("name=\"language\"") < body.find("</header>"));
    assert_eq!(body.matches("id=\"upload-button\"").count(), 1);
    let upload_start = body.find("id=\"upload-button\"").expect("upload button");
    let upload_end = upload_start + body[upload_start..].find("</button>").expect("button end");
    let upload = &body[upload_start..upload_end];
    assert!(upload.contains("title=\"上传文件\""));
    assert!(!upload.contains("</svg>上传文件"));
    assert!(
        upload_end
            < body
                .find("class=\"create-actions\"")
                .expect("theme divider")
    );
    assert!(upload_end < body.find("id=\"theme-picker\"").expect("themes"));
    assert!(upload_end < body.find("</header>").expect("header end"));
    assert!(body.find("</header>") < body.find("<textarea"));
    assert!(styles.contains("calc(100dvh - var(--toolbar-height) - 2rem)"));
    assert!(styles.contains(".source-view, .markdown"));
    assert!(styles.contains("min-height: var(--reader-height)"));
    assert!(!body.contains("max-w-6xl"));
    assert!(!body.contains("id=\"editor-wrap\""));
    assert!(!body.contains("id=\"reader-wrap\""));
    for theme in ["system", "light", "dark"] {
        assert_eq!(
            body.matches(&format!("data-theme-icon=\"{theme}\""))
                .count(),
            1
        );
        assert!(body.contains(&format!("data-theme-choice=\"{theme}\"")));
    }
    assert!(styles.contains("[data-theme-icon] { display: block; }"));
    let id = harness
        .create(&[("content", "# One"), ("language", "markdown")])
        .await;
    let body = text(harness.get(&format!("/{id}")).await).await;
    assert_eq!(body.matches(" hx-disable").count(), 2);
}

#[tokio::test]
async fn editor_and_reader_share_home_link_header_and_code_metrics() {
    let harness = Harness::new(1024).await;
    let create = text(harness.get("/").await).await;
    let id = harness
        .create(&[("content", "hello"), ("title", "Header alignment")])
        .await;
    let read = text(harness.get(&format!("/{id}")).await).await;
    let home = include_str!("../templates/home.html").trim();
    for html in [&create, &read] {
        assert!(html.contains(home));
        assert!(html.contains("<header class=\"site-toolbar"));
        assert!(html.contains("<main class=\"site-main"));
        assert_eq!(html.matches("id=\"theme-picker\"").count(), 1);
        assert!(!html.contains("rpaste<span"));
    }
    assert!(create.contains("<title>rpaste</title>"));
    assert!(read.contains("<title>Header alignment · rpaste</title>"));
    assert!(create.contains("class=\"language-group\""));
    assert!(create.contains("id=\"editor-active-line\""));
    assert!(read.contains("class=\"toolbar-actions\""));
    let header_end = read.find("</header>").expect("header end");
    for control in [
        "copy-paste",
        "reader-menu-toggle",
        "reader-menu",
        "theme-picker",
    ] {
        assert!(read.find(&format!("id=\"{control}\"")).expect("control") < header_end);
    }
    assert!(
        read.find("<h1 title=\"Header alignment\"")
            .expect("file title")
            < header_end
    );
    assert!(!read[header_end..].contains("<h1"));
    assert!(create.find("id=\"upload-button\"") < create.find("id=\"settings-toggle\""));
    assert!(create.find("id=\"upload-button\"") < create.find("id=\"create-settings\""));
    assert!(create.contains("<dialog id=\"upload-notice\""));
    let styles = include_str!("../static/styles.css");
    assert!(styles.contains(".editor-input, .editor-gutter, .source-lines { font:"));
    assert!(styles.contains(".editor-input:focus-visible { outline: none; }"));
    assert!(!styles.contains(".editor-line.is-active {"));
    let gutter = styles
        .split(".editor-gutter {")
        .nth(1)
        .expect("editor gutter rule")
        .split('}')
        .next()
        .expect("editor gutter declarations");
    assert!(gutter.contains("border-right: 1px solid transparent;"));
    assert!(styles.contains(".home-link:hover, .theme-picker .icon-button:hover"));
    assert!(styles.contains("width: var(--create-width)"));
    assert!(styles.contains("padding: 0 var(--page-gutter)"));
    assert!(styles.contains("--toolbar-height: 4rem;"));
    assert!(styles.contains("text-overflow: ellipsis; white-space: nowrap;"));
}

#[tokio::test]
async fn reader_toolbar_keeps_long_escaped_filename_and_all_actions_outside_document() {
    let harness = Harness::new(4096).await;
    let title = format!("{}\"<file>", "长".repeat(180));
    let id = harness
        .create(&[
            ("title", &title),
            ("content", "# Document heading\n\nBody"),
            ("language", "markdown"),
        ])
        .await;
    let html = text(harness.get(&format!("/{id}")).await).await;
    let header_end = html.find("</header>").expect("fixed header");
    let header = &html[..header_end];
    let main = &html[header_end..];
    assert!(header.contains(&"长".repeat(180)));
    assert!(!header.contains("\"<file>"));
    assert!(header.contains("<h1 title=\""));
    assert!(main.contains("<h1>Document heading</h1>"));
    assert!(!main.contains(&"长".repeat(180)));
    for action in ["Copy", "Download", "Raw", "Fork", "Source"] {
        assert_eq!(
            header.matches(&format!("aria-label=\"{action}\"")).count(),
            1
        );
        assert!(!main.contains(&format!("aria-label=\"{action}\"")));
    }
    assert_eq!(html.matches("<header ").count(), 1);
    assert!(header.contains("aria-controls=\"reader-menu\""));
}

#[tokio::test]
async fn plain_text_and_code_keep_their_wrap_rules_in_reader_and_fork() {
    let harness = Harness::new(1048576).await;
    for (language, plain) in [("text", true), ("Rust", false)] {
        let content = format!("{}\n\n<script>example</script>", "word ".repeat(300));
        let id = harness
            .create(&[("content", &content), ("language", language)])
            .await;
        let page = text(harness.get(&format!("/{id}")).await).await;
        assert_eq!(page.contains("source-view is-wrapped"), plain);
        assert_eq!(page.matches("class=\"line-numbers\"").count(), 3);
        assert_eq!(page.matches("id=\"reader-wrap\"").count(), 1);
        assert!(page.contains("id=\"L3\""));
        assert!(!page.contains("<script>example</script>"));
        let fork = text(harness.get(&format!("/fork/{id}")).await).await;
        let wrap = if plain { "soft" } else { "off" };
        assert!(fork.contains(&format!("wrap=\"{wrap}\"")));
        assert_eq!(fork.matches("name=\"content\"").count(), 1);
        assert!(!fork.contains("id=\"reader-wrap\""));
        assert!(!fork.contains("id=\"editor-wrap\""));
        let raw = text(harness.get(&format!("/raw/{id}")).await).await;
        assert_eq!(raw, content);
    }
}

#[tokio::test]
async fn create_view_raw_download_and_copy_preserve_original_content() {
    let harness = Harness::new(1024 * 1024).await;
    let content = "\u{feff}\r\nfn main() {\r\n    println!(\"中文 <script> & \\\"\");\r\n}\r\n";
    let id = harness
        .create(&[
            ("content", content),
            ("title", "<img src=x onerror=alert(1)>"),
            ("language", "rs"),
        ])
        .await;
    let stored = harness.paste(&id).await;
    assert_eq!(stored.content, content);
    assert_eq!(stored.language, "Rust");
    assert_eq!(stored.expires_at, Some(stored.created_at + 86400));
    assert!(stored.password_hash.is_none());
    assert!(!stored.burn_after_read);

    let response = harness.get(&format!("/{id}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "same-origin");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let html = text(response).await;
    assert!(html.contains("syn-storage"));
    assert!(html.contains("id=\"L2\" href=\"#L2\""));
    assert!(!html.contains("<img src=x onerror="));
    assert!(!html.contains("id=\"rpaste-rendered\""));
    let encoded = html
        .split("data-raw=\"")
        .nth(1)
        .expect("copy data")
        .split('"')
        .next()
        .expect("encoded content");
    assert_eq!(
        STANDARD.decode(encoded).expect("base64"),
        content.as_bytes()
    );
    for (path, disposition) in [
        (format!("/raw/{id}"), "inline"),
        (format!("/download/{id}"), "attachment"),
    ] {
        let response = harness.get(&path).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            response.headers()["content-disposition"],
            format!("{disposition}; filename=\"{id}.rs\"")
        );
        assert!(
            response.headers()["content-security-policy"]
                .to_str()
                .expect("CSP")
                .contains("sandbox")
        );
        assert_eq!(text(response).await, content);
    }
}

#[tokio::test]
async fn downloads_use_the_resolved_language_extension_and_keep_source_bytes() {
    let harness = Harness::new(1024 * 1024).await;
    for (language, content, extension) in [
        ("md", "# 文档\r\n\r\n**内容**\r\n", "md"),
        ("html", "<!doctype html>\n<script>alert(1)</script>", "html"),
        ("auto", "<!doctype html><html>Auto</html>", "html"),
        ("python", "print('hello')\n", "py"),
        ("text", "plain\n", "txt"),
    ] {
        let id = harness
            .create(&[("content", content), ("language", language)])
            .await;
        let view = text(harness.get(&format!("/{id}")).await).await;
        assert!(view.contains(&format!("data-filename=\"{id}.{extension}\"")));
        for (path, disposition) in [("download", "attachment"), ("raw", "inline")] {
            let response = harness.get(&format!("/{path}/{id}")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["content-disposition"],
                format!("{disposition}; filename=\"{id}.{extension}\"")
            );
            assert_eq!(
                response.headers()["content-type"],
                "text/plain; charset=utf-8"
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(text(response).await, content);
        }
    }
}

#[tokio::test]
async fn markdown_defaults_to_rendered_with_highlighted_source_and_safe_html() {
    let harness = Harness::new(1024 * 1024).await;
    let content = "# Hello\n\n~~gone~~\n\n```rs\nfn main() {}\n```\n\n<script>alert('bad')</script>\n\n[bad](javascript:alert%281%29)";
    let id = harness
        .create(&[("content", content), ("expires_in", "never")])
        .await;
    let stored = harness.paste(&id).await;
    assert_eq!(stored.language, "markdown");
    assert_eq!(stored.expires_at, None);
    let html = text(harness.get(&format!("/{id}")).await).await;
    assert!(html.contains("<h1>Hello</h1>"));
    assert!(html.contains("<del>gone</del>"));
    assert!(html.contains("aria-label=\"markdown\" hidden"));
    assert!(html.contains("id=\"view-toggle\""));
    assert!(html.contains("syn-storage"));
    assert!(!html.contains("<script>alert('bad')</script>"));
    assert!(!html.contains("href=\"javascript:"));
    assert_eq!(
        text(harness.get(&format!("/raw/{id}")).await).await,
        content
    );
}

#[tokio::test]
async fn fork_prefills_a_form_without_writing_until_create() {
    let harness = Harness::new(1024).await;
    let content = "\n</textarea><script>alert(1)</script>";
    let original = harness
        .create(&[
            ("content", content),
            ("title", "Original"),
            ("language", "text"),
            ("expires_in", "never"),
        ])
        .await;
    let html = text(harness.get(&format!("/fork/{original}")).await).await;
    assert!(html.contains("value=\"Original\""));
    assert!(html.contains("value=\"text\" selected"));
    assert!(html.contains("value=\"1d\" selected"));
    assert!(html.contains("\n\n&#60;/textarea&#62;") || html.contains("\n\n&lt;/textarea&gt;"));
    assert!(!html.contains("</textarea><script>"));
    assert_eq!(harness.count().await, 1);
    let fork = harness
        .create(&[
            ("content", "edited"),
            ("title", "Original"),
            ("language", "text"),
        ])
        .await;
    assert_ne!(original, fork);
    assert_eq!(harness.count().await, 2);
    assert_eq!(harness.paste(&original).await.content, content);
    let fork = harness.paste(&fork).await;
    assert_eq!(fork.content, "edited");
    assert_eq!(fork.expires_at, Some(fork.created_at + 86400));
}

#[tokio::test]
async fn invalid_forms_fail_without_creating_records() {
    let harness = Harness::new(1024).await;
    for fields in [
        vec![("content", "")],
        vec![("title", "missing content")],
        vec![("content", "hello"), ("language", "not-a-language")],
        vec![("content", "hello"), ("expires_in", "1y")],
        vec![("content", "hello"), ("expires_in", "")],
        vec![("content", "hello"), ("content", "duplicate")],
        vec![("content", "hello"), ("unknown", "rejected")],
        vec![("content", "hello"), ("burn_after_read", "invalid")],
    ] {
        assert_error(
            harness.post(&fields).await,
            StatusCode::BAD_REQUEST,
            "bad_request",
        )
        .await;
    }
    assert_error(
        harness
            .post(&[("content", "hello"), ("title", &"中".repeat(201))])
            .await,
        StatusCode::BAD_REQUEST,
        "bad_request",
    )
    .await;
    assert_eq!(harness.count().await, 0);
}

#[tokio::test]
async fn content_and_transport_limits_return_structured_413() {
    let limit = 1024 * 1024;
    let harness = Harness::new(limit).await;
    let content = format!("{}{}", "中".repeat(limit / 3), "a".repeat(limit % 3));
    assert_eq!(content.len(), limit);
    let id = harness
        .create(&[("content", &content), ("language", "text")])
        .await;
    assert_eq!(harness.paste(&id).await.content, content);
    assert_error(
        harness.post(&[("content", &format!("{content}x"))]).await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
    )
    .await;
    assert_error(
        harness
            .request(
                Method::POST,
                "/paste",
                "x".repeat(limit * 3 + 16 * 1024 + 1),
            )
            .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
    )
    .await;
    assert_eq!(harness.count().await, 1);
}

#[tokio::test]
async fn every_read_path_checks_expiry_password_and_burn_confirmation() {
    let harness = Harness::new(1024).await;
    for (id, password, burn, expires, status, code) in [
        (
            "Expired1",
            None,
            false,
            Some(101),
            StatusCode::NOT_FOUND,
            "not_found",
        ),
        (
            "Locked01",
            Some("argon2-placeholder"),
            false,
            None,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (
            "Burn0001",
            None,
            true,
            None,
            StatusCode::BAD_REQUEST,
            "bad_request",
        ),
        (
            "Both0001",
            Some("argon2-placeholder"),
            true,
            None,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (
            "BothOld1",
            Some("argon2-placeholder"),
            true,
            Some(101),
            StatusCode::NOT_FOUND,
            "not_found",
        ),
    ] {
        db::insert_paste(
            &harness.pool,
            &Paste {
                id: id.into(),
                title: Some("secret title".into()),
                content: "secret body".into(),
                language: "text".into(),
                password_hash: password.map(str::to_owned),
                burn_after_read: burn,
                created_at: 100,
                expires_at: expires,
            },
        )
        .await
        .expect("fixture");
        for path in [
            format!("/{id}"),
            format!("/raw/{id}"),
            format!("/download/{id}"),
            format!("/fork/{id}"),
        ] {
            assert_error(harness.get(&path).await, status, code).await;
        }
    }
    assert_eq!(harness.count().await, 3);
}

#[tokio::test]
async fn missing_and_invalid_ids_are_404_on_every_read_path() {
    let harness = Harness::new(1024).await;
    for id in ["Absent01", "short", "123456789", "abcd-123", "%FF"] {
        for prefix in ["", "/raw", "/download", "/fork"] {
            assert_error(
                harness.get(&format!("{prefix}/{id}")).await,
                StatusCode::NOT_FOUND,
                "not_found",
            )
            .await;
        }
    }
}

#[tokio::test]
async fn live_queries_exclude_exact_expiration_boundary() {
    let harness = Harness::new(1024).await;
    let id = harness.create(&[("content", "hello")]).await;
    let paste = harness.paste(&id).await;
    let expires_at = paste.expires_at.expect("default expiration");
    assert!(
        db::find_live_paste(&harness.pool, &id, expires_at - 1)
            .await
            .expect("query")
            .is_some()
    );
    assert!(
        db::find_live_paste(&harness.pool, &id, expires_at)
            .await
            .expect("query")
            .is_none()
    );
    assert!(
        db::find_live_paste(&harness.pool, &id, expires_at + 1)
            .await
            .expect("query")
            .is_none()
    );
}

#[tokio::test]
async fn form_and_embedded_assets_are_available_without_a_frontend_build() {
    let harness = Harness::new(1024).await;
    let html = text(harness.get("/").await).await;
    assert!(html.matches("<option value=").count() > 100);
    assert!(html.contains("action=\"/paste\" method=\"post\""));
    for (path, content_type) in [
        ("/static/app.css", "text/css; charset=utf-8"),
        ("/static/syntax.css", "text/css; charset=utf-8"),
        ("/static/app.js", "text/javascript; charset=utf-8"),
        ("/static/editor.js", "text/javascript; charset=utf-8"),
    ] {
        let response = harness.get(path).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], content_type);
        assert!(!text(response).await.is_empty());
    }
    assert_error(
        harness.get("/api/paste/Absent01").await,
        StatusCode::NOT_FOUND,
        "not_found",
    )
    .await;
}

#[tokio::test]
async fn unsupported_methods_and_media_types_are_structured_errors() {
    let harness = Harness::new(1024).await;
    assert_error(
        harness.get("/paste").await,
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    )
    .await;
    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/paste")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"hello"}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_error(response, StatusCode::BAD_REQUEST, "bad_request").await;
    assert_eq!(harness.count().await, 0);
}
