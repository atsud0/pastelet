# rpaste

无账号、无公开列表、无搜索的 Rust 文本分享站。六个阶段已实现：完整网页交互、密码保护、阅后即焚、过期清理、REST API、Docker 部署，以及 GitHub Actions 多架构镜像发布。

## 当前阶段

- Axum / Tokio 启动、tracing 日志、Ctrl-C / SIGTERM 优雅退出。
- 环境变量配置与启动校验。
- SQLite / SQLx 连接池、WAL、5 秒锁等待、嵌入式迁移。
- Paste 数据模型、创建与读取；访问所有内容入口时检查过期。
- thiserror 错误类型、统一 JSON 错误响应、内部错误脱敏。
- syntect + two-face 扩展语法库，提供 100+ 语言选项、行号和 `#L12` 行锚点。
- pulldown-cmark 渲染 Markdown（表格、任务列表、删除线、脚注），围栏代码使用 syntect；完整 HTML 再经 ammonia 清洗。
- 可操作的 Askama 创建页与查看页；Markdown 默认渲染，可切换源码；复制原文、raw、下载和 fork。
- Tailwind standalone 正式界面、HTMX 创建反馈、系统 / 亮色 / 暗色切换；所有资源嵌入二进制。
- Argon2id 密码哈希、按 paste 隔离的解锁凭证、POST 确认后原子阅后即焚。
- Tokio 定时过期清理，访问时也检查过期；解锁页、确认页和极简错误页。
- 配置、迁移、持久化、参数校验、随机 ID 冲突、Markdown 清洗、内容路由测试。

REST API 支持 JSON / text/plain 创建、密码读取和可选写入令牌；用法仅放在本 README，不增加网页 API 文档页。Docker / Compose 和 GitHub Actions 支持 linux/amd64、linux/arm64，部署与发布步骤见第 6 阶段。

## 运行

需要 Rust / Cargo（当前验证版本 Rust 1.99.1）、C 编译工具链及首次构建下载 crates 的网络。SQLite 和 Oniguruma 由依赖构建，不需要数据库服务、sqlx-cli、Node 或预先存在的数据库。应用的依赖版本由 `Cargo.lock` 固定。

```sh
cargo run --locked
```

打开 `http://localhost:3000`，输入内容后点击 Create。默认创建 `data/rpaste.db`，启动时自动执行迁移；迁移失败则启动失败。模板、迁移、基础 CSS/JS、语法库与主题均嵌入二进制，无需运行时静态资源目录。按 Ctrl-C 停止服务。

应用只读取进程环境，不自动加载 `.env`。可显式加载项目提供的可信示例：

```sh
cp .env.example .env
set -a
. ./.env
set +a
cargo run --locked
```

不要 source 不可信的配置文件。Compose 可以自动读取同目录 `.env`，无需 source；详见第 6 阶段。

## 配置

| 变量 | 默认值 | 含义 |
| --- | --- | --- |
| `BIND_ADDR` | `127.0.0.1:3000` | IP 与端口；容器或远程访问需显式设置 `0.0.0.0:3000` |
| `DATABASE_URL` | `sqlite://data/rpaste.db` | SQLite URL；自动创建缺失的父目录 |
| `BASE_URL` | `http://localhost:3000` | 对外 HTTP(S) origin；用于解锁/确认的来源校验和 Secure Cookie 判定，必须与浏览器访问地址一致；不允许路径、凭据、query、fragment |
| `MAX_CONTENT_BYTES` | `1048576` | UTF-8 原始内容字节上限，表单和 API 创建均实施 |
| `DATABASE_MAX_CONNECTIONS` | `5` | 连接池上限，正整数 |
| `CLEANUP_INTERVAL_SECONDS` | `60` | 定时清理间隔；启动后立即清理一次，随后定时清理 |
| `API_TOKEN` | 未设置 | 为空时视为未设置；设置后仅 API 创建必须携带 Bearer 令牌，网页创建不受影响 |
| `RUST_LOG` | `rpaste=info,tower_http=info` | tracing 过滤器 |

令牌只保存在内存配置中，不打印整个配置；不要提交 `.env`。应用使用 BASE_URL 生成 API 返回的分享链接，不依赖请求 Host。

## 数据模型

`migrations/0001_create_pastes.sql` 创建 STRICT 表 `pastes`：8 位 base62 主键、可空标题（最多 200 个字符）、原始内容、语言、可空密码哈希、阅后即焚标记、创建/过期时间。时间统一为 UTC Unix 秒，`expires_at = NULL` 表示永不过期；`expires_at <= now` 即过期。过期时间有部分索引，供后续清理使用。

ID 使用操作系统安全随机源和拒绝采样，生成无模偏差的 8 位 base62 字符串。插入时以主键唯一约束处理冲突，最多尝试 5 个 ID，绝不覆盖旧内容。`password_hash` 保存带独立随机盐的 Argon2id PHC 编码哈希，不存储明文密码。存储模型不实现 Serialize 或 Debug，避免直接序列化密码哈希和意外输出内容。内容大小由配置决定，因此不在迁移中写死 1 MiB。

`migrations/0002_paste_tokens.sql` 新增短期凭证表，仅存随机令牌的 SHA-256 摘要、paste ID、用途和到期时间。解锁凭证与确认凭证分开校验；删除 paste 时通过外键级联删除其凭证。升级会自动执行新迁移，无需删除原有数据库。

已应用的迁移不要修改；新增变更请添加新迁移。`build.rs` 确保迁移目录变化触发重新编译。

## 当前路由

| 方法 | 路径 | 行为 |
| --- | --- | --- |
| GET | `/` | 创建表单；没有列表或搜索 |
| POST | `/paste` | 接收 URL 编码表单，创建成功返回 `303`，跳转 `/{id}` |
| GET | `/{id}` | 高亮或 Markdown 查看页 |
| GET | `/raw/{id}` | 原始 UTF-8 内容，`text/plain` |
| GET | `/download/{id}` | 原始内容，附件扩展名跟随语言，如 `{id}.md`、`{id}.html`；纯文本或未知语言用 `.txt` |
| GET | `/fork/{id}` | 预填标题、内容与语言；不写数据库 |
| POST | `/unlock/{id}` | 校验密码，签发该 paste 的短期 Cookie，跳转原目标 |
| POST | `/read/{id}` | 校验确认令牌与访问权限，返回内容并原子烧毁 |
| POST | `/fork` | 接收已读内容快照并预填创建页；不写数据库 |
| GET | `/static/app.css`、`/static/syntax.css`、`/static/app.js`、`/static/htmx.min.js` | 二进制内嵌资源 |

