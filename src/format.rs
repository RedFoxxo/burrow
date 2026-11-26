//! Answers stay short: compact JSON without empty fields, the tree as indented lines, search
//! excerpts without `<b>`, revision diffs as unified lines.

use serde_json::{Map, Value, json};

use crate::wiki::Page;

/// `2026-11-19T14:54:39Z` → `2026-11-19 14:54`; missing or empty → nothing.
pub fn day(iso: Option<&str>) -> Value {
    match iso {
        Some(s) if !s.is_empty() => {
            Value::String(s.replacen('T', " ", 1).chars().take(16).collect())
        }
        _ => Value::Null,
    }
}

/// Search answers mark hits with `<b>`; the AI gets plain text.
pub fn plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    // `<[^>]+>`: a tag needs at least one character between the brackets.
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        match after.find('>') {
            Some(close) if close > 0 => {
                out.push_str(&rest[..open]);
                rest = &after[close + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Drops nulls, `false`, empty arrays and empty objects, so answers stay short.
pub fn compact<'a>(fields: impl IntoIterator<Item = (&'a str, Value)>) -> Value {
    let mut out = Map::new();
    for (key, value) in fields {
        let empty = match &value {
            Value::Null | Value::Bool(false) => true,
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            _ => false,
        };
        if !empty {
            out.insert(key.to_string(), value);
        }
    }
    Value::Object(out)
}

/// A field of a LeafWiki answer, or null when it is missing.
pub fn field(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

pub fn opt<T: Into<Value>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, Into::into)
}

/// A page as the AI sees it: what it needs to read, link to and change it.
pub fn page_out(page: &Page, with_content: bool) -> Value {
    let m = &page.metadata;
    compact([
        ("id", json!(page.id)),
        ("path", json!(page.path)),
        ("title", json!(page.title)),
        ("kind", json!(page.kind)),
        ("version", json!(page.version)),
        ("pinned", json!(page.pinned)),
        ("tags", opt(page.tags.clone())),
        ("properties", opt(page.properties.clone())),
        ("updated", day(m.updated_at.as_deref())),
        (
            "updatedBy",
            opt(m.last_author.as_ref().and_then(|u| u.username.clone())),
        ),
        ("created", day(m.created_at.as_deref())),
        (
            "createdBy",
            opt(m.creator.as_ref().and_then(|u| u.username.clone())),
        ),
        (
            "content",
            if with_content {
                opt(page.content.clone())
            } else {
                Value::Null
            },
        ),
    ])
}

/// A page after a change: enough to keep going without reading it again.
pub fn changed_out(page: &Page) -> Value {
    compact([
        ("id", json!(page.id)),
        ("path", json!(page.path)),
        ("title", json!(page.title)),
        ("kind", json!(page.kind)),
        ("version", json!(page.version)),
        ("pinned", json!(page.pinned)),
    ])
}

/// The tree as indented lines, one node each: `- Title (path) [section, pinned] id=…`.
/// `max_depth` counts levels below `node`; deeper children are summarized.
pub fn tree_lines(node: &Page, max_depth: usize) -> String {
    fn walk(n: &Page, depth: usize, max_depth: usize, lines: &mut Vec<String>) {
        let flags: Vec<&str> = [(n.kind == "section", "section"), (n.pinned, "pinned")]
            .into_iter()
            .filter_map(|(on, flag)| on.then_some(flag))
            .collect();
        let flags = if flags.is_empty() {
            String::new()
        } else {
            format!("[{}] ", flags.join(", "))
        };
        let mut line = format!(
            "{}- {} ({}) {flags}id={}",
            "  ".repeat(depth),
            n.title,
            n.path,
            n.id
        );
        if depth + 1 > max_depth && !n.children.is_empty() {
            line.push_str(&format!(" +{} below", n.children.len()));
        }
        lines.push(line);
        if depth < max_depth {
            for c in &n.children {
                walk(c, depth + 1, max_depth, lines);
            }
        }
    }
    let mut lines = Vec::new();
    if node.id == "root" {
        for c in &node.children {
            walk(c, 0, max_depth, &mut lines);
        }
    } else {
        walk(node, 0, max_depth, &mut lines);
    }
    if lines.is_empty() {
        "(empty)".into()
    } else {
        lines.join("\n")
    }
}

/// Finds a node by id in a tree.
pub fn find_node<'a>(node: &'a Page, id: &str) -> Option<&'a Page> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find_node(c, id))
}

/// A revision as the AI sees it.
pub fn revision_out(r: &Value) -> Value {
    let by = r
        .get("author")
        .and_then(|a| a.get("username"))
        .filter(|u| !u.is_null())
        .cloned()
        .unwrap_or_else(|| field(r, "authorId"));
    compact([
        ("id", field(r, "id")),
        ("at", day(r.get("createdAt").and_then(Value::as_str))),
        ("by", by),
        ("type", field(r, "type")),
        ("title", field(r, "title")),
        ("path", field(r, "path")),
        ("summary", field(r, "summary")),
    ])
}

const MAX_DIFF_CELLS: usize = 4_000_000;

/// Unified line diff of `before` → `after` with `context` unchanged lines around each change.
/// None when the texts are too large to compare here.
pub fn line_diff(before: &str, after: &str, context: usize) -> Option<String> {
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();
    if a.len() * b.len() > MAX_DIFF_CELLS {
        return None;
    }
    // LCS table, from the end, so the walk below goes forward.
    let w = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * w];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * w + j] = if a[i] == b[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &str)> = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            ops.push(('-', a[i]));
            i += 1;
        } else {
            ops.push(('+', b[j]));
            j += 1;
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut gap = false;
    for (k, (t, s)) in ops.iter().enumerate() {
        let near_change = ops[k.saturating_sub(context)..(k + context + 1).min(ops.len())]
            .iter()
            .any(|(t, _)| *t != ' ');
        if !near_change {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push("…".into());
        }
        gap = false;
        out.push(format!("{t}{s}"));
    }
    Some(if out.is_empty() {
        "(no changes)".into()
    } else {
        out.join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_diff_shows_changed_lines_with_context_and_elides_the_rest() {
        let before = ["a", "b", "c", "d", "e", "f", "g", "h"].join("\n");
        let after = ["a", "b", "c", "D", "e", "f", "g", "h", "i"].join("\n");
        assert_eq!(
            line_diff(&before, &after, 1).unwrap(),
            [" c", "-d", "+D", " e", "…", " h", "+i"].join("\n")
        );
        assert_eq!(line_diff("same", "same", 2).unwrap(), "(no changes)");
    }

    #[test]
    fn plain_strips_tags_and_entities() {
        assert_eq!(
            plain("<b>Deploy</b> &lt;x&gt; &amp;amp; a <> b"),
            "Deploy <x> &amp; a <> b"
        );
    }

    #[test]
    fn compact_drops_empty_values_but_keeps_zero_and_empty_text() {
        let v = compact([
            ("a", Value::Null),
            ("b", json!(false)),
            ("c", json!([])),
            ("d", json!({})),
            ("e", json!(0)),
            ("f", json!("")),
        ]);
        assert_eq!(v, json!({ "e": 0, "f": "" }));
    }

    #[test]
    fn day_keeps_date_and_minutes() {
        assert_eq!(day(Some("2026-11-19T14:54:39Z")), json!("2026-11-19 14:54"));
        assert_eq!(day(Some("")), Value::Null);
    }
}
