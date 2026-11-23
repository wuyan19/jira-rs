//! REST response types (API v2). Unknown fields are ignored; optional
//! fields are `Option` so partial responses never fail a decode.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub deployment_type: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub build_date: Option<String>,
    #[serde(default)]
    pub server_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub email_address: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub time_zone: Option<String>,
}

/// Generic `{id, name}` shape used by status, priority, issuetype, ...
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedValue {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueComment {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub author: Option<UserInfo>,
    /// ADF document; render with [`crate::adf::to_markdown`].
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentsWrapper {
    #[serde(default)]
    pub comments: Vec<IssueComment>,
    #[serde(default)]
    pub total: Option<u32>,
    #[serde(default)]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueFields {
    #[serde(default)]
    pub summary: Option<String>,
    /// ADF document (`{"version":..,"type":"doc",..}`).
    #[serde(default)]
    pub description: Option<Value>,
    #[serde(default)]
    pub status: Option<NamedValue>,
    #[serde(default)]
    pub issuetype: Option<NamedValue>,
    #[serde(default)]
    pub priority: Option<NamedValue>,
    #[serde(default)]
    pub assignee: Option<UserInfo>,
    #[serde(default)]
    pub reporter: Option<UserInfo>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Jira's field name is singular `attachment`.
    #[serde(default, rename = "attachment")]
    pub attachments: Vec<AttachmentMeta>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub comment: Option<CommentsWrapper>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub fields: Option<IssueFields>,
}

impl Issue {
    /// Description rendered as Markdown (empty string when absent).
    pub fn description_markdown(&self) -> String {
        self.fields
            .as_ref()
            .and_then(|f| f.description.as_ref())
            .map(crate::adf::to_markdown)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    #[serde(default)]
    pub start_at: Option<u32>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub total: Option<u32>,
    #[serde(default)]
    pub issues: Vec<Issue>,
}

// ---------------------------------------------------------------------------
// Comments / transitions / attachments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentsPage {
    #[serde(default)]
    pub comments: Vec<IssueComment>,
    #[serde(default)]
    pub start_at: Option<u32>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub total: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Target status.
    #[serde(default)]
    pub to: Option<NamedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionsWrapper {
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

/// Attachment entry as returned inside `fields.attachment`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    /// Absolute download URL (requires the same auth).
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub author: Option<UserInfo>,
}

// ---------------------------------------------------------------------------
// Project / metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub issue_types: Vec<IssueTypeRef>,
    #[serde(default)]
    pub versions: Vec<NamedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueTypeRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subtask: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSchema {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub items: Option<String>,
    #[serde(default)]
    pub custom: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMeta {
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub schema: Option<FieldSchema>,
    /// Allowed values for select-type fields.
    #[serde(default)]
    pub allowed_values: Option<Vec<Value>>,
}

/// createmeta response for one issue type: which fields exist and their shapes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueTypeMeta {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub subtask: Option<bool>,
    /// Field key (`summary`, `customfield_10001`, ...) → metadata.
    /// BTreeMap keeps the serialized output deterministic.
    #[serde(default)]
    pub fields: Option<std::collections::BTreeMap<String, FieldMeta>>,
}

/// Wrapper for the aggregated createmeta endpoint
/// (`GET /issue/createmeta?...&expand=projects.issuetypes.fields`),
/// the only shape available on Jira 7.x (the flat per-type endpoint is 8.4+).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatemetaWrapper {
    #[serde(default)]
    pub projects: Vec<CreatemetaProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatemetaProject {
    #[serde(default)]
    pub key: Option<String>,
    /// Jira 7.x spells this `issuetypes` (all lowercase); newer versions
    /// use camelCase. Accept both.
    #[serde(default, alias = "issuetypes")]
    pub issue_types: Vec<IssueTypeMeta>,
}

/// Entry of `GET /field`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomField {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub custom: Option<bool>,
    #[serde(default)]
    pub schema: Option<FieldSchema>,
}

/// Response of `POST /issue` (issue created).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedIssue {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub self_url: Option<String>,
}
