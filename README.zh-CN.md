# Pastelet

[English](README.md) | [简体中文](README.zh-CN.md)

**一个自托管、仅凭链接访问的 Paste 服务，整个应用只有一个 Rust 二进制文件。**

## 项目简介

Pastelet 把文本、代码、Markdown 和 PDF 附件保存在 SQLite 中，并由一个小巧的二进制文件对外提供服务——不需要外部数据库、对象存储、CDN 或 Node 运行时。分享方式只有链接：没有账号、没有公开列表、也没有搜索。每个 paste 都可以选择设置密码、过期时间，或开启阅后即焚。

它适合家庭实验室、Tailscale 网络或小型团队服务器：一个进程、一个 SQLite 文件，再加一个可选的只读容器，就是全部部署成本。

## 核心特色

### 仅凭链接分享
- 通过网页或 API 创建 paste 并拿到链接。无需注册，没有控制台，没有列表接口，所有响应都带有 `X-Robots-Tag: noindex`。
- 新 ID 是 22 位 Base62 字符串，由操作系统安全随机源生成（约 131 bit 熵）。旧的 8 位链接继续有效，可正常读取。

### 保护与生命周期
- **密码保护**——使用带独立盐值的 Argon2id 哈希。解锁 Cookie 为 `HttpOnly`、`SameSite=Strict`，按单个 paste 和访问主机隔离，最长 1 小时且不超过 paste 自身的过期时间。
- **阅后即焚**——浏览器先看到确认页；服务端用一条带条件的 `DELETE` 决出唯一成功者，因此并发读取最多只有一个请求能拿到内容。
- **过期清理**——支持 `10m`、`1h`、`1d`、`1w`、`1m`（30 天）或 `never`。每次访问都会检查是否过期，后台任务定期删除过期记录。
- **来源白名单**——解锁、焚毁确认、fork 提交和 PDF 上传会校验 `BASE_URL` 以及 `ALLOWED_ORIGINS` 中列出的来源。

### 舒适的阅读体验
- 基于 syntect + two-face 的 100+ 语言语法高亮，带行号和 `#L12` 行锚点。
- Markdown 支持表格、任务列表、删除线、脚注和 GFM，并经 ammonia 清洗；围栏代码块同样高亮。
- 可复制原文、查看 raw、按语言扩展名下载，或 fork 到新的创建页。
- 支持最大 10 MiB 的 PDF 附件，使用自托管的 PDF.js 阅读；密码、过期和阅后即焚同样适用于 PDF。
- 文本文件拖放导入和 PDF 上传都在浏览器端处理，文本内容不需要额外上传接口。
- 提供浅色、深色和跟随系统三种主题。创建、raw、下载和 fork 等基本流程在无 JavaScript 时仍可用；复制和 Markdown 源码切换需要 JavaScript。

### 易运维，也更安全
- 模板、CSS、JavaScript、语法定义、PDF.js 和数据库迁移全部嵌入同一个二进制文件。
- SQLite 使用 WAL 模式，迁移由 SQLx 内嵌并在启动时执行，无需安装数据库服务。
- Tailwind CSS 由固定版本的独立 CLI 预先生成并提交到仓库，构建时只做一致性检查；HTMX 2.0.8 与 PDF.js 6.5.438 自托管，运行时完全不依赖 CDN。
- 容器默认以 UID/GID 10001 运行，根文件系统只读，移除全部 capabilities，启用 `no-new-privileges`，`/tmp` 使用临时文件系统。
- 仓库内的 GitHub Actions 工作流会测试两种架构，并把 `linux/amd64` 与 `linux/arm64` 镜像发布到 GHCR。

## 快速开始

### 环境要求
- Rust 1.99.1 与 Cargo（edition 2024）。
- 用于编译 Oniguruma 依赖的 C 编译器与链接器（Debian/Ubuntu 上安装 `build-essential` 和 `pkg-config`）。
- 首次构建时需要联网下载 crates。

不需要：数据库服务、`sqlx-cli`、Node.js，或预先创建数据库文件。

可选工具：
- Docker Engine 与 Compose 插件，用于容器部署。
- Node.js 24，用于 DOM 级 UI 测试。
- 仅在重新生成 CSS 时需要 `curl`、`sh` 以及 `sha256sum`/`shasum`。

### 安装

#### 从源码运行

```sh
git clone https://github.com/atsud0/pastelet.git
cd pastelet
cargo run --locked
```

打开 <http://localhost:3000>。数据库（`data/pastelet.db`）及其父目录会自动创建，迁移在启动时执行。按 Ctrl-C 停止服务。

应用直接读取进程环境变量，不会自动加载 `.env`。如需使用仓库提供的示例配置：

```sh
cp .env.example .env
set -a
. ./.env
set +a
cargo run --locked
```

#### 使用 Docker Compose 运行

```sh
cp .env.example .env
docker compose up -d --build
docker compose ps
```