创建表单接受 `content`、`title`、`language`、`expires_in`、`password`、`burn_after_read`，拒绝未知或重复字段。空密码视为不保护；非空密码按原始 UTF-8 字节精确校验，不裁剪空格，最多 1024 字节。`burn_after_read` 省略为 false，也可显式传 true / false。内容不可为空，标题可省略；标题首尾空白会去掉，最多 200 个 Unicode 字符。内容按 UTF-8 字节计量，超限返回 `413`。表单传输上限为 `3 × MAX_CONTENT_BYTES + 16 KiB`，允许百分号编码膨胀；解码后仍严格检查原始内容大小。

`expires_in` 支持 `10m / 1h / 1d / 1w / 1m / never`，省略时为 `1d`，其中 `1m` 固定为 30 天。fork 提交时分配新 ID，过期时间重新计算，默认 1 天，不继承原记录的安全或过期设置。

语言可以使用下拉选项名称或常用别名（如 `rs`、`py`、`js`、`ts`、`md`、`txt`）。持久化时统一为语法库名称，例如 `Rust`、`Python`；纯文本与 Markdown 分别使用 `text`、`markdown`。省略、空值或 `auto` 开启有限的内容启发式检测，优先处理 shebang 和 Markdown 围栏，识别常见代码特征，无法识别时使用纯文本。自动检测不是 100+ 语言分类器；不确定时请手动选择。未知的手动语言返回 `400`，Markdown 内未知围栏语言则安全回退为纯文本。

代码解析保留跨行语法状态。包含超过 8192 字节单行的代码块回退为转义后的纯文本，避免极长行触发昂贵的正则匹配；原始内容不会截断。解析、渲染在受并发约束的阻塞工作线程执行，不占用 Tokio 异步执行线程；这不是用户限流。

查看页复制按钮从内嵌 Base64 还原原始 UTF-8 内容，不复制行号或 HTML，也不额外读取 raw。浏览器剪贴板要求 HTTPS 或 localhost。raw 与下载保持原始换行，文件扩展名使用保存时确定的语言（包括 Auto 检测结果）；阅后即焚的本地快照下载使用相同文件名。Markdown 下载原始 `.md` 源码，HTML 下载 `.html` 源码，不下载渲染结果；文本响应仍保持 `text/plain`、`nosniff` 和 CSP sandbox。源码显示统一换行以对齐行号。fork 使用原生 textarea，编辑提交受浏览器换行规范化影响。Markdown `#L12` 链接会自动展开源码并定位。

### 当前安全边界

- 标题、表单内容由 Askama 转义；仅服务器生成的高亮 HTML 和清洗后的 Markdown 使用 `safe`。
- ammonia 移除脚本、事件属性、危险 URL 和任意样式；任务列表输入被强制为禁用复选框，脚注 ID 使用 `md-` 前缀隔离应用 DOM。
- 页面设置 CSP、`nosniff`、`Referrer-Policy: same-origin`、`no-store` 和禁止索引响应头；raw/下载额外设置 CSP sandbox。跨源链接和资源不发送 Referer，同源表单保留真实 Origin，避免普通 POST 在 `no-referrer` 策略下发送 `Origin: null` 而被来源校验拒绝；后端仍拒绝 null 或不匹配的 Origin。
- 查看、raw、下载、fork 共用密码/过期检查，不存在、过期或已焚毁均为 `404`；清理任务删除过期记录及失效凭证。
- 密码验证前只加载安全元数据，不返回正文或标题。网页解锁后使用 `HttpOnly; SameSite=Strict` Cookie；`BASE_URL` 为 HTTPS 时还设置 Secure。Cookie 按 paste 隔离，服务端有效期最长 1 小时且不超过 paste 到期时间。重启后仍可使用尚未到期的凭证，不涉及用户账号。
- 阅后即焚的所有网页 GET / HEAD 入口均不烧毁，包括 raw、下载和 fork。浏览器先显示确认页，POST 时校验随机确认令牌（最长 10 分钟）、来源和密码授权。没有请求 HTML 的 GET 返回 `400 Confirmation required`，不能绕过确认页；缺密码仍优先返回 `401`。
- 解锁和确认 POST 校验浏览器 Origin / Sec-Fetch-Site，确认令牌仅对指定 paste 和用途有效。密码只接受表单或 `X-Paste-Password` 请求头，不接受 URL 密码；不要把真实密码放入 shell 历史、公开命令或访问日志。
- `API_TOKEN` 只保护 REST API 写入，不是网页创建密码；API 读取仍需满足 paste 自身的密码要求。

### 表单端到端检查

以下调用的是网页表单处理器，**不是 REST API**；API 示例见第 5 阶段章节：

```sh
curl -i http://localhost:3000/paste \
  --data-urlencode 'title=Hello' \
  --data-urlencode 'content=fn main() { println!("Hello"); }' \
  --data-urlencode 'language=rs' \
  --data-urlencode 'expires_in=1h'
```

响应为 `303`，`Location: /xxxxxxxx`。把实际 ID 代入以下路径：

```sh
curl -i http://localhost:3000/xxxxxxxx
curl http://localhost:3000/raw/xxxxxxxx
curl -OJ http://localhost:3000/download/xxxxxxxx
```

浏览器访问 `/fork/xxxxxxxx` 应出现预填表单；只有再次点击 Create 才生成新记录。

## 测试

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

测试使用隔离临时数据库，不修改开发数据库，不修改进程环境。

运行服务后可以检查：

```sh
curl -i http://localhost:3000/
curl -i http://localhost:3000/missing
```

