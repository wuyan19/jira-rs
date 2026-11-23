//! Wiki markup ↔ Markdown bridge for the Jira v2 REST API.
//!
//! Server/DC (and legacy Cloud) v2 stores description/comment bodies as
//! wiki markup strings; ADF is a Cloud v3 concept. All writes send wiki
//! markup via [`markdown_to_wiki`]; reads dispatch in
//! [`crate::adf::to_markdown`] — string bodies come here, object (ADF)
//! bodies keep the ADF renderer.
//!
//! Conversion covers the common constructs; anything unrecognized passes
//! through verbatim rather than being mangled.

use std::sync::OnceLock;

use regex::Regex;

fn re_attach() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!([^\s!|]+)(\|[^!]*)?!").unwrap())
}

fn re_link() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[([^\]|]+)\|([^\]]+)\]").unwrap())
}

fn re_code_block() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{code(:[^}]*)?\}").unwrap())
}

fn re_inline_code() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{([^}]+)\}\}").unwrap())
}

/// wiki `*bold*` → md `**bold**`; requires a non-space right after the
/// opening star so `* item` list bullets stay untouched.
fn re_wiki_bold() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*([^*\s][^*]*?)\*").unwrap())
}

fn re_md_bold() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*\*([^*]+)\*\*").unwrap())
}

fn re_md_link() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap())
}

fn re_md_heading() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.*)$").unwrap())
}

fn re_wiki_heading() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^h([1-6])\.\s+(.*)$").unwrap())
}

/// Render a wiki markup body as Markdown.
pub fn wiki_to_markdown(s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    let s = re_attach().replace_all(&s, "[attachment:$1]");
    let s = re_link().replace_all(&s, "[$1]($2)");
    // inline `{{x}}` must run BEFORE block `{code}`: the latter's pattern
    // matches the inner `{code}` of `{{code}}` and mangles it.
    let s = re_inline_code().replace_all(&s, "`$1`");
    let s = re_code_block().replace_all(&s, "```");
    let s = re_wiki_bold().replace_all(&s, "**$1**");
    // headings line by line (must stay anchored)
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if let Some(c) = re_wiki_heading().captures(line) {
            let level: usize = c[1].parse().unwrap_or(1);
            out.push_str(&format!("{} {}", "#".repeat(level), &c[2]));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Convert Markdown to the wiki markup Jira v2 expects on writes.
pub fn markdown_to_wiki(md: &str) -> String {
    let md = md.replace("\r\n", "\n");
    let mut out = String::with_capacity(md.len());
    let mut in_code = false;
    for line in md.lines() {
        if line.starts_with("```") {
            if in_code {
                out.push_str("{code}");
            } else {
                let lang = line.trim_start_matches('`').trim();
                if lang.is_empty() {
                    out.push_str("{code}");
                } else {
                    out.push_str(&format!("{{code:{lang}}}"));
                }
            }
            in_code = !in_code;
            out.push('\n');
            continue;
        }
        if in_code {
            // verbatim inside code blocks
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let line = re_md_heading().replace(line, |c: &regex::Captures| {
            format!("h{}. {}", c[1].len(), &c[2])
        });
        let line = re_md_link().replace_all(&line, "[$1|$2]");
        let line = re_inline_md_code(&line);
        let line = re_md_bold().replace_all(&line, "*$1*");
        out.push_str(&line);
        out.push('\n');
    }
    // drop the trailing newline of the last line unless the input ended with one
    if !md.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn re_inline_md_code_md() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"`([^`]+)`").unwrap())
}

fn re_inline_md_code(s: &str) -> String {
    re_inline_md_code_md().replace_all(s, "{{$1}}").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_to_md_images_links_code() {
        let out = wiki_to_markdown(" !image.png|thumbnail! and [百度|https://baidu.com] done");
        assert!(out.contains("[attachment:image.png]"), "{out}");
        assert!(out.contains("[百度](https://baidu.com)"), "{out}");
    }

    #[test]
    fn wiki_to_md_bold_but_not_bullets() {
        assert_eq!(wiki_to_markdown("this is *bold* ok"), "this is **bold** ok");
        // list bullets keep working
        assert_eq!(wiki_to_markdown("* item one\n* item two"), "* item one\n* item two");
    }

    #[test]
    fn wiki_to_md_headings_and_code() {
        let out = wiki_to_markdown("h2. Title\r\n{code:java}\nx=1{code}\n{{inline}}");
        assert!(out.contains("## Title"), "{out}");
        assert!(out.contains("```\nx=1```"), "{out}");
        assert!(out.contains("`inline`"), "{out}");
    }

    #[test]
    fn md_to_wiki_roundset() {
        let out = markdown_to_wiki("# T\n\n**b** and `c` and [l](http://u.co)");
        assert!(out.contains("h1. T"), "{out}");
        assert!(out.contains("*b*"), "{out}");
        assert!(out.contains("{{c}}"), "{out}");
        assert!(out.contains("[l|http://u.co]"), "{out}");
    }

    #[test]
    fn md_to_wiki_fenced_code() {
        let out = markdown_to_wiki("```rust\nfn x() {}\n```");
        assert_eq!(out, "{code:rust}\nfn x() {}\n{code}");
    }

    #[test]
    fn inline_code_not_eaten_by_code_block() {
        // regression: re_code_block used to run first and matched the inner
        // `{code}` of `{{code}}`, producing `{```}`.
        assert_eq!(wiki_to_markdown("a {{code}} b"), "a `code` b");
        assert_eq!(wiki_to_markdown("{code}
x{code}"), "```
x```");
    }

    #[test]
    fn bullet_lists_pass_through() {
        assert_eq!(markdown_to_wiki("- a\n- b"), "- a\n- b");
    }
}
