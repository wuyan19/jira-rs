//! CLI surface: subcommand definitions and JSON-first execution.
//!
//! stdout carries JSON only; diagnostics go to stderr; exit codes:
//! 1 = config, 2 = auth, 3 = api, 4 = not found.

use std::io::Read;

use clap::{Parser, Subcommand};
use dialoguer::{Input, Password, Select};
use jira_core::{Auth, Error, JiraClient, JiraConfig, ServerInfo, StoredConfig, UserInfo};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(
    name = "jira",
    version,
    about = "Agent-native Jira CLI (JSON output). Run `jira mcp` to start the stdio MCP server.",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the stdio MCP server (for MCP clients)
    Mcp,
    /// Manage credentials: login / logout / status
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Print server info
    ServerInfo,
    /// Fetch one issue by key (description rendered to Markdown)
    Get { key: String },
    /// Search with JQL
    Search {
        jql: String,
        /// Maximum results to return
        #[arg(short = 'n', long, default_value_t = 20)]
        max_results: u32,
        /// Page offset
        #[arg(long)]
        start_at: Option<u32>,
    },
    /// List comments on an issue (Markdown bodies)
    Comments {
        key: String,
        #[arg(short = 'n', long)]
        max_results: Option<u32>,
        #[arg(long)]
        start_at: Option<u32>,
    },
    /// List available workflow transitions for an issue
    Transitions { key: String },
    /// Show project config: issue types (id+name) and versions
    Project { project: String },
    /// Show create-metadata for a project + issue type (required fields, schemas)
    IssueTypeMeta { project: String, issue_type: String },
    /// List all fields incl. custom fields with ids
    Fields,
    /// Create an issue (write; supports --dry-run)
    Create {
        project: String,
        issue_type: String,
        summary: String,
        /// Description in Markdown
        #[arg(short = 'd', long)]
        description_md: Option<String>,
        /// Extra/override fields as a JSON object, e.g. '{"labels":["x"],"priority":{"name":"High"}}'
        #[arg(long)]
        fields: Option<String>,
        /// Show the request body without sending
        #[arg(long)]
        dry_run: bool,
    },
    /// Update an issue (write; supports --dry-run)
    Update {
        key: String,
        #[arg(long)]
        summary: Option<String>,
        /// New description in Markdown
        #[arg(short = 'd', long)]
        description_md: Option<String>,
        /// Extra/override fields as a JSON object (assignee, priority, labels, custom fields by id)
        #[arg(long)]
        fields: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Transition an issue by id or name (write; supports --dry-run)
    Move {
        key: String,
        transition: String,
        /// Optional comment added with the transition (Markdown)
        #[arg(short = 'c', long)]
        comment: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a comment in Markdown (write; supports --dry-run)
    Comment {
        key: String,
        body: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Attach a local file to an issue
    Attach { key: String, file: String },
    /// Download an attachment by id (see get_issue output)
    Download {
        key: String,
        attachment_id: String,
        /// Destination directory (default: current directory)
        #[arg(short = 'o', long)]
        out_dir: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AuthAction {
    /// Interactive login: verify credentials, then save to the config file
    Login {
        /// Jira URL (interactive prompt if omitted)
        #[arg(long)]
        url: Option<String>,
        /// Username for basic auth (interactive prompt if omitted)
        #[arg(long)]
        user: Option<String>,
        /// Auth type: basic | bearer (interactive selection if omitted)
        #[arg(long = "auth-type")]
        auth_type: Option<String>,
        /// Read the token from stdin (non-interactive mode)
        #[arg(long = "token-stdin")]
        token_stdin: bool,
    },
    /// Delete the saved config file
    Logout,
    /// Show active credentials (redacted) and config source; offline by default
    Status {
        /// Also verify against the server (network roundtrip)
        #[arg(long)]
        verify: bool,
    },
}

fn exit_code(e: &Error) -> i32 {
    match e {
        Error::Unauthorized(_) => 2,
        Error::NotFound(_) => 4,
        Error::Api { .. } => 3,
        _ => 1,
    }
}

fn finish(result: Result<Value, Error>) {
    match result {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(exit_code(&e));
        }
    }
}

fn require_config() -> JiraConfig {
    match JiraConfig::load() {
        Ok(c) => c,
        Err(m) => {
            eprintln!("config error: {m}");
            std::process::exit(1);
        }
    }
}

fn require_client(cfg: &JiraConfig) -> JiraClient {
    match JiraClient::new(cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("client error: {e}");
            std::process::exit(1);
        }
    }
}

/// Parse a --fields JSON string; exits with a clear message on bad JSON.
fn parse_fields(s: &Option<String>) -> Option<Value> {
    s.as_deref().map(|raw| {
        serde_json::from_str::<Value>(raw).unwrap_or_else(|e| {
            eprintln!("--fields JSON 解析失败: {e}");
            std::process::exit(1);
        })
    })
}

/// Execute a non-MCP subcommand. `Command::Mcp` is dispatched in `main`
/// before this point (MCP must start without requiring config).
pub async fn run(command: Command) {
    match command {
        Command::Mcp => unreachable!("dispatched in main before config resolution"),
        Command::Auth { action } => auth(action).await,
        Command::ServerInfo => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(async { Ok(json!(client.server_info().await?)) }.await)
        }
        Command::Get { key } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::get_issue(&client, &key).await)
        }
        Command::Search { jql, max_results, start_at } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::search(&client, &jql, start_at, max_results).await)
        }
        Command::Comments { key, max_results, start_at } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::comments(&client, &key, start_at, max_results).await)
        }
        Command::Transitions { key } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::transitions(&client, &key).await)
        }
        Command::Project { project } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::project_config(&client, &project).await)
        }
        Command::IssueTypeMeta { project, issue_type } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::issue_type_metadata(&client, &project, &issue_type).await)
        }
        Command::Fields => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::custom_fields(&client).await)
        }
        Command::Create { project, issue_type, summary, description_md, fields, dry_run } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            let fields = parse_fields(&fields);
            finish(crate::ops::create_issue(&client, &cfg.url, &project, &issue_type, &summary, description_md.as_deref(), fields.as_ref(), dry_run).await)
        }
        Command::Update { key, summary, description_md, fields, dry_run } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            let fields = parse_fields(&fields);
            finish(crate::ops::update_issue(&client, &key, summary.as_deref(), description_md.as_deref(), fields.as_ref(), dry_run).await)
        }
        Command::Move { key, transition, comment, dry_run } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::transition_issue(&client, &key, &transition, comment.as_deref(), dry_run).await)
        }
        Command::Comment { key, body, dry_run } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::add_comment(&client, &key, &body, dry_run).await)
        }
        Command::Attach { key, file } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            finish(crate::ops::attach_file(&client, &key, &file).await)
        }
        Command::Download { key, attachment_id, out_dir } => {
            let cfg = require_config();
            let client = require_client(&cfg);
            let out = out_dir.unwrap_or_else(|| ".".to_string());
            finish(crate::ops::download_attachment(&client, &key, &attachment_id, &out).await)
        }
    }
}