预期分别是 `200 text/html` 和 `404 application/json`。后者为 `{"error":{"code":"not_found","message":"Not found"}}`。浏览器发送 `Accept: text/html` 时，网页错误使用 HTML 状态页；raw / 下载可以显示密码解锁页或焚毁确认页，其他错误仍为 JSON。不存在、过期和已焚毁的记录统一为 Not found，不泄漏其历史状态。

`tests/paste_routes.rs` 覆盖内容往返、Markdown 展示、fork、各读取入口的过期和保护检查、精确的 1 MiB 边界与传输上限、错误 JSON、嵌入资源。服务单元测试覆盖随机 ID 格式与冲突重试、语言检测、所有过期选项及溢出、Markdown 白名单、高亮和密码验证。`tests/protection.rs` 覆盖所有内容入口的鉴权、令牌隔离和到期、重启持久性、确认流程、并发焚毁、删除失败不泄漏、周期清理与关闭。`tests/api.rs` 和 API 单元测试覆盖两种输入模式、解析、写入令牌、密码读取及原子焚毁。

## 第 3 阶段：界面开发

`static/styles.css` 是样式源文件，`static/app.css` 是随项目交付的生成文件，不要直接修改生成文件。Tailwind CLI 固定为 v4.3.5，脚本下载后校验 SHA-256，缓存至被 Git 忽略的 `.tools/`。支持 Linux x64 / arm64、macOS x64 / arm64；需要 curl、sh 和 sha256sum 或 shasum，不需要 Node/npm。普通 `cargo run` 直接使用已生成 CSS，不会联网下载 Tailwind。

```sh
sh scripts/build-css.sh          # 修改模板或样式后生成 CSS
sh scripts/build-css.sh --watch  # 可选：持续生成 CSS
sh scripts/build-css.sh --check  # 检查生成文件是否同步
cargo run --locked              # 资源嵌入二进制，修改后需重新编译并重启
node --test tests/*_ui.cjs       # 可选：编辑器、快捷键与折叠控件的模拟 DOM 回归测试，需要 Node
```

HTMX 2.0.8 自托管于 `static/vendor/htmx-2.0.8.min.js`，许可证见同目录 `htmx-LICENSE`。不依赖运行时 CDN。配置禁止 eval、动态脚本、跨站请求及历史快照；用户内容区域额外 `hx-disable`，Markdown 白名单移除 HTMX 属性。浏览器存储仅记录主题选择，不记录 paste 内容。无 JavaScript 时仍可正常提交、查看 raw、下载和 fork；主题跟随系统，复制按钮和 Markdown 源码切换需要 JavaScript。

创建成功时普通表单返回 `303 Location`；HTMX 返回 `200 HX-Redirect`，完整导航到新 paste。验证失败时 HTMX 只替换错误区域，保留输入；普通浏览器重新渲染并保留输入。创建页设置移至固定顶栏，编辑区至少为 `max(24rem, calc(100dvh - var(--toolbar-height) - 6rem - 2px))`。768–1099px 时密码和 Burn 收进 More；低于 768px 时顶栏显示首页、上传图标和 Settings，Create 固定于底部，并为安全区预留空间。收起的控件仍属于同一表单，Ctrl+Enter 通过原生表单校验后提交；请求期间禁用两个提交按钮并忽略重复快捷键。无 JavaScript 时设置直接展开。源码与 Markdown 阅读区至少为 `max(24rem, calc(100dvh - var(--toolbar-height) - 2rem))`，一行内容也拥有大块阅读区域。

```sh
curl -i -H 'Accept: text/html' http://localhost:3000/missing
curl -i -H 'HX-Request: true' --data-urlencode 'content=Hello' http://localhost:3000/paste
curl -i -H 'HX-Request: true' --data 'content=' http://localhost:3000/paste
```

依次应为 `404` HTML、`200` 带 `HX-Redirect`、`400` HTML 错误片段。新增自动测试覆盖这些响应、无 JS 校验回填、保护页不泄漏内容且不删除记录、HTMX 属性清洗与主题选择器。

### 编辑器行号与换行

- 编辑区的 Plain text 自动换行并隐藏行号；其他语言（包括 Markdown 源码）默认不换行，长行横向滚动。编辑区不放手动换行开关。
- Auto 默认按代码处理。只检查前 16384 个 UTF-16 单元：至少 120 个单元、不包含常见代码符号，并且至少三分之二的非空行是带句末标点的长句（至少 60 个单元），才按文章显示。
- 阅读区右下角有“换行”开关，只改变布局，不改写内容或重建文本节点；该偏好不写入 paste。Markdown 排版保持原有行为，切到源码后显示此开关。
- 编辑器和阅读器共用字体、字号、行高、上下/左右留白及行号颜色变量；编号按位数扩宽。编辑器只渲染可见行号，输入更新合并到下一动画帧，光标定位采用二分查找。聚焦代码输入框时，浅灰色当前行背景从行号后铺满正文区，随滚动同步；编号不着色，输入框不显示蓝色焦点框。
- 阅读区的纯文本和代码均保留行号及 `#L12` 行锚点。纯文本默认换行，代码默认横向滚动；行号与每个原始行使用同一 Grid 行，换行后也只在行首编号一次，无需 JavaScript 测量高度。复制、raw 和下载始终返回原文。

### 浏览器验收

