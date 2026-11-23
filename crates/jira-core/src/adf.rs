//! ADF (Atlassian Document Format) helpers.
//!
//! MVP: a tolerant loose node model (type-string + children) that round-trips
//! any document, plus a Markdown converter for the common node subset.
//! A strict recursive enum can replace `Node` later without changing callers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mark {
    #[serde(rename = "type")]
    pub mark_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// ADF text nodes carry no `type` field; empty is skipped on serialize.
    #[serde(rename = "type", skip_serializing_if = "String::is_empty", default)]
    pub node_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Value>,
}

impl Node {
    pub fn new(node_type: &str) -> Self {
        Self { node_type: node_type.into(), ..Default::default() }
    }

    pub fn text_of(text: impl Into<String>) -> Self {
        Self { text: Some(text.into()), ..Default::default() }
    }

    fn attr_u32(&self, key: &str) -> Option<u32> {
        self.attrs.as_ref()?.get(key)?.as_u64().map(|v| v as u32)
    }
}

impl Default for Node {
    fn default() -> Self {
        Self {
            node_type: String::new(),
            content: Vec::new(),
            text: None,
            marks: Vec::new(),
            attrs: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub version: u8,
    #[serde(rename = "type")]
    pub doc_type: String,
    pub content: Vec<Node>,
}

impl Document {
    pub fn new(content: Vec<Node>) -> Self {
        Self { version: 1, doc_type: "doc".into(), content }
    }
}

/// Convert Markdown text to an ADF document.
///
/// Supports the common subset: headings, paragraphs, bullet/ordered lists,
/// fenced code blocks, blockquotes, and `---` rules. Inline marks are kept
/// as plain text (conversion of `**bold**` etc. is a later refinement).
pub fn markdown_to_adf(md: &str) -> Document {
    let mut content = Vec::new();
    let mut paragraph: Vec<Node> = Vec::new();
    let mut in_code = false;
    let mut code_lines: Vec<String> = Vec::new();
    let mut list_stack: Vec<(bool /*ordered*/, Vec<Node>)> = Vec::new();

    fn flush_para(paragraph: &mut Vec<Node>, content: &mut Vec<Node>) {
        if !paragraph.is_empty() {
            content.push(Node::new("paragraph").with_content(std::mem::take(paragraph)));
        }
    }
    fn flush_list(list_stack: &mut Vec<(bool, Vec<Node>)>, content: &mut Vec<Node>) {
        while let Some((ordered, items)) = list_stack.pop() {
            let mut list = Node::new(if ordered { "orderedList" } else { "bulletList" });
            list.content = items;
            match list_stack.last_mut() {
                Some((_, parent_items)) => {
                    let mut li = Node::new("listItem");
                    li.content = vec![list];
                    parent_items.push(li);
                }
                None => content.push(list),
            }
        }
    }

    for line in md.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            if in_code {
                let mut code = Node::new("codeBlock");
                code.content = code_lines
                    .iter()
                    .map(|l| Node::text_of(format!("{l}\n")))
                    .collect();
                content.push(code);
                code_lines.clear();
            } else {
                flush_para(&mut paragraph, &mut content);
                flush_list(&mut list_stack, &mut content);
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code_lines.push(line.to_string());
            continue;
        }

        // Heading: # .. ######
        if let Some(rest) = trimmed.strip_prefix('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if rest.starts_with(' ') && (1..=6).contains(&level) {
                flush_para(&mut paragraph, &mut content);
                flush_list(&mut list_stack, &mut content);
                let mut h = Node::new("heading");
                h.attrs = Some(serde_json::json!({ "level": level }));
                h.content = parse_inline(rest.trim());
                content.push(h);
                continue;
            }
        }

        // Rule
        if trimmed == "---" || trimmed == "***" {
            flush_para(&mut paragraph, &mut content);
            flush_list(&mut list_stack, &mut content);
            content.push(Node::new("rule"));
            continue;
        }

        // Blockquote (single level)
        if let Some(rest) = trimmed.strip_prefix("> ") {
            flush_para(&mut paragraph, &mut content);
            let mut bq = Node::new("blockQuote");
            bq.content = vec![Node::new("paragraph").with_content(parse_inline(rest))];
            content.push(bq);
            continue;
        }

        // List item; leading indent decides nesting depth (2 spaces per level).
        let indent = line.len() - trimmed.len();
        if let Some(rest) = trimmed.strip_prefix("- ").or(trimmed.strip_prefix("* ")) {
            flush_para(&mut paragraph, &mut content);
            let depth = indent / 2;
            while list_stack.len() > depth + 1 {
                // close deeper lists into parents
                let (ordered, items) = list_stack.pop().unwrap();
                let mut list = Node::new(if ordered { "orderedList" } else { "bulletList" });
                list.content = items;
                if let Some((_, parent)) = list_stack.last_mut() {
                    let mut li = Node::new("listItem");
                    li.content = vec![list];
                    parent.push(li);
                }
            }
            while list_stack.len() < depth + 1 {
                list_stack.push((false, Vec::new()));
            }
            let item = Node::new("paragraph").with_content(parse_inline(rest));
            list_stack.last_mut().unwrap().1.push(Node::new("listItem").with_content(vec![item]));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("1. ").or(trimmed.strip_prefix("1) ")) {
            flush_para(&mut paragraph, &mut content);
            while list_stack.last().is_some_and(|(o, _)| !*o) {
                let (_, items) = list_stack.pop().unwrap();
                let mut list = Node::new("bulletList");
                list.content = items;
                if let Some((_, parent)) = list_stack.last_mut() {
                    let mut li = Node::new("listItem");
                    li.content = vec![list];
                    parent.push(li);
                } else {
                    content.push(list);
                }
            }
            if list_stack.is_empty() {
                list_stack.push((true, Vec::new()));
            }
            let item = Node::new("paragraph").with_content(parse_inline(rest));
            list_stack.last_mut().unwrap().1.push(Node::new("listItem").with_content(vec![item]));
            continue;
        }

        if trimmed.is_empty() {
            flush_para(&mut paragraph, &mut content);
            flush_list(&mut list_stack, &mut content);
        } else {
            paragraph.extend(parse_inline(trimmed));
        }
    }

    if in_code && !code_lines.is_empty() {
        let mut code = Node::new("codeBlock");
        code.content = code_lines.iter().map(|l| Node::text_of(format!("{l}\n"))).collect();
        content.push(code);
    }
    flush_para(&mut paragraph, &mut content);
    flush_list(&mut list_stack, &mut content);

    Document::new(content)
}

/// Split a Markdown fragment into text nodes with bold/italic/code/strike
/// marks and links. Underscore emphasis is intentionally NOT supported so
/// snake_case identifiers survive verbatim.
fn parse_inline(text: &str) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut plain = String::new();

