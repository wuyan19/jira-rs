//! Write-operation request assembly, shared by the CLI and MCP surfaces.
//!
//! Both surfaces only ever build `fields` objects here; `dry_run` handling
//! lives in `jira-app` which returns these bodies without sending.

use serde_json::{json, Map, Value};

use crate::wiki::markdown_to_wiki;

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn project_ref(s: &str) -> Value {
    if is_numeric(s) { json!({ "id": s }) } else { json!({ "key": s }) }
}

fn issuetype_ref(s: &str) -> Value {
    if is_numeric(s) { json!({ "id": s }) } else { json!({ "name": s }) }
}

fn merge_extra(fields: &mut Map<String, Value>, extra: Option<&Value>) {
    if let Some(Value::Object(o)) = extra {
        for (k, v) in o {
            fields.insert(k.clone(), v.clone());
        }
    }
}

/// `fields` for `POST /issue`. `extra` (raw JSON object) merges last and
/// overrides — labels, custom fields by id, assignee, etc.
pub fn build_create_fields(
    project: &str,
    issue_type: &str,
    summary: &str,
    description_md: Option<&str>,
    extra: Option<&Value>,
) -> Value {
    let mut f = Map::new();
    f.insert("project".into(), project_ref(project));
    f.insert("issuetype".into(), issuetype_ref(issue_type));
    f.insert("summary".into(), json!(summary));
    if let Some(md) = description_md.filter(|s| !s.is_empty()) {
        // v2 API expects wiki markup text, not ADF.
        f.insert("description".into(), json!(markdown_to_wiki(md)));
    }
    merge_extra(&mut f, extra);
    Value::Object(f)
}

/// `fields` for `PUT /issue/{key}`. Empty result means "nothing to update".
pub fn build_update_fields(
    summary: Option<&str>,
    description_md: Option<&str>,
    extra: Option<&Value>,
) -> Value {
    let mut f = Map::new();
    if let Some(s) = summary {
        f.insert("summary".into(), json!(s));
    }
    if let Some(md) = description_md {
        f.insert("description".into(), json!(markdown_to_wiki(md)));
    }
    merge_extra(&mut f, extra);
    Value::Object(f)
}

/// `body` for `POST /issue/{key}/comment` (wiki markup text).
pub fn build_comment_body(body_md: &str) -> Value {
    json!({ "body": markdown_to_wiki(body_md) })
}