- 两页使用同一个首页图标模板和固定顶栏样式：房子为 20px 内联 SVG、40px 链接点击区域，保留 `/` 地址、原生链接行为和键盘焦点。页面与顶栏共用左右边距变量；顶栏框架共用 `base.html` 的标题 / 操作插槽，高度统一为 `--toolbar-height: 4rem`（64px），不再用空白占位模拟操作布局。阅读页文件名省略截断并提供完整 title；复制、下载常驻，源码 / 文档切换、Raw、Fork 放入 ⋯ 菜单。窄屏只保留首页、文件名、⋯，复制、下载和主题组移入菜单，复用同一组控件而非复制按钮。菜单支持 Enter / Space、方向键、Home / End、Escape、Tab 离开及外部点击关闭。标签页标题与 favicon 不变。
- 分别在 1440px、900px、375px 下检查顶栏、More、Settings 和底部 Create；滚动时主操作始终可见。用 Escape 或点击外部收起设置，Ctrl+Enter 提交；空内容不能提交，收起后密码与 Burn 仍生效。
- 编辑器粘贴 5000 行代码、插入空行、删除与撤销，滚动到中间和末尾检查编号/光标高亮。切换语言和调整编辑区大小后，内容与选区保持不变。
- 创建一行文本：阅读区域仍占大块空间且显示行号；在阅读区切换换行，检查长行、空行、Tab、中文、emoji 与多行注释。折行不重复编号，关闭换行后可横向滚动且行号固定在左侧。
- 创建包含表格、引用、任务列表和围栏代码的 Markdown：默认渲染，图标切换源码；访问 `#L2` 自动显示源码并定位。
- System（显示器）、Light（太阳）、Dark（月亮）三个按钮同时展示，单击直接选择，当前项高亮；系统模式随系统变化，手动模式覆盖系统，刷新保留选择。
- 复制粘贴核对原文；raw、下载、fork 可用，fork 不会直接新增记录。
- 在窄屏和键盘 Tab 操作下检查控件、焦点、tooltip；错误页只有状态和 Home 主操作。
- 密码和 Burn 控件可用；密码错误时保持解锁页，密码正确后才显示内容或焚毁确认页；不存在或过期链接只显示 Not found。

自动化验证不等同浏览器视觉验收；当前尚未完成真实浏览器渲染验证。

### 文本 / 代码文件导入

- 将文件拖入编辑区，或点击顶部栏主题组分隔线左侧的上传图标（手机上直接位于 Settings 左侧）。图标保留「上传文件」提示和无障碍名称。文件仅在浏览器中读取，不新增上传接口；点击 Create 时仍提交普通文本 paste。
- 确认、成功和错误均使用原生 `<dialog>` 模态弹窗，不在正文中增加提示条；Escape 可取消确认，取消不更改正文。
- 文件名只填充空白 Title；扩展名选择现有语言，未知扩展名回到 Auto。已有内容时通过模态弹窗「替换 / 取消」确认；多文件只读取第一个。
- 文件读取上限为 `static/editor.js` 的 `MAX_UPLOAD_BYTES`，默认 `1024 * 1024`（1 MiB），读取前检查。服务器的 `MAX_CONTENT_BYTES` 同样默认 `1048576`（1 MiB），适用于网页表单、JSON API 和 text/plain API。文件上限按原文件字节计量，创建上限按转换后的 UTF-8 内容计量；旧编码转为 UTF-8 后若超过 1 MiB，创建仍返回 413。已有环境变量显式设置不会被新默认值覆盖，升级时请检查 `.env` 或部署环境。
- 优先严格解码 UTF-8，支持带 BOM 的 UTF-16 LE/BE。对 GB18030、Windows-1252 做保守推测，并要求弹窗确认；无 BOM 编码无法普遍可靠识别，不支持或无法识别时请先转换为 UTF-8。推测结果导入后请检查正文。
- 文本导入拒绝 Office 文档、图片、压缩包等类型，并检查常见二进制签名、NUL 和异常控制字符。PDF 进入独立附件模式，不经过文本解码。HTML / SVG 只写入 textarea 的文本值，绝不作为 HTML 插入页面。
- 验证：`node --test tests/*_ui.cjs`、`sh scripts/build-css.sh --check`、`cargo test --locked`。资源嵌入二进制，修改后需重新启动 `cargo run --locked`。
- 手动验收：空编辑器导入 `.py` / `.md` / `.txt` / `.html`，确认标题、语言、行号；输入内容后导入并取消 / 替换；拖入多个文件、超过 1 MiB 的文本文件、伪装成 `.txt` 的图片；切换亮暗主题，并在手机上点选文件。

### PDF 附件

- 使用同一个顶部上传按钮或拖放 PDF。按钮变成 PDF 图标，悬停或键盘聚焦显示文件名和大小；点击后可移除。不会增加文件卡片。Title 只在空白时填入文件名。
- 附加期间编辑框只读并显示文件名提示，隐藏行号、禁用语言；原文本、选区和语言暂存，移除附件时恢复。过期、密码和 Burn 设置照常使用。
- Create 显示上传百分比和服务器处理状态；失败使用现有弹窗提示，保留文件与设置，可直接重试。刷新页面不会保留本地尚未提交的附件。
- PDF 上限为 **10 MiB（10485760 字节）**，常量位于 `src/services/pdf.rs::MAX_PDF_BYTES`，由模板注入前端；前端读取前检查，后端流式接收时再次限制并检查 `%PDF-` 文件头。不相信扩展名或 MIME。文件头检查不等于完整 PDF 结构校验，损坏文件会在阅读时提示失败并保留原文件下载。
- PDF 通过 `POST /paste/pdf`、`multipart/form-data` 上传，字段为 `file`、`title`、`expires_in`、`password`、`burn_after_read`，返回 `201 { id, url }`。要求同源请求；请求体额外允许 16 KiB 表单开销。原有文本表单和 REST API 不变；JSON/text/plain API 不接受 PDF，API 读取 PDF 在鉴权后返回 400，且不会焚毁附件。
- 阅读器采用**自托管 PDF.js**，所有页面纵向连续排列，可直接下滑阅读；滚动同步当前页码，并保留上下页、页码跳转和缩放。只渲染视口附近的页面（最多缓存 8 张画布，每张最多约 3 百万像素），远离视口后释放画布，滚回时重新渲染；不同尺寸的页面按需修正占位高度，缩放及窗口变窄时保持阅读位置。不依赖手机浏览器是否支持内嵌 PDF。脚本、Worker、字体及解码资源均嵌入单个二进制，无 CDN 或 Node 运行时依赖。版本和来源见 `static/vendor/pdfjs/PROVENANCE.md`。只渲染页面画布，不执行 PDF 文档脚本或 XFA；PDF 自身加密的文件提示下载后打开，这与网站的 paste 密码保护不同。
- PDF 阅读页隐藏复制文本、源码切换和 fork，保留原始文件下载，文件名使用清理后的 Title（自动补 `.pdf`）。`/raw/:id` 和 `/download/:id` 同样经过密码、过期及焚毁确认检查，返回 `Content-Type: application/pdf`、`X-Content-Type-Options: nosniff`，不暴露公共附件目录。
- 迁移 `0003_pdf_files.sql` 将附件作为 SQLite BLOB 保存，存储标识为随机 paste ID 加 `.pdf`，不把用户文件名当路径。删除 paste 会通过外键级联删除附件；访问过期记录时立即删除，未访问记录由后台清理任务删除。数据库开启 `secure_delete`，但不承诺消除 WAL、系统快照或旧备份中的历史副本。
- 阅后即焚仍先显示确认页，密码验证后才确认读取。确认响应一次性包含 PDF 快照，服务端原子删除记录及 BLOB；浏览器用快照继续翻页和下载，不再请求已删除的文件地址。并发请求最多一个成功；网络中断可能造成删除后未收到内容。已交付的浏览器副本无法撤回。
- 验证：`cargo test --locked --test pdf`、`node --test tests/pdf_editor_ui.cjs tests/pdf_viewer_ui.cjs tests/reader_toolbar_ui.cjs`。覆盖大小边界、伪装文件、密码绕过、并发焚毁、过期级联删除、失败回滚、重启持久性、上传失败保留和阅读器翻页。升级后重新启动服务，迁移自动执行；备份 SQLite 持久卷即包含 PDF。

