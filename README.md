# jira-rs

Agent 原生的 Jira 工具（Rust）。**单二进制 `jira`，两个表面**：`jira mcp` 起 stdio MCP server，其余子命令为人/脚本准备的 CLI（恒定 JSON 输出）。共用同一个类型化 client 库。

## 安装

Windows（PowerShell），装到 `%LOCALAPPDATA%\Programs\jira` 并加入用户 PATH，新开终端生效：

```powershell
irm https://raw.githubusercontent.com/wuyan19/jira-rs/main/scripts/install.ps1 | iex
```

Linux / macOS，装到 `~/.local/bin`（不在 PATH 时脚本会给出添加方法）：

```sh
sh -c "$(curl -fsSL https://raw.githubusercontent.com/wuyan19/jira-rs/main/scripts/install.sh)"
```

两个脚本都先下载再校验 SHA-256，不通过则不安装。

**国内网络**：脚本默认直连 GitHub。访问不稳时二选一：

- **加速站**：需同时代理 `github.com` 与 `raw.githubusercontent.com`。脚本与安装包都经加速站获取，版本号仍从 `github.com` 查询。

  ```powershell
  $m = '<加速站地址>'    # 形如 https://xxx.example
  $env:JIRA_DOWNLOAD_BASE = "$m/https://github.com"
  irm "$m/https://raw.githubusercontent.com/wuyan19/jira-rs/main/scripts/install.ps1" | iex
  ```

  ```sh
  m='<加速站地址>'
  JIRA_DOWNLOAD_BASE="$m/https://github.com" \
    sh -c "$(curl -fsSL "$m/https://raw.githubusercontent.com/wuyan19/jira-rs/main/scripts/install.sh")"
  ```

- **代理**：Windows 脚本遵循系统代理；sh 脚本遵循 curl/wget 的代理环境变量（如 `https_proxy`）。