// ---------------------------------------------------------------------------
// auth subcommand group
// ---------------------------------------------------------------------------

async fn auth(action: AuthAction) {
    match action {
        AuthAction::Login { url, user, auth_type, token_stdin } => {
            login(url, user, auth_type, token_stdin).await
        }
        AuthAction::Logout => logout(),
        AuthAction::Status { verify } => status(verify).await,
    }
}

fn interactive_unavailable() -> ! {
    eprintln!("交互输入不可用（非终端），请改用 --url/--user/--auth-type/--token-stdin 参数");
    std::process::exit(1);
}

async fn verify_credentials(
    client: &JiraClient,
) -> Result<(ServerInfo, UserInfo), Error> {
    // /serverInfo is anonymous-accessible on many DC installs;
    // /myself is the actual auth check.
    let info = client.server_info().await?;
    let me = client.myself().await?;
    Ok((info, me))
}

async fn login(url: Option<String>, user: Option<String>, auth_type: Option<String>, token_stdin: bool) {
    // --token-stdin declares non-interactive mode: required flags instead of
    // prompts, auth_type defaults to basic.
    let url = match url {
        Some(u) => u,
        None if token_stdin => {
            eprintln!("--token-stdin 模式需要 --url");
            std::process::exit(1);
        }
        None => Input::new()
            .with_prompt("Jira 地址")
            .interact_text()
            .unwrap_or_else(|_| interactive_unavailable()),
    };
    let cloud = url.to_lowercase().ends_with(".atlassian.net");

    let auth_type = match auth_type {
        Some(t) => t,
        None if token_stdin || cloud => "basic".to_string(),
        None => {
            let idx = Select::new()
                .with_prompt("认证方式")
                .items(&["Basic (API token / 密码)", "Bearer (DC PAT)"])
                .default(0)
                .interact()
                .unwrap_or_else(|_| interactive_unavailable());
            if idx == 1 { "bearer".to_string() } else { "basic".to_string() }
        }
    };
    let bearer = auth_type.eq_ignore_ascii_case("bearer");

    let user = if bearer {
        None
    } else {
        match user {
            Some(u) => Some(u),
            None if token_stdin => {
                eprintln!("--token-stdin 模式下 basic 认证需要 --user");
                std::process::exit(1);
            }
            None => {
                let prompt = if cloud { "邮箱" } else { "账号" };
                Some(
                    Input::new()
                        .with_prompt(prompt)
                        .interact_text()
                        .unwrap_or_else(|_| interactive_unavailable()),
                )
            }
        }
    };

    let token = if token_stdin {
        let mut buf = String::new();
        let _ = std::io::stdin().read_to_string(&mut buf);
        let t = buf.trim().to_string();
        if t.is_empty() {
            eprintln!("stdin 未提供 token");
            std::process::exit(1);
        }
        t
    } else {
        Password::new()
            .with_prompt("Token")
            .interact()
            .unwrap_or_else(|_| interactive_unavailable())
    };

    // Verify against the server before saving anything.
    let stored = StoredConfig { url, auth_type, user, token, timeout_secs: 30 };
    let cfg = match JiraConfig::from_stored(&stored) {
        Ok(c) => c,
        Err(m) => {
            eprintln!("config error: {m}");
            std::process::exit(1);
        }
    };
    let client = require_client(&cfg);
    let (info, me) = match verify_credentials(&client).await {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("验证失败（未保存）: {e}");
            std::process::exit(exit_code(&e));
        }
    };

    let path = match JiraConfig::save_file(&stored) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("{m}");
            std::process::exit(1);
        }
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "server": { "version": info.version, "deploymentType": info.deployment_type },
            "user": { "displayName": me.display_name, "email": me.email_address },
            "savedTo": path.display().to_string(),
            "note": "token 以明文存储于上述文件",
        }))
        .unwrap()
    );
}

