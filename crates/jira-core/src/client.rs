//! HTTP client: typed requests with Jira error-body decoding.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::config::JiraConfig;
use crate::error::{Error, Result};
use crate::types::{
    CommentsPage, CreatemetaWrapper, CreatedIssue, CustomField, Issue, IssueComment, IssueTypeMeta,
    ProjectConfig, SearchResults, ServerInfo, TransitionsWrapper, UserInfo,
};

pub struct JiraClient {
    http: reqwest::Client,
    base: String,
}

impl JiraClient {
    pub fn new(config: &JiraConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .default_headers(config.auth.headers())
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .user_agent(concat!("jira-rs/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, base: format!("{}/rest/api/2", config.url) })
    }

    // ------------------------------------------------------------------
    // Reads
    // ------------------------------------------------------------------

    /// `GET /serverInfo`
    pub async fn server_info(&self) -> Result<ServerInfo> {
        self.get("/serverInfo").await
    }

    /// `GET /myself`
    pub async fn myself(&self) -> Result<UserInfo> {
        self.get("/myself").await
    }

    /// `GET /issue/{key}`
    pub async fn get_issue(&self, key: &str) -> Result<Issue> {
        self.get(&format!("/issue/{}", encode_path(key))).await
    }

    /// `GET /search?jql=...` (API v2 search; works on DC and legacy Cloud).
    pub async fn search(&self, jql: &str, start_at: Option<u32>, max_results: u32) -> Result<SearchResults> {
        let start = start_at.unwrap_or(0);
        self.get(&format!(
            "/search?jql={}&startAt={start}&maxResults={max_results}",
            percent_encode(jql),
        ))
        .await
    }

    /// `GET /issue/{key}/comment`
    pub async fn get_comments(
        &self,
        key: &str,
        start_at: Option<u32>,
        max_results: Option<u32>,
    ) -> Result<CommentsPage> {
        let mut qs = String::new();
        if let Some(v) = start_at {
            qs.push_str(&format!("&startAt={v}"));
        }
        if let Some(v) = max_results {
            qs.push_str(&format!("&maxResults={v}"));
        }
        self.get(&format!("/issue/{}/comment?{qs}", encode_path(key))).await
    }

    /// `GET /issue/{key}/transitions`
    pub async fn get_transitions(&self, key: &str) -> Result<TransitionsWrapper> {
        self.get(&format!("/issue/{}/transitions", encode_path(key))).await
    }

    /// `GET /project/{key}` — issue types and versions overview.
    pub async fn get_project(&self, project: &str) -> Result<ProjectConfig> {
        self.get(&format!("/project/{}", encode_path(project))).await
    }

    /// Createmeta for one project + issue type via the **aggregated** endpoint
    /// (`GET /issue/createmeta?projectKeys=&issuetypeIds=&expand=projects.issuetypes.fields`).
    /// Chosen over the flat per-type endpoint (8.4+) because Jira 7.x DC only
    /// has this one.
    pub async fn get_issue_type_meta(&self, project: &str, issue_type_id: &str) -> Result<IssueTypeMeta> {
        let path = format!(
            "/issue/createmeta?projectKeys={}&issuetypeIds={}&expand=projects.issuetypes.fields",
            percent_encode(project),
            percent_encode(issue_type_id),
        );
        let wrapper: CreatemetaWrapper = self.get(&path).await?;
        wrapper
            .projects
            .into_iter()
            .next()
            .and_then(|p| p.issue_types.into_iter().next())
            .ok_or_else(|| {
                Error::NotFound(format!("createmeta returned nothing for {project} / issue type {issue_type_id}"))
            })
    }

    /// `GET /field` — all fields, custom ones included.
    pub async fn get_fields(&self) -> Result<Vec<CustomField>> {
        self.get("/field").await
    }

    // ------------------------------------------------------------------
    // Writes
    // ------------------------------------------------------------------

    /// `POST /issue` with prebuilt `fields`.
    pub async fn create_issue(&self, fields: Value) -> Result<CreatedIssue> {
        self.post_json("/issue", json!({ "fields": fields })).await
    }

    /// `PUT /issue/{key}` with prebuilt `fields`.
    pub async fn update_issue(&self, key: &str, fields: Value) -> Result<()> {
        self.send_unit("PUT", &format!("/issue/{}", encode_path(key)), json!({ "fields": fields }))
            .await
    }

    /// `POST /issue/{key}/transitions`
    pub async fn transition_issue(&self, key: &str, transition_id: &str, comment_text: Option<String>) -> Result<()> {
        let mut body = json!({ "transition": { "id": transition_id } });
        if let Some(text) = comment_text {
            body["update"] = json!({ "comment": [ { "add": { "body": text } } ] });
        }
        self.send_unit("POST", &format!("/issue/{}/transitions", encode_path(key)), body).await
    }

    /// `POST /issue/{key}/comment` (wiki markup body)
    pub async fn add_comment(&self, key: &str, body_text: String) -> Result<IssueComment> {
        self.post_json(&format!("/issue/{}/comment", encode_path(key)), json!({ "body": body_text })).await
    }

    /// `POST /issue/{key}/attachments` (multipart; `X-Atlassian-Token: no-check`).
    pub async fn attach_file(&self, key: &str, path: &str) -> Result<Value> {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| Error::Io(format!("failed to read file {path}: {e}")))?;
        let filename = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());
        let part = reqwest::multipart::Part::bytes(data)
            .file_name(filename)
            .mime_str(mime_guess(path))
            .map_err(|e| Error::Parse(e.to_string()))?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let resp = self
            .http
            .post(format!("{}/issue/{}/attachments", self.base, encode_path(key)))
            .header("X-Atlassian-Token", "no-check")
            .multipart(form)
            .send()
            .await?;
        let value: Value = decode(resp).await?;
        Ok(value.as_array().and_then(|a| a.first().cloned()).unwrap_or(value))
    }

