# Pastelet

[English](README.md) | [简体中文](README.zh-CN.md)

**A self-hosted, link-only paste service that ships as a single Rust binary.**

## What it does

Pastelet stores text, code, Markdown, and PDF attachments in SQLite and serves them from one small binary — no external database, object storage, CDN, or Node runtime. Sharing is link-only by design: there are no accounts, no paste directory, and no search. Every paste can be password-protected, given an expiry time, or set to burn after its first read.

It is built for a homelab, a Tailscale network, or a small team server: one process, one SQLite file, and an optional read-only container are the whole deployment.

## Why it is useful

### Link-only sharing
- Create a paste from the web page or the API and get a URL. No sign-up, no dashboard, no listing endpoint, and `X-Robots-Tag: noindex` on every response.
- New IDs are 22-character Base62 strings generated from the OS secure random source (about 131 bits of entropy). Legacy 8-character links remain valid and readable.

### Protection and lifecycle
- **Password protection** — Argon2id hashes with per-paste salts. Unlock cookies are `HttpOnly`, `SameSite=Strict`, scoped to one paste and one host, and expire within an hour or at the paste's own expiry.
- **Burn after read** — browsers see a confirmation page first; a single conditional `DELETE` picks the winner, so concurrent reads produce at most one successful response.
- **Expiring pastes** — `10m`, `1h`, `1d`, `1w`, `1m` (30 days), or `never`. Expiry is checked on every access and expired rows are removed by a background task.
- **Origin allowlist** — unlock, burn confirmation, fork submission, and PDF upload are checked against `BASE_URL` plus any origins listed in `ALLOWED_ORIGINS`.

### A pleasant reading experience
- Syntax highlighting for 100+ languages (syntect + two-face), with line numbers and `#L12` anchors.
- Markdown rendering with tables, task lists, strikethrough, footnotes, and GFM, sanitized with ammonia. Fenced code blocks are highlighted.
- Copy the original text, open the raw form, download with a language-aware file extension, or fork the content into a new paste.
- PDF attachments up to 10 MiB, viewed with a self-hosted PDF.js build. Passwords, expiry, and burn-after-read apply to PDFs as well.
- Drag-and-drop text file import and PDF upload are handled in the browser; text files never need a separate upload endpoint.
- Light, dark, and system themes. Basic create, raw, download, and fork flows work without JavaScript; copying and the Markdown source toggle require it.

### Simple and safe to operate
- One binary containing templates, CSS, JavaScript, syntax definitions, PDF.js, and migrations.
- SQLite in WAL mode with embedded SQLx migrations — no database service to install.
- Tailwind CSS is generated ahead of time with a pinned standalone CLI and committed; HTMX 2.0.8 and PDF.js 6.5.438 are self-hosted. Nothing is loaded from a CDN at runtime.
- Container-friendly defaults: UID/GID 10001, read-only root filesystem, dropped capabilities, `no-new-privileges`, and a temporary `/tmp`.
- The included GitHub Actions workflow tests both architectures and publishes `linux/amd64` and `linux/arm64` images to GHCR.

## Quick start

### Requirements
- Rust 1.99.1 and Cargo (edition 2024).
- A C compiler and linker for the Oniguruma dependency (`build-essential` and `pkg-config` on Debian/Ubuntu).
- Network access on the first build to fetch crates.

Not required: a database server, `sqlx-cli`, Node.js, or a pre-created database file.

Optional tooling:
- Docker Engine with the Compose plugin for container deployment.
- Node.js 24 for the DOM-level UI tests.
- `curl`, `sh`, and `sha256sum`/`shasum` only when regenerating CSS.

### Installation

#### Run from source

```sh
git clone https://github.com/atsud0/pastelet.git
cd pastelet
cargo run --locked
```

Open <http://localhost:3000>. The database (`data/pastelet.db`) and its parent directory are created automatically, and migrations run at startup. Stop the server with Ctrl-C.

The process reads environment variables directly and does not load `.env` by itself. To use the provided example:

```sh
cp .env.example .env
set -a
. ./.env
set +a
cargo run --locked
```

#### Run with Docker Compose

```sh
cp .env.example .env
docker compose up -d --build
docker compose ps
```

The service listens on `127.0.0.1:3000` and stores data in the `pastelet-data` named volume. Set `HOST_IP` and `HOST_PORT` in `.env` to change the binding, for example `HOST_PORT=3001`.

The workflow can publish multi-architecture images to `ghcr.io/<owner>/<repo>` after you push to the default branch or a `v*` tag. Once such an image exists, point Compose at it:

```dotenv
PASTELET_IMAGE=ghcr.io/your-owner/pastelet:v1.0.0
```

```sh
docker compose pull pastelet
docker compose up -d --no-build
```

#### Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `BIND_ADDR` | `127.0.0.1:3000` | Listen address. Use `0.0.0.0:3000` inside a container. |
| `DATABASE_URL` | `sqlite://data/pastelet.db` | SQLite URL. Missing parent directories are created. |
| `BASE_URL` | `http://localhost:3000` | Canonical HTTP(S) origin used for generated share links. Always allowed for origin checks. No path, credentials, query, or fragment. |
| `ALLOWED_ORIGINS` | empty | Comma-separated extra origins for unlock, burn confirmation, fork submission, and PDF upload. Exact scheme, host, and port; no wildcards. |
| `MAX_CONTENT_BYTES` | `1048576` | Maximum UTF-8 content size for web and API creation. |
| `DATABASE_MAX_CONNECTIONS` | `5` | SQLite connection-pool size. |
| `CLEANUP_INTERVAL_SECONDS` | `60` | Interval for removing expired pastes and tokens. |
| `API_TOKEN` | unset | When set, API creation requires `Authorization: Bearer <token>`. Empty means disabled. |
| `RUST_LOG` | `pastelet=info,tower_http=info` | Tracing filter. |

Share links returned by the API use `BASE_URL`, not the request's `Host` header. Changing environment variables requires recreating the container; `docker restart` alone does not apply them.

### Usage examples

#### Web interface

Open the home page, paste or drop content, choose a language and expiry, and optionally set a password or enable **Burn**. The resulting link is the only way to reach the paste. `/raw/<id>` returns plain text, `/download/<id>` downloads it, and `/fork/<id>` opens a prefilled create page.

#### Create a paste with the API

```sh
curl -sS -X POST http://localhost:3000/api/paste \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "content": "fn main() { println!(\"hello\"); }\n",
    "title": "Hello",
    "language": "rs",
    "expires_in": "1d"
  }'
```

```json
{
  "id": "Ab12Cd34Ef56Gh78Ij90Kl",
  "url": "http://localhost:3000/Ab12Cd34Ef56Gh78Ij90Kl",
  "raw_url": "http://localhost:3000/raw/Ab12Cd34Ef56Gh78Ij90Kl",
  "expires_at": 1794096000
}
```

Plain-text input uses the request body as content and query parameters for everything else:

```sh
printf 'hello\n' | curl -sS -X POST \
  'http://localhost:3000/api/paste?language=txt&expires_in=1h' \
  -H 'Content-Type: text/plain' --data-binary @-
```

#### Read a paste

```sh
curl -sS http://localhost:3000/api/paste/Ab12Cd34Ef56Gh78Ij90Kl
```

The response contains `id`, `title`, `content`, `language`, `created_at`, `expires_at`, and `burn_after_read`. Password-protected pastes require the password in a header:

```sh
curl -sS -H 'X-Paste-Password: demo-only' \
  http://localhost:3000/api/paste/Ab12Cd34Ef56Gh78Ij90Kl
```

Burned pastes are deleted on the first successful API read; do not retry automatically. `HEAD` returns `405` and never burns content.

#### Protect API writes with a token

```sh
API_TOKEN='replace-with-a-long-random-token' cargo run --locked

curl -sS -X POST http://localhost:3000/api/paste \
  -H 'Authorization: Bearer replace-with-a-long-random-token' \
  -H 'Content-Type: text/plain' --data-binary 'hello'
```

`API_TOKEN` restricts API creation only. It does not protect the web form, PDF upload, or reads. Reads rely on the link and the paste's own password.

#### Upload a PDF

PDFs are uploaded through the web form endpoint, not the JSON API:

```sh
curl -sS -X POST http://localhost:3000/paste/pdf \
  -F 'file=@document.pdf' \
  -F 'title=Document' \
  -F 'expires_in=1w'
```

A successful upload returns `201` with `{"id":"...","url":"/..."}`.

#### Field reference

| Field | Type | Notes |
| --- | --- | --- |
| `content` | string, required | Non-empty UTF-8; at most `MAX_CONTENT_BYTES`. Whitespace and line breaks are preserved. |
| `title` | string, optional | Up to 200 characters; surrounding whitespace is trimmed. |
| `language` | string, optional | `auto`, `text`, `markdown`, or a language name/alias such as `rs`, `py`, `js`, `ts`. Unknown manual values return `400`. |
| `expires_in` | string, optional | `10m`, `1h`, `1d`, `1w`, `1m` (30 days), or `never`. Defaults to `1d`. |
| `password` | string, optional | Up to 1024 bytes. Stored as an Argon2id hash; an empty value means no password. |
| `burn_after_read` | boolean, optional | Defaults to `false`. JSON mode requires a real boolean. |