fn logout() {
    let path = JiraConfig::config_path().unwrap_or_else(|m| {
        eprintln!("{m}");
        std::process::exit(1);
    });
    match JiraConfig::delete_file() {
        Ok(true) => {
            println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "deleted": path.display().to_string() })).unwrap())
        }
        Ok(false) => {
            eprintln!("配置文件不存在: {}", path.display());
            std::process::exit(1);
        }
        Err(m) => {
            eprintln!("{m}");
            std::process::exit(1);
        }
    }
}

async fn status(verify: bool) {
    let cfg = require_config();
    let (auth_type, user) = match &cfg.auth {
        Auth::Basic { user, .. } => ("basic", Some(user.clone())),
        Auth::Bearer { .. } => ("bearer", None),
    };
    let mut value = json!({
        "source": cfg.source.label(),
        "url": cfg.url,
        "authType": auth_type,
        "user": user,
        "token": "(已设置，已脱敏)",
        "timeoutSecs": cfg.timeout_secs,
        "configFile": JiraConfig::config_path().ok().map(|p| p.display().to_string()),
        // null = 未验证（离线）；--verify 后为 true/false
        "authenticated": serde_json::Value::Null,
    });
    if verify {
        let client = require_client(&cfg);
        match verify_credentials(&client).await {
            Ok((info, me)) => {
                value["authenticated"] = json!(true);
                value["verified"] = json!({
                    "ok": true,
                    "server": { "version": info.version, "deploymentType": info.deployment_type },
                    "user": { "displayName": me.display_name, "email": me.email_address },
                });
            }
            Err(e) => {
                value["authenticated"] = json!(false);
                value["verified"] = json!({ "ok": false, "error": e.to_string() });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
                std::process::exit(exit_code(&e));
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}
