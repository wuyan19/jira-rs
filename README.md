# jira-rs

Agent 原生的 Jira 工具（Rust）。**单二进制 `jira`，两个表面**：`jira mcp` 起 stdio MCP server，其余子命令为人/脚本准备的 CLI（恒定 JSON 输出）。共用同一个类型化 client 库。

## 开局设置

```sh
jira auth login            # 交互式：地址 → 认证方式 → 账号 → Token，验证通过才保存
jira auth status           # 查看生效配置（脱敏）与来源；--verify 附带网络验证
jira auth logout           # 删除已保存凭据
```

- 非交互模式（脚本/mintty）：`jira auth login --url ... --user ... --token-stdin`（token 走 stdin 不进历史；此模式下 `--auth-type` 缺省 basic）
- Cloud（`*.atlassian.net`）自动选 Basic 并把账号提示为邮箱；DC PAT 选 Bearer 时不再询问账号
- 配置文件：`%APPDATA%\jira\config.toml`（Linux `~/.config/jira/`），路径可用 `JIRA_CONFIG_FILE` 覆盖；**token 明文存储**

## 结构

```
crates/
├── jira-core       # 库：类型化 Jira REST（API v2）+ 认证 + ADF↔Markdown + 配置(env→file)
└── jira-app        # 唯一二进制 `jira`
    └── src/
        ├── main.rs # 分发：mcp 子命令 → stdio server，其余 → CLI
        ├── cli.rs  # clap 子命令树（auth / server-info / get / search）
        └── mcp/    # rmcp tool 层（当前 tools: test_auth, server_info）
```

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

## 快速开始

```sh
cargo build --release
./target/release/jira.exe auth login

./target/release/jira.exe auth status --verify
./target/release/jira.exe get PROJ-123
./target/release/jira.exe search 'project = PROJ AND status = Open' -n 10
./target/release/jira.exe mcp    # stdio MCP server
```

MCP 客户端接入（登录后**无需 env 块**，server 自行读取配置文件）：

```json
{ "mcpServers": { "jira": { "command": "jira.exe", "args": ["mcp"] } } }
```

CI/自动化不想落文件时，继续用环境变量（整组生效时覆盖文件）。

## 约定

- CLI：stdout 只出 JSON；诊断走 stderr；退出码 `1=config 2=auth 3=api 4=not found`
- 裸跑 `jira` 打印帮助并退出；`mcp` 永远是显式子命令，不会意外挂住 stdin
- MCP 模式下 stdout 被 rmcp 传输层独占
- `test_auth`（MCP tool）与 `auth status --verify`（CLI）输出含 `configSource`，标明 env / file(path)
- 写操作（create/update/move/comment）支持 `--dry-run` / `dry_run=true`：返回将发送的确切请求体而不发送
- MCP 错误带 `data.category`（config|auth|not_found|api|http|parse|io），Agent 据此决定重试/改配置/问人

## MCP tools（15 个，P1）

每个工具的参数、返回与错误语义见 [docs/tools.md](docs/tools.md)。

| 类别 | tools |
|---|---|
| 连接 | `test_auth`、`server_info` |
| 查询 | `search_issues`(JQL+分页)、`get_issue`(含附件列表)、`get_comments`、`get_transitions` |
| 动态元数据 | `get_project_config`、`get_issue_type_metadata`(名字自动解析→id)、`get_custom_fields` |
| 写操作 | `create_issue`、`update_issue`、`transition_issue`(id 或名字)、`add_comment`（均带 dry_run） |
| 附件 | `attach_file`(multipart 上传)、`download_attachment`(按 id 落盘) |

CLI 同步暴露：`get / search / comments / transitions / project / issue-type-meta / fields / create / update / move / comment / attach / download`。

## 兼容性

真机验证于 Jira **7.0.9** DC（2026-11-23）+ 假 Jira E2E。针对旧版的适配：

- createmeta 用**聚合端点**（扁平端点 8.4+ 才有），响应里 `issuetypes` 全小写拼写（新版 camelCase）双兼容
- 描述/评论正文按 **wiki markup** 读写（v2 API 在 DC 上的格式；ADF 转换器保留给 Cloud v3）——Markdown↔wiki 双向转换
- `serverInfo.deploymentType` 在 7.x 上不存在（输出 null 属正常）

## 状态

P1 完成（2026-11-23）：查建改附闭环 + 动态元数据 + dry-run + 附件上传下载；真机（Jira 7.0.9）只读+元数据+MCP+dry-run 全部验证通过，真实写操作待测试项目确认。P2 候选：sprint 全套、create_version、search_users、worklog、bulk、link/unlink、delete。
