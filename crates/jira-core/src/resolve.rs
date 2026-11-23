//! Name → id resolution helpers (issue types, transitions) so callers can
//! accept human-friendly names while the API wants ids.

use crate::client::JiraClient;
use crate::error::{Error, Result};

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Resolve an issue type name (case-insensitive) to its id within a project.
/// Numeric input is passed through as an id.
pub async fn resolve_issue_type_id(c: &JiraClient, project: &str, name_or_id: &str) -> Result<String> {
    if is_numeric(name_or_id) {
        return Ok(name_or_id.to_string());
    }
    let p = c.get_project(project).await?;
    p.issue_types
        .iter()
        .find(|t| {
            t.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name_or_id))
        })
        .and_then(|t| t.id.clone())
        .ok_or_else(|| {
            let available: Vec<String> =
                p.issue_types.iter().filter_map(|t| t.name.clone()).collect();
            Error::NotFound(format!(
                "project {project} has no issue type '{name_or_id}' (available: {})",
                available.join(", ")
            ))
        })
}

/// Resolve a transition id-or-name to `(id, name)` for an issue. The error
/// lists available transitions so agents can self-correct.
pub async fn resolve_transition_id(c: &JiraClient, key: &str, id_or_name: &str) -> Result<(String, String)> {
    let w = c.get_transitions(key).await?;
    for t in &w.transitions {
        let id_match = t.id.as_deref() == Some(id_or_name);
        let name_match = t
            .name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(id_or_name));
        if id_match || name_match {
            return Ok((
                t.id.clone().unwrap_or_default(),
                t.name.clone().unwrap_or_default(),
            ));
        }
    }
    let available: Vec<String> = w
        .transitions
        .iter()
        .filter_map(|t| t.name.clone())
        .collect();
    Err(Error::NotFound(format!(
        "issue {key} has no transition '{id_or_name}' (available: {})",
        available.join(", ")
    )))
}