## 第 4 阶段：保护与生命周期

### 运行与手动测试

停止旧服务后启动新版本；新迁移保留原数据。若通过其他地址访问，请将 BASE_URL 改成实际 origin。公网部署应使用 HTTPS，TLS 终止和 Docker 配置见第 6 阶段。

```sh
cargo run --locked
cargo test --locked --test protection
cargo test --locked services::password
```

创建页设置密码，分别访问查看 / raw / 下载 / fork：未解锁不应看见标题或内容；错误密码应返回 401；正确密码后同浏览器可访问同一 paste，不能解锁其他 paste。下面只使用测试密码演示现有网页表单，并非 REST API：

```sh
curl -i http://localhost:3000/paste \
  --data-urlencode 'content=Secret sample' \
  --data-urlencode 'password=demo-only' \
  --data-urlencode 'expires_in=10m' \
  --data-urlencode 'burn_after_read=false'

# 将 xxxxxxxx 换成响应 Location 中的 ID
curl -i -c /tmp/rpaste-cookies.txt http://localhost:3000/unlock/xxxxxxxx \
  --data-urlencode 'password=demo-only' --data-urlencode 'target=raw'
curl -b /tmp/rpaste-cookies.txt http://localhost:3000/raw/xxxxxxxx
curl -H 'X-Paste-Password: demo-only' http://localhost:3000/raw/xxxxxxxx
rm -f /tmp/rpaste-cookies.txt
```

再创建 Burn paste：反复 GET、HEAD、链接预览只显示确认页，不删除；点击 View 后返回内容，此后原链接为 404。密码 + Burn 必须先解锁再确认，解锁 POST 本身不会烧毁。两个浏览器同时确认时最多一个成功。

### 阅后即焚边界

响应先完成高亮 / Markdown 渲染，然后使用带有效期和确认令牌条件的单条 DELETE 原子争抢；只有删除成功的请求才返回已经准备好的内容。渲染失败不会删除，数据库删除失败不返回内容。确认凭证随 paste 级联删除，重试或并发失败的请求不会拿到内容。

这里保证的是“最多一次服务端成功返回”，不是“用户一定看到”：数据库提交后网络中断或浏览器关闭仍可能导致内容丢失，无法把 HTTP 传输与用户看到页面做成同一事务。删除也不是磁盘安全擦除，SQLite WAL、空闲页或备份可能残留历史数据；密码保护不是内容加密。

成功烧毁后的页面保留当前内存中的内容：复制仍用原始字节，raw / 下载使用本地 Blob（需要 JavaScript），fork 提交当前内容快照并预填新表单；不会从已删除的服务器记录再次读取，也不会保存到 localStorage。无 JavaScript 时仍可通过确认页 POST 选择原请求的 raw / 下载 / fork 目标。

### 清理任务

启动时执行一次清理，每隔 CLEANUP_INTERVAL_SECONDS 清理到期 paste 和到期凭证；一次清理失败会记录错误并在下一轮重试。关闭服务时停止任务后再关闭连接池。访问时仍独立检查过期，因此即使清理未运行，过期内容也返回 404。自动测试使用临时/内存数据库，不会删除开发数据库的数据。

## 第 5 阶段：REST API

### 创建

`POST /api/paste` 支持 `application/json` 和 `text/plain`（UTF-8，可带 `charset=utf-8`）。成功返回 201 和 Location 响应头：

```json
{
  "id": "Ab12Cd34",
  "url": "http://localhost:3000/Ab12Cd34",
  "raw_url": "http://localhost:3000/raw/Ab12Cd34",
  "expires_at": 1794096000
}
```

此处 ID 和时间仅为示例；URL 使用配置的 BASE_URL。时间为 UTC Unix 秒，永不过期返回 null。

| 字段 | 类型 | 默认 / 约束 |
| --- | --- | --- |
| `content` | string，必填 | 非空，原始 UTF-8 字节最多 MAX_CONTENT_BYTES；保留空白、换行与 BOM |
| `title` | string，可选 | 最多 200 个字符；去除首尾空白，空标题返回 null |
| `language` | string，可选 | 省略或 `auto` 自动检测；`text` / `markdown`，或 `rs`、`py`、`js` 等语法名称 / 别名；无法识别的手动值返回 400 |
| `expires_in` | string，可选 | `10m` / `1h` / `1d` / `1w` / `1m` / `never`；默认 `1d`，`1m` 为 30 天 |
| `password` | string，可选 | 空字符串不设密码；最多 1024 字节，存储 Argon2id 哈希 |
| `burn_after_read` | boolean，可选 | 默认 false；JSON 必须使用布尔值而非字符串 |

JSON 模式将全部参数放入请求体，不接受 query 参数：

```sh
curl -i http://localhost:3000/api/paste \
  -H 'Content-Type: application/json' \
  --data-binary '{"content":"fn main() {}\n","title":"Example","language":"rs","expires_in":"1d"}'
```

