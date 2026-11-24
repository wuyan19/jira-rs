//! Shared operations used by both the CLI and MCP surfaces.
//!
//! `dry_run` is implemented here: write operations build the exact request
//! body, then either return it (dry run) or send it. Both surfaces get
//! identical behavior for free.

use jira_core::{
    build_create_fields, build_update_fields, markdown_to_wiki, resolve_issue_type_id,
    resolve_transition_id, to_markdown, Error, JiraClient,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

pub async fn search(c: &JiraClient, jql: &str, start_at: Option<u32>, max_results: u32) -> Result<Value, Error> {
    Ok(json!(c.search(jql, start_at, max_results).await?))
}

pub async fn get_issue(c: &JiraClient, key: &str) -> Result<Value, Error> {
    let issue = c.get_issue(key).await?;
    let desc = issue.description_markdown();
    Ok(json!({ "issue": issue, "descriptionMarkdown": desc }))
}

pub async fn comments(c: &JiraClient, key: &str, start_at: Option<u32>, max_results: Option<u32>) -> Result<Value, Error> {
    let page = c.get_comments(key, start_at, max_results).await?;
    let items: Vec<Value> = page
        .comments
        .iter()
        .map(|cm| {
            json!({
                "id": cm.id,
                "author": cm.author.as_ref().and_then(|a| a.display_name.clone()),
                "created": cm.created,
                "updated": cm.updated,
                "bodyMarkdown": cm.body.as_ref().map(to_markdown),
            })
        })
        .collect();
    Ok(json!({
        "comments": items,
        "startAt": page.start_at,
        "maxResults": page.max_results,
        "total": page.total,
    }))
}

pub async fn transitions(c: &JiraClient, key: &str) -> Result<Value, Error> {
    Ok(json!(c.get_transitions(key).await?))
}

pub async fn project_config(c: &JiraClient, project: &str) -> Result<Value, Error> {
    Ok(json!(c.get_project(project).await?))
}

pub async fn issue_type_metadata(c: &JiraClient, project: &str, issue_type: &str) -> Result<Value, Error> {
    let id = resolve_issue_type_id(c, project, issue_type).await?;
    let meta = c.get_issue_type_meta(project, &id).await?;
    Ok(json!({ "project": project, "issueTypeId": id, "metadata": meta }))
}

pub async fn custom_fields(c: &JiraClient) -> Result<Value, Error> {
    Ok(json!(c.get_fields().await?))
}

// ---------------------------------------------------------------------------
// Writes (dry_run-aware)
// ---------------------------------------------------------------------------

pub async fn create_issue(
    c: &JiraClient,
    base_url: &str,
    project: &str,
    issue_type: &str,
    summary: &str,
    description_md: Option<&str>,
    fields: Option<&Value>,
    dry_run: bool,
) -> Result<Value, Error> {
    let f = build_create_fields(project, issue_type, summary, description_md, fields);
    if dry_run {
        return Ok(json!({ "dryRun": true, "method": "POST", "path": "/rest/api/2/issue", "body": { "fields": f } }));
    }
    let created = c.create_issue(f).await?;
    let key = created.key.clone().unwrap_or_default();
    Ok(json!({
        "created": true,
        "key": key,
        "id": created.id,
        "self": created.self_url,
        "url": format!("{base_url}/browse/{key}"),
    }))
}

pub async fn update_issue(
    c: &JiraClient,
    key: &str,
    summary: Option<&str>,
    description_md: Option<&str>,
    fields: Option<&Value>,
    dry_run: bool,
) -> Result<Value, Error> {
    let f = build_update_fields(summary, description_md, fields);
    if f.as_object().is_none_or(|o| o.is_empty()) {
        return Err(Error::Parse("nothing to update: provide summary / description-md / fields".into()));
    }
    if dry_run {
        return Ok(json!({ "dryRun": true, "method": "PUT", "path": format!("/rest/api/2/issue/{key}"), "body": { "fields": f } }));
    }
    c.update_issue(key, f).await?;
    Ok(json!({ "updated": true, "key": key }))
}

pub async fn transition_issue(
    c: &JiraClient,
    key: &str,
    id_or_name: &str,
    comment_md: Option<&str>,
    dry_run: bool,
) -> Result<Value, Error> {
    let (id, name) = resolve_transition_id(c, key, id_or_name).await?;
    let comment_text = comment_md
        .filter(|s| !s.is_empty())
        .map(markdown_to_wiki);
    if dry_run {
        return Ok(json!({
            "dryRun": true,
            "method": "POST",
            "path": format!("/rest/api/2/issue/{key}/transitions"),
            "body": { "transition": { "id": id, "name": name }, "commentMarkdown": comment_md },
        }));
    }
    c.transition_issue(key, &id, comment_text).await?;
    Ok(json!({ "transitioned": true, "key": key, "transition": name }))
}

pub async fn add_comment(c: &JiraClient, key: &str, body_md: &str, dry_run: bool) -> Result<Value, Error> {
    let text = markdown_to_wiki(body_md);
    if dry_run {
        return Ok(json!({ "dryRun": true, "method": "POST", "path": format!("/rest/api/2/issue/{key}/comment"), "body": { "body": text } }));
    }
    let cm = c.add_comment(key, text).await?;
    Ok(json!({ "added": true, "key": key, "commentId": cm.id }))
}

pub async fn attach_file(c: &JiraClient, key: &str, file_path: &str) -> Result<Value, Error> {
    let att = c.attach_file(key, file_path).await?;
    Ok(json!({ "attached": true, "key": key, "attachment": att }))
}

pub async fn download_attachment(c: &JiraClient, key: &str, attachment_id: &str, dest_dir: &str) -> Result<Value, Error> {
    let path = c.download_attachment(key, attachment_id, dest_dir).await?;
    Ok(json!({ "downloaded": true, "key": key, "attachmentId": attachment_id, "savedTo": path }))
}
