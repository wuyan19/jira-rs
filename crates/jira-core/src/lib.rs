//! Typed async Jira REST library powering the `jira` binary (CLI + MCP).
//!
//! Targets REST API v2 first (works on both Server/DC and legacy Cloud);
//! Cloud v3 / enhanced search endpoints are tracked for the next phase.

mod adf;
mod auth;
mod client;
mod config;
mod error;
mod resolve;
mod types;
mod wiki;
mod write;

pub use adf::{markdown_to_adf, markdown_to_adf_value, to_markdown, Document, Mark, Node};
pub use auth::Auth;
pub use client::JiraClient;
pub use config::{ConfigSource, JiraConfig, StoredConfig};
pub use error::{Error, Result};
pub use resolve::{resolve_issue_type_id, resolve_transition_id};
pub use types::{
    AttachmentMeta, CommentsPage, CreatemetaProject, CreatemetaWrapper, CreatedIssue, CustomField,
    FieldMeta, FieldSchema, Issue, IssueFields, IssueTypeMeta, IssueTypeRef, NamedValue,
    ProjectConfig, SearchResults, ServerInfo, Transition, TransitionsWrapper, UserInfo,
};
pub use wiki::{markdown_to_wiki, wiki_to_markdown};
pub use write::{build_comment_body, build_create_fields, build_update_fields};