text/plain 模式将请求体直接作为内容，其余字段放入 query；布尔值只能为 `true` / `false`，query 中的特殊字符需进行 URL 编码。未知字段、重复参数、错误类型返回 400。

```sh
# 一行创建，语言自动检测，默认保存一天
printf 'Hello\n' | curl -sS http://localhost:3000/api/paste -H 'Content-Type: text/plain' --data-binary @-

# 上传文件，保留原始换行；title 中的空格编码成 %20
curl -i 'http://localhost:3000/api/paste?language=rs&title=My%20code&expires_in=1w&burn_after_read=false' \
  -H 'Content-Type: text/plain; charset=utf-8' --data-binary @src/main.rs
```

密码可以通过 text/plain 的 `password` query 参数传入，但 URL 可能被代理、访问日志或 shell 历史记录；涉及密码时建议使用 JSON 请求体，并通过 HTTPS 调用。不要把真实密码或令牌写进共享脚本。

### 读取与密码

`GET /api/paste/:id` 成功返回 200，只包含以下字段：

```json
{
  "id": "Ab12Cd34",
  "title": "Example",
  "content": "fn main() {}\n",
  "language": "Rust",
  "created_at": 1794009600,
  "expires_at": 1794096000,
  "burn_after_read": false
}
```

返回原文而不是 HTML，title / expires_at 可以为 null，密码哈希不返回。设置密码的 paste 必须使用 `X-Paste-Password`，缺失或错误返回 401。API 不接受网页解锁 Cookie 替代密码头；不存在、过期或已焚毁均为 404，且不透露是否曾设置密码。

```sh
# 将 Ab12Cd34 替换为创建响应的实际 ID
curl -i http://localhost:3000/api/paste/Ab12Cd34

# 创建带密码且阅后即焚的测试内容
curl -i http://localhost:3000/api/paste \
  -H 'Content-Type: application/json' \
  --data-binary '{"content":"Secret sample","password":"demo-only","expires_in":"10m","burn_after_read":true}'

# 使用上一步新 ID；首次成功读取即删除，重试会得到 404
curl -i http://localhost:3000/api/paste/Ab12Cd34 \
  -H 'X-Paste-Password: demo-only'

# 普通非焚毁 paste 的原文入口同样接受密码头
curl http://localhost:3000/raw/Ab12Cd34 -H 'X-Paste-Password: demo-only'
```

API 的 GET 是明确的程序化读取，阅后即焚内容没有网页确认步骤：先验证密码、准备 JSON，再通过带过期条件的原子 DELETE 争抢，只有删除成功的请求返回内容。并发读取最多一个成功；错误密码不会烧毁。网页链接的 GET 仍只展示确认页。API 不支持 HEAD（405，且不会焚毁）；不要使用读取接口做链接预览或健康探测。API 响应禁止缓存。

与网页焚毁相同，删除提交后网络中断仍可能丢失内容；不保证客户端实际收到，不应自动重试焚毁读取。

### 可选写入保护

```sh
# 仅为本地测试示例，部署时使用私密的高熵令牌
API_TOKEN='replace-with-a-long-random-token' cargo run --locked

curl -i http://localhost:3000/api/paste \
  -H 'Authorization: Bearer replace-with-a-long-random-token' \
  -H 'Content-Type: text/plain' --data-binary 'Hello'
```

设置 API_TOKEN 后，两个创建模式都必须提供匹配的 Authorization 头，否则返回 401；未设置或空值则开放 API 创建。读取不需要 API_TOKEN，只依赖链接以及 paste 自身可选的密码。网页创建不受 API_TOKEN 限制，因此它不是全站写入保护。未实现限流。

### 错误与大小限制

错误统一使用 JSON，即使客户端声明 `Accept: text/html`：

```json
{"error":{"code":"bad_request","message":"Invalid expiration"}}
```

| HTTP | code | 情况 |
| --- | --- | --- |
| 400 | `bad_request` | 内容为空、JSON / UTF-8 / 参数无效、缺少或不支持 Content-Type |
| 401 | `unauthorized` | 创建令牌或读取密码缺失 / 不匹配 |
| 404 | `not_found` | 无效 ID、不存在、过期或已焚毁 |
| 413 | `payload_too_large` | 内容或整个请求体超限 |
| 405 | `method_not_allowed` | 不支持的方法；HEAD 按 HTTP 语义不返回响应体 |
| 500 | `internal_error` | 内部故障，响应不包含数据库或密码细节 |

大小按解码后的 content 的 UTF-8 字节计算，而不是字符数。text/plain 请求体上限等于内容上限；JSON 请求体另设 `6 × MAX_CONTENT_BYTES + 16 KiB` 上限，容纳 Unicode 转义和元数据，解码后仍必须满足内容上限。其他字段沿用表单校验，不会因 API 入口绕过限制。

### 运行与测试

停止旧服务并执行 `cargo run --locked`，无需新增迁移。随后使用上述 curl 示例；Bearer 测试需以相应 API_TOKEN 重启服务。

```sh
cargo test --locked --test api
cargo test --locked api::tests
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

API 测试覆盖 JSON / text/plain、字段与 UTF-8 校验、过期选项、内容与请求体大小、令牌、密码、Cookie 隔离、HEAD 不焚毁、过期 404、12 个并发请求的单次焚毁以及凭证级联删除。测试使用临时数据库，不触碰开发数据。

## 第 6 阶段：Docker 与 GitHub CI/CD

### 镜像结构

- 多阶段构建：Rust 1.99.1 / Debian Bookworm 编译；最终镜像仅复制一个 `rpaste` 应用二进制，包含嵌入的模板、静态资源和迁移，无需 PHP、Node、Rust 工具链或额外资源目录。
- 最终镜像使用 Debian slim，保留 glibc、libgcc、CA 证书与 curl（健康检查使用）。这是单应用二进制部署，不是完全静态的 scratch 镜像。
- SQLite 与 Oniguruma 随 Cargo 依赖编译；Docker 构建会用已校验 SHA-256 的 Tailwind standalone CLI 检查生成 CSS，无 Node 构建依赖。Node 只用于 CI 的现有 UI 测试。
- 默认以 UID/GID `10001:10001` 运行，数据库目录 `/data`；Compose 启用只读根文件系统、临时 `/tmp`、移除 capabilities、禁止提权。
- `.dockerignore` 使用白名单，开发数据库、`.env`、Git 目录、构建缓存和本地报告不会进入构建上下文。
- 健康检查访问首页，不读取 paste，因此不触发阅后即焚。它仅验证 HTTP 服务响应，不代替数据库写入探测。

容器内固定监听 `0.0.0.0:3000`，数据库 URL 为 `sqlite:///data/rpaste.db`。修改宿主机端口不需要修改容器端口。镜像以 SIGTERM 优雅退出，Compose 给出 30 秒退出时间。

