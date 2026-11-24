//! MCP surface: stdio server exposing Jira tools (launched via `jira mcp`).
//!
//! Configuration comes from the environment or the config file written by
//! `jira auth login` (see `jira_core::config`). Tool logic lives in
//! `crate::ops` and is shared with the CLI surface.

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, ErrorCode, Implementation, ProtocolVersion,
        ServerCapabilities, ServerConfig,
    },
    schemars,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

use jira_core::{Error, JiraClient, JiraConfig};

use crate::ops;

// ---------------------------------------------------------------------------
// Tool argument structs (field docs become the schema descriptions agents see)
// ---------------------------------------------------------------------------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// JQL query expression, e.g. "project = PRJ AND status = Open"
    pub jql: String,
    /// Maximum results to return (default 20)
    pub max_results: Option<u32>,
    /// Page offset for paging through large result sets
    pub start_at: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct KeyArgs {
    /// Issue key, e.g. PRJ-123
    pub key: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CommentsArgs {
    /// Issue key
    pub key: String,
    /// Page offset
    pub start_at: Option<u32>,
    /// Maximum results per page
    pub max_results: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProjectArgs {
    /// Project key or id
    pub project: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct IssueTypeMetaArgs {
    /// Project key or id
    pub project: String,
    /// Issue type id or name (name is resolved via the project config)
    pub issue_type: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CreateArgs {
    /// Project key or id
    pub project: String,
    /// Issue type id or name
    pub issue_type: String,
    /// Issue summary (title)
    pub summary: String,
    /// Description in Markdown (converted to ADF)
    pub description_md: Option<String>,
    /// Extra/override fields as a JSON object, e.g. {"labels":["x"],"priority":{"name":"High"},"customfield_10001":3}
    pub fields: Option<serde_json::Value>,
    /// Preview the exact request body without sending (default false)
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct UpdateArgs {
    /// Issue key
    pub key: String,
    /// New summary
    pub summary: Option<String>,
    /// New description in Markdown (replaces the old one)
    pub description_md: Option<String>,
    /// Fields to set as a JSON object (assignee, priority, labels, custom fields by id)
    pub fields: Option<serde_json::Value>,
    /// Preview the exact request body without sending (default false)
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TransitionArgs {
    /// Issue key
    pub key: String,
    /// Transition id or name (from get_transitions)
    pub transition: String,
    /// Optional comment added with the transition (Markdown)
    pub comment_md: Option<String>,
    /// Preview the exact request body without sending (default false)
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CommentArgs {
    /// Issue key
    pub key: String,
    /// Comment body in Markdown (converted to ADF)
    pub body_md: String,
    /// Preview the exact request body without sending (default false)
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AttachArgs {
    /// Issue key
    pub key: String,
    /// Path of the file to attach, on the machine running the jira binary
    pub file_path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DownloadArgs {
    /// Issue key
    pub key: String,
    /// Attachment id (from get_issue's attachment list)
    pub attachment_id: String,
    /// Destination directory on the local machine (default: current directory)
    pub dest_dir: Option<String>,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct JiraMcp {
    /// Read by the `#[tool_handler]`-generated dispatch; not referenced directly.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

fn err(msg: impl Into<String>) -> McpError {
    McpError::internal_error(msg.into(), None)
}

/// Map a jira-core error to a structured MCP error. `data` carries
/// `category` (config|auth|not_found|api|http|parse|io) and, when known,
/// the HTTP status, so agents can decide between retry / re-configure /
/// ask-the-user.
fn mcp_error(e: &Error) -> McpError {
    let (category, status) = match e {
        Error::Unauthorized(m) => {
            return McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("authentication failed: {m}"),
                Some(json!({ "category": "auth" })),
            )
        }
        Error::NotFound(m) => {
            return McpError::new(
                ErrorCode::INVALID_PARAMS,
                m.clone(),
                Some(json!({ "category": "not_found" })),
            )
        }
        Error::Api { status, .. } => ("api", Some(status.as_u16())),
        Error::Http(_) => ("http", None),
        Error::Parse(_) => ("parse", None),
        Error::Io(_) => ("io", None),
    };
    McpError::new(
        ErrorCode::INTERNAL_ERROR,
        e.to_string(),
        Some(json!({ "category": category, "status": status })),
    )
}

fn ok_json(value: serde_json::Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| err(e.to_string()))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn client() -> Result<(JiraConfig, JiraClient), McpError> {
    let cfg = JiraConfig::load().map_err(|m| {
        McpError::new(ErrorCode::INVALID_PARAMS, m, Some(json!({ "category": "config" })))
    })?;
    let client = JiraClient::new(&cfg).map_err(|e| err(e.to_string()))?;
    Ok((cfg, client))
}

/// Run an ops call and map errors to structured MCP errors.
async fn call_ops<F, Fut>(f: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, Error>>,
{
    let value = f().await.map_err(|e| mcp_error(&e))?;
    ok_json(value)
}

#[tool_router]
impl JiraMcp {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    // ------------------------------------------------------------------
    // Auth / server
    // ------------------------------------------------------------------

    #[tool(description = "Verify Jira credentials and connectivity; returns server version, deployment type, current user and the active config source. Run this first to validate configuration.")]
    async fn test_auth(&self) -> Result<CallToolResult, McpError> {
        let (cfg, c) = client()?;
        let info = c.server_info().await.map_err(|e| mcp_error(&e))?;
        let me = c.myself().await.map_err(|e| mcp_error(&e))?;
        ok_json(json!({
            "ok": true,
            "configSource": cfg.source.label(),
            "server": info,
            "user": {
                "displayName": me.display_name,
                "email": me.email_address,
                "accountId": me.account_id,
                "name": me.name,
                "active": me.active,
            },
        }))
    }

    #[tool(description = "Get Jira server info: version, deployment type (Cloud/Server), base URL.")]
    async fn server_info(&self) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        let info = c.server_info().await.map_err(|e| mcp_error(&e))?;
        ok_json(json!(info))
    }

    // ------------------------------------------------------------------
    // Reads
    // ------------------------------------------------------------------

    #[tool(description = "Search issues with JQL, e.g. \"project = PRJ AND status = Open\". Returns issues plus startAt/maxResults/total for manual paging (pass start_at to fetch the next page).")]
    async fn search_issues(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::search(&c, &a.jql, a.start_at, a.max_results.unwrap_or(20))).await
    }

    #[tool(description = "Get one issue by key: all fields as JSON, description rendered to Markdown, attachments listed with ids (use download_attachment with those ids).")]
    async fn get_issue(&self, Parameters(a): Parameters<KeyArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::get_issue(&c, &a.key)).await
    }

    #[tool(description = "List comments on an issue with bodies rendered to Markdown. Paginated via start_at/max_results.")]
    async fn get_comments(&self, Parameters(a): Parameters<CommentsArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::comments(&c, &a.key, a.start_at, a.max_results)).await
    }

    #[tool(description = "List available workflow transitions (id, name, target status) for an issue. Use before transition_issue; transition_issue also accepts names.")]
    async fn get_transitions(&self, Parameters(a): Parameters<KeyArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::transitions(&c, &a.key)).await
    }

    #[tool(description = "Get project overview: issue types (id + name) and versions. Use to resolve issue type ids for create_issue.")]
    async fn get_project_config(&self, Parameters(a): Parameters<ProjectArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::project_config(&c, &a.project)).await
    }

    #[tool(description = "Get create-metadata for a project + issue type (accepts type id or name): required fields, per-field schema and allowed values. ALWAYS call this before create_issue instead of guessing field formats.")]
    async fn get_issue_type_metadata(&self, Parameters(a): Parameters<IssueTypeMetaArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::issue_type_metadata(&c, &a.project, &a.issue_type)).await
    }

    #[tool(description = "List all fields including custom fields with their ids, types and schemas. Prefer field ids (customfield_100xx) in create/update fields payloads.")]
    async fn get_custom_fields(&self) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::custom_fields(&c)).await
    }

    // ------------------------------------------------------------------
    // Writes (dry_run-aware)
    // ------------------------------------------------------------------

    #[tool(description = "Create an issue. description_md is Markdown (converted to ADF). fields: optional JSON object merged over the defaults — labels, assignee {name|accountId}, priority {name}, custom fields by id. Set dry_run=true to see the exact request body without sending.")]
    async fn create_issue(&self, Parameters(a): Parameters<CreateArgs>) -> Result<CallToolResult, McpError> {
        let (cfg, c) = client()?;
        call_ops(|| {
            ops::create_issue(
                &c,
                &cfg.url,
                &a.project,
                &a.issue_type,
                &a.summary,
                a.description_md.as_deref(),
                a.fields.as_ref(),
                a.dry_run.unwrap_or(false),
            )
        })
        .await
    }

    #[tool(description = "Update an issue: change summary/description (Markdown) and/or any fields via the JSON object (assignee, priority, labels, custom fields by id). At least one change is required. Set dry_run=true to preview the request body.")]
    async fn update_issue(&self, Parameters(a): Parameters<UpdateArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| {
            ops::update_issue(
                &c,
                &a.key,
                a.summary.as_deref(),
                a.description_md.as_deref(),
                a.fields.as_ref(),
                a.dry_run.unwrap_or(false),
            )
        })
        .await
    }

    #[tool(description = "Transition (move) an issue to another status. transition accepts the id or name from get_transitions; optional comment_md is added with the transition. Set dry_run=true to preview.")]
    async fn transition_issue(&self, Parameters(a): Parameters<TransitionArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| {
            ops::transition_issue(&c, &a.key, &a.transition, a.comment_md.as_deref(), a.dry_run.unwrap_or(false))
        })
        .await
    }

    #[tool(description = "Add a comment to an issue. body_md is Markdown (converted to ADF). Set dry_run=true to preview the request body.")]
    async fn add_comment(&self, Parameters(a): Parameters<CommentArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::add_comment(&c, &a.key, &a.body_md, a.dry_run.unwrap_or(false))).await
    }

    #[tool(description = "Attach a local file to an issue. file_path is a path on the machine running the jira binary (in stdio MCP mode: the user's machine).")]
    async fn attach_file(&self, Parameters(a): Parameters<AttachArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        call_ops(|| ops::attach_file(&c, &a.key, &a.file_path)).await
    }

    #[tool(description = "Download an attachment by id (ids come from get_issue's attachment list) into dest_dir (default: current directory). Returns the saved path; the file can then be read/analyzed locally.")]
    async fn download_attachment(&self, Parameters(a): Parameters<DownloadArgs>) -> Result<CallToolResult, McpError> {
        let (_, c) = client()?;
        let dir = a.dest_dir.unwrap_or_else(|| ".".to_string());
        call_ops(|| ops::download_attachment(&c, &a.key, &a.attachment_id, &dir)).await
    }
}

#[tool_handler]
impl ServerHandler for JiraMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "jira",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Jira tools. Configure via `jira auth login` (config file) or \
                 JIRA_URL / JIRA_USER (or JIRA_EMAIL) / JIRA_API_TOKEN / \
                 JIRA_AUTH_TYPE=bearer env vars. Start with test_auth. \
                 Before create/update, use get_issue_type_metadata and \
                 get_custom_fields instead of guessing field formats; \
                 prefer field ids. Writes support dry_run=true to preview \
                 the exact request. Errors carry data.category \
                 (config|auth|not_found|api|http|parse|io) to guide retries."
                    .to_string(),
            )
    }
}

/// Serve the MCP protocol over stdio until the client disconnects.
pub async fn serve() -> anyhow::Result<()> {
    let service = JiraMcp::new()
        .serve(stdio())
        .await
        .inspect_err(|e| eprintln!("jira mcp: failed to start: {e}"))?;
    service.waiting().await?;
    Ok(())
}