    fn flush(plain: &mut String, nodes: &mut Vec<Node>) {
        if !plain.is_empty() {
            nodes.push(Node::text_of(std::mem::take(plain)));
        }
    }

    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with('[') {
            // [text](url) link
            if let Some(close) = rest.find("](") {
                let label = &rest[1..close];
                if let Some(end_rel) = rest[close + 2..].find(')') {
                    let url = &rest[close + 2..close + 2 + end_rel];
                    if !label.is_empty() && !url.is_empty() {
                        flush(&mut plain, &mut nodes);
                        let mut node = Node::text_of(label);
                        node.marks.push(Mark {
                            mark_type: "link".into(),
                            attrs: Some(serde_json::json!({ "href": url })),
                        });
                        nodes.push(node);
                        // close/end_rel are offsets relative to `rest`;
                        // advance `i` past the closing ')'.
                        i += close + 2 + end_rel + 1;
                        continue;
                    }
                }
            }
            plain.push('[');
            i += 1;
            continue;
        }
        let (delim, mark): (&str, &str) = if rest.starts_with("**") {
            ("**", "strong")
        } else if rest.starts_with("~~") {
            ("~~", "strike")
        } else if rest.starts_with("`") {
            ("`", "code")
        } else if rest.starts_with("*") {
            ("*", "em")
        } else {
            let c = rest.chars().next().unwrap();
            plain.push(c);
            i += c.len_utf8();
            continue;
        };
        let content_start = i + delim.len();
        if let Some(close_rel) = text[content_start..].find(delim) {
            let inner = &text[content_start..content_start + close_rel];
            if inner.is_empty() {
                plain.push_str(delim);
                i = content_start;
                continue;
            }
            flush(&mut plain, &mut nodes);
            let mut node = Node::text_of(inner);
            node.marks.push(Mark { mark_type: mark.into(), attrs: None });
            nodes.push(node);
            i = content_start + close_rel + delim.len();
        } else {
            // unmatched delimiter: keep it literal
            plain.push_str(delim);
            i += delim.len();
        }
    }
    flush(&mut plain, &mut nodes);
    nodes
}