API errors are JSON:

| HTTP | `code` | Meaning |
| --- | --- | --- |
| `400` | `bad_request` | Invalid or missing fields, unsupported content type, invalid expiration. |
| `401` | `unauthorized` | Missing or wrong API token, or missing/wrong paste password. |
| `404` | `not_found` | Unknown, expired, or already burned paste. |
| `413` | `payload_too_large` | Content or request body exceeds the configured limit. |
| `405` | `method_not_allowed` | Unsupported method; `HEAD` returns no body. |
| `500` | `internal_error` | Internal failure without database or password details. |

### Deployment notes

- **HTTPS / reverse proxy** — terminate TLS at Caddy, Nginx, or another proxy and forward to `127.0.0.1:3000`. Set `BASE_URL=https://paste.example.com`. Do not strip or rewrite `Origin` or `Referer`; the browser origin checks rely on them. The application does not derive its origin from `X-Forwarded-Host`.
- **Multiple entry points** — list every additional origin in `ALLOWED_ORIGINS`, including scheme and non-default port. `http://node:8007` and `https://node:8007` are different origins, and so are short hostnames, FQDNs, and IP addresses. There is no wildcard support.
- **Backups** — SQLite runs in WAL mode, so stop the service and archive the whole data directory (`/data` in the container), not just `pastelet.db`. Backups contain paste content and password hashes; protect them accordingly.
- **Upgrades** — migrations run automatically at startup, and a failed migration prevents the server from starting. Migration `0004` rebuilds the tables to allow 22-character IDs while preserving existing 8-character links; back up the data directory before upgrading.
- **Scaling** — run a single instance against a local volume. Do not share one SQLite database between containers or place it on a network filesystem.

## Project structure

```text
.
├── src/
│   ├── main.rs, lib.rs         # startup, shared state, graceful shutdown
│   ├── routes.rs               # routes, body limits, response headers
│   ├── config.rs               # environment parsing and validation
│   ├── db.rs                   # SQLite pool, embedded migrations, queries
│   ├── api.rs                  # JSON / text-plain REST API
│   ├── error.rs, models.rs     # error mapping and data types
│   ├── handlers/               # web pages, assets, paste and PDF handlers
│   └── services/               # rendering, language, password, access, PDF, cleanup
├── migrations/                 # SQLx migrations 0001–0004
├── templates/                  # Askama HTML templates
├── static/                     # CSS, JS, vendored HTMX and PDF.js (embedded)
├── tests/                      # Rust integration tests and Node UI tests
├── scripts/                    # CSS build and container smoke test
├── .github/workflows/ci.yml    # checks, multi-arch builds, GHCR release
├── Dockerfile
├── docker-compose.yml
├── .env.example
└── Cargo.toml / Cargo.lock
```

## Security model and limits

- Passwords guard access; they do not encrypt content at rest. Anyone with the SQLite file or a backup can read it.
- Burn-after-read guarantees that the server deletes the paste before returning it to at most one request. It cannot guarantee that the client received or displayed it, and it is not a secure erase of WAL files, free pages, snapshots, or backups.
- `API_TOKEN` protects API writes only. The web form and PDF upload remain open unless you restrict access at the network or proxy layer. There is no built-in rate limiting.
- Every response sets `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin`, `X-Robots-Tag: noindex, nofollow, noarchive`, and a restrictive `Content-Security-Policy`. Rendered Markdown is sanitized, and raw responses use a sandbox CSP.
- Not found, expired, and burned pastes all return the same `404`, so the service does not reveal whether a link ever existed.

## Contributing & License

### Contributing

Issues and pull requests are welcome. Before submitting a change, run the same checks as CI:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test tests/*_ui.cjs        # optional, requires Node.js 24
sh scripts/build-css.sh --check
docker compose config --quiet
```

Please keep changes focused, add tests for behavior changes, and update the README when user-visible behavior changes. The CSS in `static/app.css` is generated from `static/styles.css` and the templates — edit the source and run `sh scripts/build-css.sh` instead of editing generated files.

If you find a security issue in a deployment that is not yet fixed, please contact the maintainer through GitHub rather than opening a detailed public issue.

### License

Pastelet is released under the **GNU Affero General Public License v3.0**. See [LICENSE](LICENSE) for the full text.

The AGPL requires that if you modify Pastelet and make it available to users over a network, you offer them the corresponding source code.