### 本地构建与启动

需要 Docker Engine / Docker Desktop 和 Compose 插件（使用 `docker compose`，不使用旧 Python Compose v1）。首次构建需要联网获取基础镜像、系统包、crates 和 Tailwind CLI。

```sh
# 若已有 .env，保留它，不要覆盖
cp .env.example .env
docker compose config --quiet
docker compose up -d --build
docker compose ps
docker compose logs --tail=100 -f rpaste
```

打开 `http://localhost:3000`。首次启动自动迁移；数据库保存在 Compose 的 `rpaste-data` 命名卷中，**不会自动导入开发目录 `data/`**。新卷继承镜像 `/data` 的非 root 所有权；已有卷或自行挂载的宿主机目录必须允许 UID 10001 写入，不能只给数据库文件权限而不给目录权限，WAL/SHM 也需要创建。

如果之前的 `cargo run` 服务仍占用 3000 端口，请先停止旧服务，或者按下文改用另一个宿主机端口；不要让新旧服务同时监听同一端口。

Compose 默认只绑定宿主机 `127.0.0.1`。调整 `.env` 中的设置，例如本地改用 3001：

```dotenv
HOST_PORT=3001
BASE_URL=http://localhost:3001
```

如果确实需要从其他机器直接访问，额外设置 `HOST_IP=0.0.0.0` 并配置防火墙，同时将 BASE_URL 改成实际访问的 origin。公网不要裸露明文 HTTP，使用下文反向代理方案。

Compose 会使用 API_TOKEN、MAX_CONTENT_BYTES、DATABASE_MAX_CONNECTIONS、CLEANUP_INTERVAL_SECONDS、RUST_LOG；本地 `.env.example` 的 BIND_ADDR / DATABASE_URL 不覆盖容器内部固定路径与端口，避免把容器绑到回环地址或把数据库写到临时层。环境变量变更后重新执行 `docker compose up -d`，不要只执行 restart。

```sh
# 查看健康状态
docker inspect --format '{{.State.Health.Status}}' "$(docker compose ps -q rpaste)"

# 停止并移除容器，但保留数据库卷
docker compose down
```

不要使用 `docker compose down -v`，除非明确要删除全部 paste 数据。健康检查变成 unhealthy 不会单独触发 `unless-stopped` 重启策略；请查看日志处理故障。

### 使用 GHCR 预构建镜像

仓库工作流发布至 `ghcr.io/<owner>/<repo>`，名称自动转成小写。把占位符替换成自己的 GitHub owner/repository，在 `.env` 中设置：

```dotenv
RPASTE_IMAGE=ghcr.io/your-owner/rpaste:v1.0.0
BASE_URL=https://paste.example.com
```

这里的 v1.0.0 是发布示例，需先推送对应标签并等工作流成功。首次未发布版本时可使用默认分支成功构建的 `edge`。正式环境建议固定版本或 digest，不依赖会移动的 edge/latest。

```sh
docker compose pull rpaste
docker compose up -d --no-build
docker compose ps
```

同一个镜像标签包含 `linux/amd64` 和 `linux/arm64`，由运行主机自动选择，不要在 Compose 中写死 amd64。这里的 ARM 指 64 位 ARM；不提供 ARMv7 / 32 位镜像。树莓派需要 64 位系统。

GHCR 包的可见性独立于仓库：第一次发布后检查 GitHub Packages 的权限；公开包可匿名拉取，私有包需要具备该包读取权限的账号和带 `read:packages` 的 classic PAT，组织启用 SSO 时还需授权。PAT 只用于部署机器登录，不放入 Compose / 镜像：

```sh
printf '%s' "$GHCR_TOKEN" | docker login ghcr.io -u YOUR_GITHUB_USERNAME --password-stdin
```

### HTTPS 与反向代理

假设宿主机上已有 Caddy，域名 DNS 指向服务器，Compose 仍只发布 `127.0.0.1:3000`，将 BASE_URL 设置为 `https://paste.example.com`。Caddyfile 可以使用：

```caddyfile
paste.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

这段配置以宿主机 Caddy 为前提；如果代理在另一个容器中，应接入同一 Docker 网络并转发至 `rpaste:3000`，不能把代理容器自己的 localhost 当作 rpaste。

反向代理不要改写或删除 Origin / Referer，否则网页解锁或焚毁确认可能被拒绝。BASE_URL 必须是用户实际访问的 HTTPS origin（不能带子路径），应用据此校验来源、生成链接并设置 Secure Cookie。应用自身不终止 TLS，不依赖 X-Forwarded-Host 推导分享地址。

不要缓存 paste、raw、下载或 API 响应；应用已经发送 `Cache-Control: no-store`。代理的请求体上限至少容纳 JSON 转义后的传输大小及 PDF multipart：默认 JSON 传输上限为 6 MiB + 16 KiB，PDF 为 10 MiB + 16 KiB，代理可设 12 MiB，例如 Nginx 的 `client_max_body_size 12m`，提高 MAX_CONTENT_BYTES 或 PDF 常量时也要同步调整。API_TOKEN 仍只保护 API 创建，不会关闭网页创建或 PDF 上传。

### 数据备份、升级和回滚

SQLite 使用 WAL，不要在服务运行时只复制 `rpaste.db` 而忽略 WAL。下面用短暂停机方式打包完整目录，命名卷不会被删除：

```sh
umask 077
mkdir -p backups
docker compose stop rpaste
docker compose run --rm --no-deps -T --entrypoint tar rpaste -C /data -czf - . \
  > "backups/rpaste-$(date +%Y%m%d-%H%M%S).tar.gz"