**手工下载或内网分发**：从 [Releases](https://github.com/wuyan19/jira-rs/releases/latest) 取对应文件，放进 PATH 即可。单文件、无运行时依赖，可直接拷给内网同事。

| 平台 | 文件 |
|---|---|
| Windows x86_64 | `jira-<版本>-x86_64-pc-windows-msvc.exe` |
| Linux x86_64 / aarch64 | `jira-<版本>-x86_64-unknown-linux-musl`、`…-aarch64-unknown-linux-musl`（静态链接） |
| macOS Intel / Apple Silicon | `jira-<版本>-x86_64-apple-darwin`、`…-aarch64-apple-darwin` |

同名 `.sha256` 文件可用 `sha256sum -c` 校验（macOS 用 `shasum -a 256 -c`）。macOS 上经浏览器下载的文件带隔离属性，首次运行前执行 `xattr -d com.apple.quarantine ./jira`。

**从源码**（需 Rust 工具链）：`cargo install --git https://github.com/wuyan19/jira-rs --locked`。

## 升级

```sh
jira upgrade            # 升级到最新 release
jira upgrade --check    # 只查看是否有新版本
jira upgrade --force    # 已是最新也重装，用于修复损坏的安装
```

下载内容按 SHA-256 校验，不通过则不替换。正在运行的 `jira mcp` 进程仍是旧版本，重启 MCP 客户端后生效。

国内网络同样支持两种方式：环境变量 `JIRA_DOWNLOAD_BASE` 为加速站前缀（形如 `https://xxx.example/https://github.com`）；`HTTPS_PROXY` 指定代理，`JIRA_UPGRADE_PROXY` 可单独为升级指定代理。

## 开局设置

```sh
jira auth login            # 交互式：地址 → 认证方式 → 账号 → Token，验证通过才保存
jira auth status           # 查看生效配置（脱敏）与来源；--verify 附带网络验证
jira auth logout           # 删除已保存凭据
```

- 非交互模式（脚本/管道）：`jira auth login --url ... --user ... --token-stdin`（token 走 stdin 不进历史；此模式下 `--auth-type` 缺省 basic）
- Cloud（`*.atlassian.net`）自动选 Basic 并把账号提示为邮箱；DC PAT 选 Bearer 时不再询问账号
- 配置文件：Windows `%APPDATA%\jira\config.toml`，Linux `~/.config/jira/`，macOS `~/Library/Application Support/jira/`；路径可用 `JIRA_CONFIG_FILE` 覆盖；**token 明文存储**

## 使用

```sh
jira auth status --verify
jira get PROJ-123
jira search 'project = PROJ AND status = Open' -n 10
jira mcp                   # stdio MCP server
```

MCP 客户端接入（登录后**无需 env 块**，server 自行读取配置文件）：

```json
{ "mcpServers": { "jira": { "command": "jira", "args": ["mcp"] } } }
```

CI/自动化不想落文件时，继续用环境变量（整组生效时覆盖文件）。

## MCP tools（15 个）

每个工具的参数、返回与错误语义见 [docs/tools.md](docs/tools.md)。

| 类别 | tools |
|---|---|
| 连接 | `test_auth`、`server_info` |
| 查询 | `search_issues`(JQL+分页)、`get_issue`(含附件列表)、`get_comments`、`get_transitions` |
| 动态元数据 | `get_project_config`、`get_issue_type_metadata`(名字自动解析→id)、`get_custom_fields` |
| 写操作 | `create_issue`、`update_issue`、`transition_issue`(id 或名字)、`add_comment`（均带 dry_run） |
| 附件 | `attach_file`(multipart 上传)、`download_attachment`(按 id 落盘) |

CLI 同步暴露：`get / search / comments / transitions / project / issue-type-meta / fields / create / update / move / comment / attach / download`。

## 约定

- CLI：stdout 只出 JSON；诊断走 stderr；退出码 `1=config 2=auth 3=api 4=not found`
- 裸跑 `jira` 打印帮助并退出；`mcp` 永远是显式子命令，不会意外挂住 stdin
- MCP 模式下 stdout 被 rmcp 传输层独占
- `test_auth`（MCP tool）与 `auth status --verify`（CLI）输出含 `configSource`，标明 env / file(path)
- 写操作（create/update/move/comment）支持 `--dry-run` / `dry_run=true`：返回将发送的确切请求体而不发送
- MCP 错误带 `data.category`（config|auth|not_found|api|http|parse|io），Agent 据此决定重试/改配置/问人
- `jira upgrade [--check]` 输出 JSON，`status` 取值 `up_to_date` / `update_available` / `newer_than_release`
- 输出编码无需配置：PowerShell 读取外部程序输出时按控制台代码页解码，所以 `jira` 由 PowerShell 启动时自动按该代码页输出，代码页表示不了的字符在 JSON 中转义，内容不丢失；Git Bash、cmd、文件重定向与 MCP 均为 UTF-8。只有经 PowerShell 再经其他程序间接启动（如 `cmd /c jira …`）时不做转换。需要时可用 `JIRA_OUTPUT_ENCODING=utf-8` 关闭转换

## 配置解析（MCP 与 CLI 共用）

整体优先级 **环境变量 > 配置文件**，不做字段级合并：

| 变量 | 说明 |
|---|---|
| `JIRA_URL` | 基址，如 `https://jira.example.com` |
| `JIRA_USER` / `JIRA_EMAIL` | 用户名（Cloud 填邮箱），basic 认证必填 |
| `JIRA_API_TOKEN` | API token / 密码（basic）或 PAT（bearer） |
| `JIRA_AUTH_TYPE` | 设为 `bearer` 启用 DC PAT；缺省 basic |
| `JIRA_TIMEOUT_SECS` | 请求超时（秒），缺省 30 |
| `JIRA_CONFIG_FILE` | 覆盖配置文件路径 |

## 兼容性

**平台**：见[安装](#安装)的文件表。Linux 为静态链接二进制，不加载 NSS 模块（如 sssd、mdns），主机名解析只依赖 `/etc/hosts` 与 DNS。

**Jira 版本**：在 Jira **7.0.9** DC 真机验证（读写全路径）。针对旧版的适配：

- createmeta 用**聚合端点**（扁平端点 8.4+ 才有），响应里 `issuetypes` 全小写拼写（新版 camelCase）双兼容
- 描述/评论正文按 **wiki markup** 读写（v2 API 在 DC 上的格式；ADF 转换器保留给 Cloud v3）——Markdown↔wiki 双向转换
- `serverInfo.deploymentType` 在 7.x 上不存在（输出 null 属正常）

## 结构

```
crates/
├── jira-core       # 库：类型化 Jira REST（API v2）+ 认证 + Markdown↔wiki/ADF + 配置(env→file)
└── jira-app        # 唯一二进制 `jira`
    └── src/
        ├── main.rs     # 分发：mcp 子命令 → stdio server，其余 → CLI
        ├── cli.rs      # clap 子命令树
        ├── ops.rs      # CLI 与 MCP 共用的操作层（dry_run 在此实现）
        ├── upgrade.rs  # 自更新
        └── mcp/        # rmcp tool 层
scripts/            # 安装脚本与 release 说明片段
.github/workflows/  # CI（4 类平台构建+测试）；Release（5 目标构建 → 发布 → 各平台实跑安装脚本）
```

## 状态

P1 完成：查建改附闭环 + 动态元数据 + dry-run + 附件上传下载，真机（Jira 7.0.9）全部验证。P2 候选：sprint 全套、create_version、search_users、worklog、bulk、link/unlink、delete。
