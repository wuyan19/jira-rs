//! Configuration: whole-config precedence — environment first, then the
//! TOML file written by `jira auth login`.
//!
//! File (default `%APPDATA%\jira\config.toml` / `~/.config/jira/config.toml`,
//! overridable via `JIRA_CONFIG_FILE`):
//!
//! ```toml
//! url = "https://jira.example.com"
//! auth_type = "basic"        # or "bearer" (DC PAT)
//! user = "wuyan"             # omitted for bearer
//! token = "..."
//! timeout_secs = 30
//! ```
//!
//! Environment (when fully resolvable it overrides the whole file — no merge):
//!
//! | Variable            | Meaning                                      |
//! |---------------------|----------------------------------------------|
//! | `JIRA_URL`          | Base URL, e.g. `https://jira.example.com`    |
//! | `JIRA_USER`/`JIRA_EMAIL` | Username (Cloud: email; basic auth)    |
//! | `JIRA_API_TOKEN`    | API token / password (basic) or PAT (bearer) |
//! | `JIRA_AUTH_TYPE`    | `basic` (default) or `bearer` for DC PAT     |
//! | `JIRA_TIMEOUT_SECS` | Request timeout in seconds                   |
//! | `JIRA_CONFIG_FILE`  | Override the config file path                |

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::auth::Auth;

/// Contents of the on-disk config file (`jira auth login` writes this).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConfig {
    pub url: String,
    /// "basic" (default) or "bearer".
    pub auth_type: String,
    /// Required for basic; unused for bearer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub token: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

/// Where a resolved config came from.
#[derive(Debug, Clone)]
pub enum ConfigSource {
    Env,
    File(PathBuf),
}

impl ConfigSource {
    pub fn label(&self) -> String {
        match self {
            Self::Env => "env".to_string(),
            Self::File(p) => format!("file({})", p.display()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JiraConfig {
    pub url: String,
    pub auth: Auth,
    pub timeout_secs: u64,
    pub source: ConfigSource,
}

impl JiraConfig {
    pub fn new(url: impl Into<String>, auth: Auth) -> Self {
        let url = url.into().trim_end_matches('/').to_string();
        Self { url, auth, timeout_secs: 30, source: ConfigSource::Env }
    }

    /// Config file path: `$JIRA_CONFIG_FILE` or the OS config dir.
    pub fn config_path() -> Result<PathBuf, String> {
        if let Ok(p) = std::env::var("JIRA_CONFIG_FILE") {
            if !p.trim().is_empty() {
                return Ok(PathBuf::from(p));
            }
        }
        dirs::config_dir()
            .map(|d| d.join("jira").join("config.toml"))
            .ok_or_else(|| "cannot determine the system config directory; set JIRA_CONFIG_FILE".to_string())
    }

    /// Read the config file; `Ok(None)` when it does not exist.
    pub fn load_file() -> Result<Option<StoredConfig>, String> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let stored: StoredConfig = toml::from_str(&text)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
        Ok(Some(stored))
    }

    pub fn save_file(stored: &StoredConfig) -> Result<PathBuf, String> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create directory {}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(stored)
            .map_err(|e| format!("failed to serialize the config: {e}"))?;
        std::fs::write(&path, text)
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
        Ok(path)
    }

    /// Delete the config file; returns whether it existed.
    pub fn delete_file() -> Result<bool, String> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)
            .map_err(|e| format!("failed to delete {}: {e}", path.display()))?;
        Ok(true)
    }

    pub fn from_stored(stored: &StoredConfig) -> Result<Self, String> {
        let url = stored.url.trim_end_matches('/').to_string();
        if url.is_empty() {
            return Err("url is empty".to_string());
        }
        if stored.token.is_empty() {
            return Err("token is empty".to_string());
        }
        let auth = if stored.auth_type.eq_ignore_ascii_case("bearer") {
            Auth::Bearer { token: stored.token.clone() }
        } else {
            let user = stored
                .user
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .ok_or("basic auth requires a user")?;
            Auth::Basic { user: user.to_string(), token: stored.token.clone() }
        };
        Ok(Self {
            url,
            auth,
            timeout_secs: stored.timeout_secs,
            source: ConfigSource::File(Self::config_path()?),
        })
    }

    /// Build a config from environment variables only.
    pub fn from_env() -> Result<Self, String> {
        let url = std::env::var("JIRA_URL")
            .map_err(|_| "JIRA_URL is not set (e.g. https://jira.example.com)".to_string())?
            .trim_end_matches('/')
            .to_string();
        if url.is_empty() {
            return Err("JIRA_URL is empty".to_string());
        }

        let token = std::env::var("JIRA_API_TOKEN")
            .map_err(|_| "JIRA_API_TOKEN is not set".to_string())?;
        if token.is_empty() {
            return Err("JIRA_API_TOKEN is empty".to_string());
        }

        let auth_type = std::env::var("JIRA_AUTH_TYPE").unwrap_or_default();
        let auth = if auth_type.eq_ignore_ascii_case("bearer") {
            Auth::Bearer { token }
        } else {
            let user = std::env::var("JIRA_USER")
                .or_else(|_| std::env::var("JIRA_EMAIL"))
                .map_err(|_| "JIRA_USER (or JIRA_EMAIL) is not set for basic auth".to_string())?;
            if user.is_empty() {
                return Err("JIRA_USER is empty".to_string());
            }
            Auth::Basic { user, token }
        };

        let timeout_secs = std::env::var("JIRA_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        Ok(Self { url, auth, timeout_secs, source: ConfigSource::Env })
    }

    /// Resolve the active config: environment first (as a whole), then the
    /// file. Partial env setups do not merge with the file.
    pub fn load() -> Result<Self, String> {
        match Self::from_env() {
            Ok(cfg) => Ok(cfg),
            Err(env_err) => match Self::load_file() {
                Ok(Some(stored)) => Self::from_stored(&stored),
                Ok(None) => Err(format!("{env_err}; no config file was found either (run `jira auth login` first)")),
                Err(file_err) => Err(file_err),
            },
        }
    }
}
