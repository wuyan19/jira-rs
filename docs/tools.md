# jira tools 参考

`jira mcp` 提供 15 个工具，与 CLI 子命令一一对应（见文末对照表）。配置与启动方式见 README，本文只写调用细节。

## 通用约定

**写操作预演**：`create_issue`、`update_issue`、`transition_issue`、`add_comment` 支持 `dry_run`。置 `true` 时不发送请求，返回将提交的确切内容（`method`、`path`、`body`），用于发送前确认。

**错误结构**：调用失败返回结构化错误，`data.category` 取值与含义：

| category | 含义 | 建议处置 |
|---|---|---|
| `config` | 未登录或配置缺失 | 引导运行 `jira auth login` |
| `auth` | 凭据无效（401/403） | 停止重试，问用户 |
| `not_found` | 工单、项目、字段或流转不存在 | 检查 key 与可用值 |
| `api` | Jira 拒绝请求（含字段校验失败），附 `status` | 按错误消息修正后重试 |
| `http` / `io` / `parse` | 网络、文件或响应异常 | 可重试 |

**分页**：`search_issues`、`get_comments` 返回 `startAt`、`maxResults`、`total`，需要更多结果时用 `start_at` 翻页，不做自动聚合。

**字段逃生舱**：`create_issue` / `update_issue` 的 `fields` 接受任意 JSON 对象，直接合并进请求（优先级高于专用参数）。自定义字段用字段 id（`customfield_100xx`，从 `get_custom_fields` 查）。assignee 的写法按部署类型：Data Center 用 `{"name": "账号"}`，Cloud 用 `{"accountId": "..."}`。

**文本格式**：`description_md`、`body_md`、`comment_md` 收 Markdown，自动转为 Jira 所需格式；读方向的描述与评论也转成 Markdown 返回（`descriptionMarkdown`、`bodyMarkdown`）。

## 连接

### test_auth

验证凭据与连通性，返回服务端版本、部署类型、当前用户、配置来源（`configSource`，值为 `env` 或 `file(路径)`）。首次接入先调它。

无参数。

### server_info

返回 `version`、`deploymentType`、`baseUrl`。Jira 7.x 上 `deploymentType` 为 `null` 属正常。

## 查询

### search_issues

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `jql` | string | 是 | JQL 表达式，如 `project = X AND status = Open` |
| `max_results` | number | 否 | 页大小，默认 20 |
| `start_at` | number | 否 | 页偏移 |

返回 `issues` 数组与分页三元组。JQL 含项目条件时优先于任何项目过滤。

### get_issue

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |

返回完整 `issue`（全部字段 JSON）与 `descriptionMarkdown`。附件在 `issue.fields.attachment`（注意是单数），每项含 `id`、`filename`、`size`——下载附件要用这个 `id`。

### get_comments

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |
| `start_at` | number | 否 | 页偏移 |
| `max_results` | number | 否 | 页大小 |

返回 `comments` 数组（`author`、`created`、`bodyMarkdown`）与分页三元组。

### get_transitions

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |

返回当前状态下可用的 `transitions`（`id`、`name`、目标状态 `to`）。流转前先查它；可用流转随当前状态变化。

## 元数据

创建、更新前先查这三类，不要猜字段格式与取值。

### get_project_config

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `project` | string | 是 | 项目 key 或 id |

返回 `issueTypes`（`id` + `name`，创建工单要用的类型标识）与 `versions`。

### get_issue_type_metadata

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `project` | string | 是 | 项目 key 或 id |
| `issue_type` | string | 是 | 类型 id 或名称（名称自动解析） |

返回该类型的字段清单：每个字段的 `required`、`schema`、可选值（`allowedValues`）。注意个别字段标记为必填但服务端有默认值，实际以创建报错为准。

### get_custom_fields

返回全部字段（含自定义）：`id`、`name`、`custom`、`schema`。无参数。

## 写操作

均支持 `dry_run`。

### create_issue

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `project` | string | 是 | 项目 key 或 id |
| `issue_type` | string | 是 | 类型 id 或名称 |
| `summary` | string | 是 | 标题 |
| `description_md` | string | 否 | 描述，Markdown |
| `fields` | object | 否 | 额外字段，见"字段逃生舱" |
| `dry_run` | boolean | 否 | 默认 `false` |

返回 `created`、`key`、`url`（可直接给用户的浏览链接）。

### update_issue

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |
| `summary` | string | 否 | 新标题 |
| `description_md` | string | 否 | 新描述，**整体替换**非追加 |
| `fields` | object | 否 | 要设置的字段（assignee、priority、labels 等） |
| `dry_run` | boolean | 否 | 默认 `false` |

至少提供一项变更，否则报错。清空字段传 `null`，如 `{"assignee": null}`。

### transition_issue

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |
| `transition` | string | 是 | 流转 id 或名称 |
| `comment_md` | string | 否 | 随流转附带的评论，Markdown |
| `dry_run` | boolean | 否 | 默认 `false` |

名称不匹配时报错并列出全部可用流转，可直接自纠。返回 `transitioned`、实际执行的流转名。

### add_comment

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |
| `body_md` | string | 是 | 评论正文，Markdown |
| `dry_run` | boolean | 否 | 默认 `false` |

返回 `added`、`commentId`。

## 附件

### attach_file

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |
| `file_path` | string | 是 | 本机文件路径（stdio 模式即用户机器） |

返回 `attached` 与附件元数据（`id`、`filename`、`size`）。

### download_attachment

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 工单 key |
| `attachment_id` | string | 是 | 附件 id，来自 `get_issue` |
| `dest_dir` | string | 否 | 目标目录，默认当前目录，不存在则创建 |

返回 `downloaded`、`savedTo`（实际落盘路径，以它为准定位文件）。

## CLI 对照

| MCP tool | CLI 子命令 |
|---|---|
| `test_auth` | `jira auth status --verify` |
| `server_info` | `jira server-info` |
| `search_issues` | `jira search <jql> [-n N] [--start-at N]` |
| `get_issue` | `jira get <key>` |
| `get_comments` | `jira comments <key>` |
| `get_transitions` | `jira transitions <key>` |
| `get_project_config` | `jira project <project>` |
| `get_issue_type_metadata` | `jira issue-type-meta <project> <type>` |
| `get_custom_fields` | `jira fields` |
| `create_issue` | `jira create <project> <type> <summary> [-d 描述] [--fields JSON] [--dry-run]` |
| `update_issue` | `jira update <key> [--summary S] [-d 描述] [--fields JSON] [--dry-run]` |
| `transition_issue` | `jira move <key> <transition> [-c 评论] [--dry-run]` |
| `add_comment` | `jira comment <key> <body> [--dry-run]` |
| `attach_file` | `jira attach <key> <file>` |
| `download_attachment` | `jira download <key> <attachment_id> [-o 目录]` |

CLI 侧 `--dry-run` 对应 MCP 的 `dry_run`；退出码 `1=config 2=auth 3=api 4=not found` 对应上表的错误分类。