服务默认监听 `127.0.0.1:3000`，数据保存在名为 `pastelet-data` 的命名卷中。可在 `.env` 中设置 `HOST_IP` 和 `HOST_PORT` 改变绑定，例如 `HOST_PORT=3001`。

推送默认分支或 `v*` 标签后，仓库内的工作流可以把多架构镜像发布到 `ghcr.io/<owner>/<repo>`。镜像发布后，可让 Compose 直接拉取：

```dotenv
PASTELET_IMAGE=ghcr.io/your-owner/pastelet:v1.0.0
```

```sh
docker compose pull pastelet
docker compose up -d --no-build
```

#### 配置项

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `BIND_ADDR` | `127.0.0.1:3000` | 监听地址。容器内使用 `0.0.0.0:3000`。 |
| `DATABASE_URL` | `sqlite://data/pastelet.db` | SQLite URL。缺失的父目录会自动创建。 |
| `BASE_URL` | `http://localhost:3000` | 生成分享链接所用的 HTTP(S) origin，始终加入来源白名单。不能包含路径、凭据、query 或 fragment。 |
| `ALLOWED_ORIGINS` | 空 | 额外允许的来源，以逗号分隔，用于解锁、焚毁确认、fork 提交和 PDF 上传；须包含完整协议、主机和端口，不支持通配符。 |
| `MAX_CONTENT_BYTES` | `1048576` | 网页与 API 创建的 UTF-8 内容大小上限。 |
| `DATABASE_MAX_CONNECTIONS` | `5` | SQLite 连接池上限。 |
| `CLEANUP_INTERVAL_SECONDS` | `60` | 清理过期 paste 和凭证的间隔。 |
| `API_TOKEN` | 未设置 | 设置后，API 创建必须携带 `Authorization: Bearer <token>`；空值视为未启用。 |
| `RUST_LOG` | `pastelet=info,tower_http=info` | tracing 日志过滤器。 |

API 返回的分享链接使用 `BASE_URL` 生成，不依赖请求的 `Host` 头。修改环境变量后需要重新创建容器，单独执行 `docker restart` 不会生效。

### 使用示例

#### 网页界面

打开首页，粘贴或拖入内容，选择语言和过期时间，也可以设置密码或开启 **Burn**。生成的链接是访问该 paste 的唯一入口。`/raw/<id>` 返回纯文本，`/download/<id>` 下载原文，`/fork/<id>` 打开预填内容的创建页。

#### 通过 API 创建 paste

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

使用 `text/plain` 时，请求体就是内容，其余参数放在 query 中：

```sh
printf 'hello\n' | curl -sS -X POST \
  'http://localhost:3000/api/paste?language=txt&expires_in=1h' \
  -H 'Content-Type: text/plain' --data-binary @-
```

#### 读取 paste

```sh
curl -sS http://localhost:3000/api/paste/Ab12Cd34Ef56Gh78Ij90Kl
```

响应包含 `id`、`title`、`content`、`language`、`created_at`、`expires_at` 和 `burn_after_read`。带密码的 paste 需要通过请求头提供密码：

```sh
curl -sS -H 'X-Paste-Password: demo-only' \
  http://localhost:3000/api/paste/Ab12Cd34Ef56Gh78Ij90Kl
```

阅后即焚的 paste 会在首次成功读取时删除，请不要自动重试。`HEAD` 请求返回 `405`，且永远不会焚毁内容。

#### 用令牌保护 API 写入

```sh
API_TOKEN='replace-with-a-long-random-token' cargo run --locked

curl -sS -X POST http://localhost:3000/api/paste \
  -H 'Authorization: Bearer replace-with-a-long-random-token' \
  -H 'Content-Type: text/plain' --data-binary 'hello'
```

`API_TOKEN` 只限制 API 创建，不会保护网页表单、PDF 上传或读取操作。读取仍然依赖链接本身以及 paste 自己的密码。

#### 上传 PDF

PDF 通过网页表单接口上传，而不是 JSON API：

```sh
curl -sS -X POST http://localhost:3000/paste/pdf \
  -F 'file=@document.pdf' \
  -F 'title=Document' \
  -F 'expires_in=1w'
```

上传成功返回 `201`，响应体为 `{"id":"...","url":"/..."}`。

#### 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `content` | 字符串，必填 | 非空 UTF-8，最多 `MAX_CONTENT_BYTES`；保留空白和换行。 |
| `title` | 字符串，可选 | 最多 200 个字符，首尾空白会被去除。 |
| `language` | 字符串，可选 | `auto`、`text`、`markdown`，或 `rs`、`py`、`js`、`ts` 等语言名/别名。手动填写未知值返回 `400`。 |
| `expires_in` | 字符串，可选 | `10m`、`1h`、`1d`、`1w`、`1m`（30 天）或 `never`，默认 `1d`。 |
| `password` | 字符串，可选 | 最多 1024 字节，保存为 Argon2id 哈希；空值表示不设密码。 |
| `burn_after_read` | 布尔值，可选 | 默认 `false`；JSON 模式必须使用真正的布尔值。 |