/// Convert Markdown to an ADF document as a ready-to-send JSON value.
pub fn markdown_to_adf_value(md: &str) -> Value {
    serde_json::to_value(markdown_to_adf(md)).unwrap_or(Value::Null)
}

/// Render a description/comment body as Markdown. Dispatches on shape:
/// string bodies (wiki markup — Jira v2 Server/DC) go through
/// [`crate::wiki::wiki_to_markdown`]; object bodies (ADF — Cloud v3)
/// keep the ADF renderer below.
pub fn to_markdown(doc: &Value) -> String {
    match doc {
        Value::String(s) => crate::wiki::wiki_to_markdown(s),
        Value::Object(_) => match serde_json::from_value::<Document>(doc.clone()) {
            Ok(parsed) => {
                let mut out = String::new();
                for node in &parsed.content {
                    render_block(node, 0, &mut out);
                }
                out
            }
            Err(_) => String::new(),
        },
        _ => String::new(),
    }
}

fn render_block(node: &Node, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match node.node_type.as_str() {
        "heading" => {
            let level = node.attr_u32("level").unwrap_or(1).clamp(1, 6);
            out.push_str(&"#".repeat(level as usize));
            out.push(' ');
            render_inline(node, out);
            out.push_str("\n\n");
        }
        "paragraph" => {
            out.push_str(&indent);
            render_inline(node, out);
            out.push_str("\n\n");
        }
        "codeBlock" => {
            out.push_str(&indent);
            out.push_str("```\n");
            for child in &node.content {
                if let Some(t) = &child.text {
                    out.push_str(t);
                }
            }
            out.push_str("```\n\n");
        }
        "blockQuote" => {
            for child in &node.content {
                let mut inner = String::new();
                render_block(child, 0, &mut inner);
                for line in inner.lines() {
                    out.push_str(&format!("> {line}\n"));
                }
                out.push('\n');
            }
        }
        "bulletList" | "orderedList" => {
            let ordered = node.node_type == "orderedList";
            for (i, item) in node.content.iter().enumerate() {
                let marker = if ordered { format!("{}. ", i + 1) } else { "- ".into() };
                for (j, block) in item.content.iter().enumerate() {
                    if j == 0 {
                        out.push_str(&indent);
                        out.push_str(&marker);
                    } else {
                        out.push_str(&indent);
                        out.push_str(&" ".repeat(marker.len()));
                    }
                    let mut inner = String::new();
                    render_block(block, 0, &mut inner);
                    out.push_str(inner.trim_end_matches('\n'));
                    out.push('\n');
                }
                // nested lists inside this item
                for block in &item.content {
                    if block.node_type == "bulletList" || block.node_type == "orderedList" {
                        render_block(block, depth + 1, out);
                    }
                }
            }
            out.push('\n');
        }
        "media" => {
            out.push_str("[attachment]\n\n");
        }
        "rule" => {
            out.push_str("---\n\n");
        }
        "table" | "tableRow" | "tableCell" | "panel" | "expand" => {
            for child in &node.content {
                render_block(child, depth, out);
            }
        }
        _ => {
            // Unknown block: fall through to inline rendering of text.
            render_inline(node, out);
            out.push_str("\n\n");
        }
    }
}