docker compose start rpaste
```

备份文件包含 paste 明文和密码哈希，应妥善加密保管、限制访问。确认 tar 命令成功后再依赖该备份；备份也可能保留后来过期或已焚毁的内容。恢复时先停止服务，把备份解压到一个**空的新卷**，保持 UID/GID 10001 可写，再使用兼容该数据库版本的镜像启动；不要覆盖正在使用的数据库或混入旧 WAL。

升级流程：先备份，将 RPASTE_IMAGE 改为目标版本或 digest，再 `docker compose pull rpaste && docker compose up -d --no-build`，随后检查健康状态和日志。启动会自动执行尚未应用的迁移；迁移失败时不会启动 HTTP 服务。

应用不提供向下迁移。回滚到不兼容旧 schema 的版本时，应一起恢复升级前的完整备份，不能仅切换镜像。SQLite 部署保持单个应用实例和本地持久卷，不要对同一个数据库横向扩多个容器，也不要放在不可靠的网络文件系统上。

### GitHub Actions CI/CD

工作流文件：`.github/workflows/ci.yml`。提交并推送本项目到自己的 GitHub 仓库后才会运行，本地新增文件不会自动在 GitHub 生效。

触发条件：PR、推送 main / master、推送 `v*` 标签、Actions 页面手动运行。默认分支若不是 main/master，修改工作流的 branches 列表。发布条件仍只允许仓库默认分支或版本标签，其他分支手动运行只验证。

流程：

1. **Checks**：固定 Rust 1.99.1，执行 fmt、全部 Rust 测试、Clippy、现有 Node UI 测试、Tailwind 生成文件一致性检查和 Compose 语法检查。
2. **Images**：分别使用 `ubuntu-24.04` 和 `ubuntu-24.04-arm` 原生编译 AMD64 / ARM64，不通过 QEMU 编译 ARM。Buildx 缓存按架构分开。
3. **Smoke**：每种架构都实际启动刚构建的镜像，在非 root、只读根文件系统、真实命名卷下测试 API_TOKEN、JSON/text/plain、密码、阅后即焚、SIGTERM 正常退出和重启后数据持久性。测试卷执行后清理。
4. **Publish**：仅在允许发布的事件上，用仓库的 GITHUB_TOKEN 登录 GHCR，将通过测试的镜像推送。两种架构都成功后才创建公开使用的多架构标签，并校验 manifest 同时包含 amd64 / arm64。

发布标签规则：

| Git 事件 | GHCR 标签 |
| --- | --- |
| 默认分支成功构建 | `edge`、`sha-<完整 commit SHA>` |
| 稳定版本 `v1.2.3` | `v1.2.3`、`latest`、`sha-<完整 commit SHA>` |
| 预发布 `v1.2.3-rc.1` | `v1.2.3-rc.1`、`sha-<完整 commit SHA>`，不更新 latest |
| PR / 非默认分支手动运行 | 仅检查、构建、冒烟测试，不登录或发布镜像 |

版本标签格式要求 `vMAJOR.MINOR.PATCH` 或附带 `-prerelease`；不要使用 `vtest`。默认分支不更新 latest，避免覆盖稳定发布。单架构中间标签 `build-<run-id>-<attempt>-amd64/arm64` 保留用于合并和排查；未通过完整流程的构建不会更新 edge/latest/版本标签。清理旧包版本时注意保留仍被发布 manifest 引用的架构镜像。

首次配置：

- 仓库启用 GitHub Actions，允许该工作流及所引用的 Actions 运行。组织策略需要允许使用对应 ARM64 runner。
- 工作流顶层只有 contents:read；发布相关 job 明确请求 packages:write，使用内置 GITHUB_TOKEN，无需另建 Docker Hub 密钥或把 GHCR PAT 塞入仓库。
- 若同名 GHCR 包以前由其他方式创建，须在该包的 Manage Actions access 中授予此仓库写入权限。组织禁止创建包时，需要管理员先放行。
- Actions 使用完整 commit SHA 固定；更新依赖时重新核对上游版本与 SHA。Docker 基础镜像 tag 和系统仓库仍可能更新，因此构建不是逐字节完全可重现；线上固定已发布镜像 digest。
- PR 不使用 `pull_request_target`，不执行带发布凭据的外来 PR 代码。建议对默认分支和 `v*` 标签设置保护规则，只允许受信任维护者发布。

发布示例（仅在准备好版本后执行）：

```sh
git tag v1.0.0
git push origin v1.0.0
```

等待 Actions 全绿后检查架构：

```sh
docker buildx imagetools inspect ghcr.io/your-owner/rpaste:v1.0.0
docker pull --platform linux/arm64 ghcr.io/your-owner/rpaste:v1.0.0
```

本阶段的 CD 指**自动发布容器镜像**，没有向任何服务器执行 SSH、修改运行中的服务或接入自动更新器。服务器部署采用上述 Compose 拉取/更新流程；若后续需要自动上线，应另行配置目标服务器、环境审批与部署凭据。

### 本地验证

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
node --test tests/*_ui.cjs
sh scripts/build-css.sh --check
docker compose config --quiet
docker build -t rpaste:local .
bash scripts/test-container.sh rpaste:local
```

容器冒烟脚本需要宿主机 Bash、Python 3 标准库和容器引擎，不要求宿主机安装 Rust、Node 或第三方 Python 包；脚本使用独立随机端口与独立卷，不连接或清理开发数据库。它不会向 GHCR 推送。也可用兼容的 Podman 运行：`CONTAINER_ENGINE=podman bash scripts/test-container.sh localhost/rpaste:local`。若使用 Podman 构建，执行 `podman build --format docker -t localhost/rpaste:local .`，显式保留 Docker HEALTHCHECK；Podman 默认 OCI 格式可能忽略此配置。

GitHub 负责真正的 ARM64 构建和运行验证；本地 AMD64 构建成功不等于已验证 ARM。首次推送后需检查两个架构的 job 和最终 manifest，再用于生产。