API 错误统一为 JSON：

| HTTP | `code` | 含义 |
| --- | --- | --- |
| `400` | `bad_request` | 字段无效或缺失、Content-Type 不支持、过期参数无效。 |
| `401` | `unauthorized` | API 令牌缺失或错误，或 paste 密码缺失/错误。 |
| `404` | `not_found` | paste 不存在、已过期或已焚毁。 |
| `413` | `payload_too_large` | 内容或请求体超过配置上限。 |
| `405` | `method_not_allowed` | 方法不支持；`HEAD` 不返回响应体。 |
| `500` | `internal_error` | 内部故障，响应不包含数据库或密码细节。 |

### 部署注意事项

- **HTTPS / 反向代理**——由 Caddy、Nginx 等终止 TLS 后转发到 `127.0.0.1:3000`，并设置 `BASE_URL=https://paste.example.com`。不要删除或改写 `Origin` / `Referer`，浏览器的来源校验依赖它们。应用不会根据 `X-Forwarded-Host` 推断 origin。
- **多个访问入口**——把其他来源全部写入 `ALLOWED_ORIGINS`，包括协议和非默认端口。`http://node:8007` 与 `https://node:8007` 是不同来源，短主机名、完整域名和 IP 地址也各自不同。不支持通配符。
- **备份**——SQLite 使用 WAL 模式，请先停止服务，再打包整个数据目录（容器内为 `/data`），不要只复制 `pastelet.db`。备份中包含 paste 明文和密码哈希，应妥善保护。
- **升级**——迁移在启动时自动执行；迁移失败时服务不会启动。迁移 `0004` 会重建表以支持 22 位 ID，同时保留已有的 8 位链接；升级前请备份数据目录。
- **横向扩展**——只运行一个实例并挂载本地卷。不要多个容器共享同一个 SQLite 数据库，也不要把它放在网络文件系统上。

## 项目结构

```text
.
├── src/
│   ├── main.rs, lib.rs         # 启动、共享状态、优雅退出
│   ├── routes.rs               # 路由、请求体上限、响应头
│   ├── config.rs               # 环境变量解析与校验
│   ├── db.rs                   # SQLite 连接池、内嵌迁移、查询
│   ├── api.rs                  # JSON / text-plain REST API
│   ├── error.rs, models.rs     # 错误映射与数据类型
│   ├── handlers/               # 网页、静态资源、paste 与 PDF 处理器
│   └── services/               # 渲染、语言、密码、访问控制、PDF、清理任务
├── migrations/                 # SQLx 迁移 0001–0004
├── templates/                  # Askama HTML 模板
├── static/                     # CSS、JS、自托管 HTMX 与 PDF.js（嵌入二进制）
├── tests/                      # Rust 集成测试与 Node UI 测试
├── scripts/                    # CSS 构建与容器冒烟测试
├── .github/workflows/ci.yml    # 检查、多架构构建、GHCR 发布
├── Dockerfile
├── docker-compose.yml
├── .env.example
└── Cargo.toml / Cargo.lock
```

## 安全模型与限制

- 密码只控制访问，不加密静态内容。任何拿到 SQLite 文件或备份的人都能读取内容。
- 阅后即焚保证服务端在向最多一个请求返回内容前完成删除，但无法保证客户端真的收到或显示了内容；它也不是对 WAL、空闲页、快照或备份的安全擦除。
- `API_TOKEN` 只保护 API 写入。网页表单和 PDF 上传仍然开放，除非你在网络或代理层限制访问。应用没有内置限流。
- 所有响应都设置 `Cache-Control: no-store`、`X-Content-Type-Options: nosniff`、`Referrer-Policy: same-origin`、`X-Robots-Tag: noindex, nofollow, noarchive` 以及严格的 `Content-Security-Policy`。渲染后的 Markdown 经过清洗，raw 响应使用 sandbox CSP。
- 不存在、已过期和已焚毁的 paste 都返回相同的 `404`，不会透露链接是否曾经存在。

## 贡献指南与许可证

### 贡献指南

欢迎提交 Issue 和 Pull Request。提交改动前，请运行与 CI 相同的检查：

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test tests/*_ui.cjs        # 可选，需要 Node.js 24
sh scripts/build-css.sh --check
docker compose config --quiet
```

请保持改动聚焦，为行为变化补充测试，并在用户可见行为变化时同步更新 README。`static/app.css` 由 `static/styles.css` 和模板生成——请修改源文件并运行 `sh scripts/build-css.sh`，不要直接编辑生成文件。

如果你在尚未修复的部署中发现安全问题，请通过 GitHub 联系维护者，而不要直接公开详细 Issue。

### 许可证

Pastelet 使用 **GNU Affero General Public License v3.0** 发布，完整条款见 [LICENSE](LICENSE)。

AGPL 要求：如果你修改了 Pastelet 并通过网络向用户提供服务，需要向这些用户提供对应的源代码。