    /// Download an attachment by id into `dest_dir` (created if missing);
    /// returns the written path. Filename is sanitized to its basename.
    pub async fn download_attachment(&self, key: &str, attachment_id: &str, dest_dir: &str) -> Result<String> {
        let issue = self.get_issue(key).await?;
        let att = issue
            .fields
            .as_ref()
            .and_then(|f| f.attachments.iter().find(|a| a.id.as_deref() == Some(attachment_id)))
            .ok_or_else(|| Error::NotFound(format!("issue {key} has no attachment {attachment_id}")))?;
        let url = att
            .content
            .clone()
            .ok_or_else(|| Error::Parse("attachment has no content URL".into()))?;

        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(api_error(status, &String::from_utf8_lossy(&bytes)));
        }

        let filename = att
            .filename
            .as_deref()
            .and_then(|f| std::path::Path::new(f).file_name().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| "attachment.bin".to_string());
        let dir = if dest_dir.is_empty() { "." } else { dest_dir };
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| Error::Io(format!("failed to create directory {dir}: {e}")))?;
        let path = std::path::Path::new(dir).join(&filename);
        let path_str = path.display().to_string();
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| Error::Io(format!("failed to write {path_str}: {e}")))?;
        Ok(path_str)
    }

    // ------------------------------------------------------------------
    // Plumbing
    // ------------------------------------------------------------------

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.http.get(format!("{}{}", self.base, path)).send().await?;
        decode(resp).await
    }

    async fn post_json<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        let resp = self.http.post(format!("{}{}", self.base, path)).json(&body).send().await?;
        decode(resp).await
    }

    /// JSON request whose response body is empty (204) on success.
    async fn send_unit(&self, method: &str, path: &str, body: Value) -> Result<()> {
        let url = format!("{}{}", self.base, path);
        let req = if method == "PUT" { self.http.put(url) } else { self.http.post(url) };
        let resp = req.json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(api_error(status, &text));
        }
        Ok(())
    }
}

async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(api_error(status, &body));
    }
    serde_json::from_str(&body).map_err(|e| Error::Parse(e.to_string()))
}

/// Map a non-2xx response to a friendly error, extracting Jira's
/// `errorMessages` + `errors{field: msg}` when present.
fn api_error(status: reqwest::StatusCode, body: &str) -> Error {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            let mut parts: Vec<String> = Vec::new();
            if let Some(arr) = v.get("errorMessages").and_then(|m| m.as_array()) {
                for e in arr {
                    if let Some(s) = e.as_str() {
                        parts.push(s.to_string());
                    }
                }
            }
            if let Some(obj) = v.get("errors").and_then(|m| m.as_object()) {
                for (k, val) in obj {
                    parts.push(format!("{k}: {}", val.as_str().unwrap_or_default()));
                }
            }
            (!parts.is_empty()).then(|| parts.join("; "))
        })
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                format!("HTTP {}", status.as_u16())
            } else {
                trimmed.chars().take(300).collect()
            }
        });

    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            Error::Unauthorized(message)
        }
        reqwest::StatusCode::NOT_FOUND => Error::NotFound(message),
        _ => Error::Api { status, message },
    }
}

/// Percent-encode a path segment (issue keys are `[A-Z][A-Z0-9]*-\d+` but
/// stay defensive).
fn encode_path(s: &str) -> String {
    percent_encode(s)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Minimal extension-based MIME guess; unknown extensions are sent as
/// application/octet-stream (Jira sniffs content anyway).
fn mime_guess(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        _ => "application/octet-stream",
    }
}