fn render_inline(node: &Node, out: &mut String) {
    if let Some(text) = &node.text {
        out.push_str(&apply_marks(text, &node.marks));
    }
    for child in &node.content {
        match child.node_type.as_str() {
            "hardBreak" => out.push('\n'),
            "media" => out.push_str("[attachment]"),
            "emoji" => {
                if let Some(short) = child
                    .attrs
                    .as_ref()
                    .and_then(|a| a.get("shortName"))
                    .and_then(|v| v.as_str())
                {
                    out.push_str(short);
                }
            }
            _ => render_inline(child, out),
        }
    }
}

fn apply_marks(text: &str, marks: &[Mark]) -> String {
    let mut s = text.to_string();
    for mark in marks {
        s = match mark.mark_type.as_str() {
            "strong" => format!("**{s}**"),
            "em" => format!("*{s}*"),
            "code" => format!("`{s}`"),
            "strike" => format!("~~{s}~~"),
            "underline" => format!("<u>{s}</u>"),
            "link" => {
                let href = mark
                    .attrs
                    .as_ref()
                    .and_then(|a| a.get("href"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("[{s}]({href})")
            }
            _ => s,
        };
    }
    s
}

impl Node {
    fn with_content(mut self, content: Vec<Node>) -> Self {
        self.content = content;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(md: &str) -> String {
        to_markdown(&serde_json::to_value(markdown_to_adf(md)).unwrap())
    }

    /// Regression: link offsets were treated as absolute, sending the cursor
    /// backwards and looping until OOM. The test must terminate and produce
    /// a link mark.
    #[test]
    fn inline_link_parses_and_terminates() {
        let doc = markdown_to_adf("前缀 [链接](http://x.co) 后缀");
        let para = &doc.content[0];
        assert_eq!(para.node_type, "paragraph");
        let link = para
            .content
            .iter()
            .find(|n| n.marks.iter().any(|m| m.mark_type == "link"))
            .expect("link node");
        assert_eq!(link.text.as_deref(), Some("链接"));
        assert_eq!(
            link.marks[0].attrs.as_ref().unwrap()["href"],
            serde_json::json!("http://x.co")
        );
        // text after the link must survive (cursor advanced past ')')
        assert!(para.content.iter().any(|n| n.text.as_deref() == Some(" 后缀")));
    }

    #[test]
    fn inline_marks_roundtrip() {
        let out = roundtrip("**b** `c` *i* ~~s~~ [l](u)");
        assert!(out.contains("**b**"), "bold: {out}");
        assert!(out.contains("`c`"), "code: {out}");
        assert!(out.contains("*i*"), "em: {out}");
        assert!(out.contains("~~s~~"), "strike: {out}");
        assert!(out.contains("[l](u)"), "link: {out}");
    }

    #[test]
    fn snake_case_survives_verbatim() {
        let out = roundtrip("use my_var_name here");
        assert!(out.contains("my_var_name"), "verbatim: {out}");
    }

    #[test]
    fn headings_and_lists() {
        let doc = markdown_to_adf("# T\n\n- a\n- b\n");
        assert_eq!(doc.content[0].node_type, "heading");
        assert_eq!(doc.content[0].attr_u32("level"), Some(1));
        assert_eq!(doc.content[1].node_type, "bulletList");
        assert_eq!(doc.content[1].content.len(), 2);
    }

    #[test]
    fn unmatched_delimiters_stay_literal() {
        let out = roundtrip("a * b ** c ` d");
        assert_eq!(out.trim(), "a * b ** c ` d");
    }
}
