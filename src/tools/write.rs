use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::general_purpose::GeneralPurpose;
use base64::engine::{DecodePaddingMode, GeneralPurposeConfig};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::read::handler;
use super::{Ctx, Tool, ToolResult, failed, invalid, params, parent_id, stale, success, with_page};
use crate::format::changed_out;
use crate::schema::{Field, boolean, integer, names, record, text, texts, trimmed};
use crate::wiki::{Page, PageUpdate};

/// LeafWiki's own rule (internal/core/tree/slug_service.go).
const SLUG: &str = "^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$";
const FILE_NAME: &str = r"^[^/\\]+$";

fn slug() -> Field {
    trimmed("slug")
        .pattern(SLUG, "letters and digits, single dashes between them")
        .optional()
        .describe("URL segment; omit to derive it from the title")
}
fn tags() -> Field {
    names("tags")
        .describe("Full tag list, replaces the current one")
        .optional()
}
fn properties() -> Field {
    record("properties")
        .describe("Full property map, replaces the current one")
        .optional()
}

#[derive(Default)]
struct Changes {
    title: Option<String>,
    slug: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
    properties: Option<Map<String, Value>>,
}

/// Saves `page` with the given changes, keeping everything else as it is.
async fn save(ctx: &Ctx, page: &Page, changes: Changes) -> crate::http::Res<Page> {
    let update = PageUpdate {
        version: page.version.clone(),
        title: changes.title.unwrap_or_else(|| page.title.clone()),
        slug: changes.slug.unwrap_or_else(|| page.slug.clone()),
        content: changes.content.or_else(|| page.content.clone()),
        tags: changes
            .tags
            .or_else(|| page.tags.clone())
            .unwrap_or_default(),
        properties: changes
            .properties
            .or_else(|| page.properties.clone())
            .unwrap_or_default(),
    };
    ctx.wiki.update(&page.id, update).await
}

async fn suggest_slug(ctx: &Ctx, title: &str, parent: Option<&str>) -> Result<String, ToolResult> {
    let r = ctx
        .wiki
        .slug(title, parent, None)
        .await
        .map_err(failed(format!("Could not derive a slug from \"{title}\"")))?;
    Ok(r.get("slug")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// Node's base64 decoder: both alphabets, padding optional, a dangling last character ignored.
fn decode_base64(data: &str) -> Option<Vec<u8>> {
    let mut standard: String = data
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    if standard.len() % 4 == 1 {
        standard.pop();
    }
    let lenient = GeneralPurpose::new(
        &STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    lenient.decode(standard).ok()
}

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "write_create_page",
            description: "Create a page, or a section (a folder-like page for children), under a parent, optionally with content, tags and properties; answers with its id, path and version. To create a whole missing path at once, use write_ensure_path.",
            fields: vec![
                params::parent(),
                trimmed("title").nonempty().describe("Page title"),
                slug(),
                params::kind().optional(),
                text("content")
                    .optional()
                    .describe("Markdown; default \"# Title\""),
                tags(),
                properties(),
            ],
            handler: handler!(|ctx, args| {
                let parent = parent_id(ctx, args.opt_str("parent").as_deref()).await?;
                let title = args.str("title");
                let page_slug = match args.opt_str("slug").filter(|s| !s.is_empty()) {
                    Some(s) => s,
                    None => suggest_slug(ctx, &title, parent.as_deref()).await?,
                };
                let kind = args.opt_str("kind").unwrap_or_else(|| "page".into());
                let created = ctx
                    .wiki
                    .create(parent.as_deref(), &title, &page_slug, &kind)
                    .await
                    .map_err(failed(format!("Could not create \"{title}\"")))?;
                let (content, tags, properties) = (
                    args.opt_str("content"),
                    args.opt_strs("tags"),
                    args.opt_map("properties"),
                );
                if content.is_none() && tags.is_none() && properties.is_none() {
                    return success(changed_out(&created));
                }
                // LeafWiki creates empty pages; content, tags and properties are a second, versioned save.
                let saved = save(ctx, &created, Changes { content, tags, properties, ..Changes::default() }).await.map_err(failed(format!(
                    "Created \"{}\" (id {}), but saving its content, tags or properties failed: retry with write_update_page",
                    created.path, created.id
                )))?;
                success(changed_out(&saved))
            }),
        },
        Tool {
            name: "write_ensure_path",
            description: "Make sure a path exists, creating every missing segment; the last one gets the title.",
            fields: vec![
                trimmed("path")
                    .nonempty()
                    .describe("Wiki path like docs/team/onboarding"),
                trimmed("title")
                    .nonempty()
                    .describe("Title of the last segment"),
                params::kind()
                    .optional()
                    .describe("Of the last segment; section: a folder-like page for children"),
            ],
            handler: handler!(|ctx, args| {
                let path = args.str("path");
                let kind = args.opt_str("kind").unwrap_or_else(|| "page".into());
                let page = ctx
                    .wiki
                    .ensure(&path, &args.str("title"), &kind)
                    .await
                    .map_err(failed(format!("Could not create the path \"{path}\"")))?;
                success(changed_out(&page))
            }),
        },
        Tool {
            name: "write_update_page",
            description: "Change a page's title, slug, content, tags or properties; fields you leave out stay as they are.",
            fields: vec![
                params::page(),
                params::version(),
                trimmed("title").nonempty().optional().describe("New title"),
                slug().describe("New URL segment; changes the path, which can break links"),
                text("content")
                    .optional()
                    .describe("Full new Markdown content"),
                tags(),
                properties(),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                stale(&p, &args.str("version"))?;
                let changes = Changes {
                    title: args.opt_str("title"),
                    slug: args.opt_str("slug"),
                    content: args.opt_str("content"),
                    tags: args.opt_strs("tags"),
                    properties: args.opt_map("properties"),
                };
                if changes.title.is_none()
                    && changes.slug.is_none()
                    && changes.content.is_none()
                    && changes.tags.is_none()
                    && changes.properties.is_none()
                {
                    return Err(invalid("nothing to change"));
                }
                let saved = save(ctx, &p, changes)
                    .await
                    .map_err(failed(format!("Could not update \"{}\"", p.path)))?;
                success(changed_out(&saved))
            }),
        },
        Tool {
            name: "write_edit_text",
            description: "Replace one exact passage of a page's content, without sending the whole page.",
            fields: vec![
                params::page(),
                params::version(),
                text("find")
                    .nonempty()
                    .describe("Exact text, occurring once in the page"),
                text("replace").describe("New text; empty removes the passage"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                stale(&p, &args.str("version"))?;
                let (find, replace) = (args.str("find"), args.str("replace"));
                let content = p.text();
                let Some(at) = content.find(&find) else {
                    return Err(invalid(format!(
                        "the text to replace is not in \"{}\"; read the page again and copy it exactly",
                        p.path
                    )));
                };
                // Overlapping matches count too, as with indexOf(find, at + 1).
                let next = at + content[at..].chars().next().map_or(1, char::len_utf8);
                if content[next..].contains(&find) {
                    return Err(invalid(format!(
                        "the text to replace occurs more than once in \"{}\"; include more of the surrounding text",
                        p.path
                    )));
                }
                let edited = format!("{}{replace}{}", &content[..at], &content[at + find.len()..]);
                let saved = save(
                    ctx,
                    &p,
                    Changes {
                        content: Some(edited),
                        ..Changes::default()
                    },
                )
                .await
                .map_err(failed(format!("Could not edit \"{}\"", p.path)))?;
                success(changed_out(&saved))
            }),
        },
        Tool {
            name: "write_append",
            description: "Add Markdown at the end of a page, after a blank line; the rest stays as it is. Needs no version: it appends to the latest one. To change existing text, use write_edit_text.",
            fields: vec![
                params::page(),
                text("text").nonempty().describe("Markdown to add"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let content = p.text();
                let sep = if content.is_empty() || content.ends_with("\n\n") {
                    ""
                } else if content.ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                };
                let appended = format!("{content}{sep}{}", args.str("text"));
                let saved = save(
                    ctx,
                    &p,
                    Changes {
                        content: Some(appended),
                        ..Changes::default()
                    },
                )
                .await
                .map_err(failed(format!("Could not append to \"{}\"", p.path)))?;
                success(changed_out(&saved))
            }),
        },
        Tool {
            name: "write_tags",
            description: "Add or remove tags on a page, keeping its other tags; answers with the new tag list. To replace the whole list, use write_update_page.",
            fields: vec![
                params::page(),
                names("add").optional().describe("Tags to add"),
                names("remove").optional().describe("Tags to remove"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let (add, remove) = (
                    args.opt_strs("add").unwrap_or_default(),
                    args.opt_strs("remove").unwrap_or_default(),
                );
                if add.is_empty() && remove.is_empty() {
                    return Err(invalid("give tags to add or remove"));
                }
                let mut next: Vec<String> = Vec::new();
                for tag in p.tags.iter().flatten().chain(&add) {
                    if !next.contains(tag) && !remove.contains(tag) {
                        next.push(tag.clone());
                    }
                }
                let saved = save(
                    ctx,
                    &p,
                    Changes {
                        tags: Some(next),
                        ..Changes::default()
                    },
                )
                .await
                .map_err(failed(format!(
                    "Could not change the tags of \"{}\"",
                    p.path
                )))?;
                let mut out = changed_out(&saved);
                out["tags"] = json!(saved.tags);
                success(out)
            }),
        },
        Tool {
            name: "write_properties",
            description: "Set or remove properties (key-value page metadata) on a page, keeping the others; answers with the new properties. To replace them all, use write_update_page.",
            fields: vec![
                params::page(),
                record("set").optional().describe("Keys and values to set"),
                texts("remove").optional().describe("Keys to remove"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let (set, remove) = (
                    args.opt_map("set").unwrap_or_default(),
                    args.opt_strs("remove").unwrap_or_default(),
                );
                if set.is_empty() && remove.is_empty() {
                    return Err(invalid("give properties to set or remove"));
                }
                let mut next = p.properties.clone().unwrap_or_default();
                next.extend(set);
                next.retain(|k, _| !remove.contains(k));
                let saved = save(
                    ctx,
                    &p,
                    Changes {
                        properties: Some(next),
                        ..Changes::default()
                    },
                )
                .await
                .map_err(failed(format!(
                    "Could not change the properties of \"{}\"",
                    p.path
                )))?;
                let mut out = changed_out(&saved);
                out["properties"] = json!(saved.properties);
                success(out)
            }),
        },
        Tool {
            name: "write_move",
            description: "Move a page (with its children) under another parent; links to it are not rewritten (see write_refactor).",
            fields: vec![
                params::page(),
                trimmed("parent")
                    .nonempty()
                    .describe("New parent id or path; \"/\" for the top level"),
                integer("position", Some(0), None)
                    .optional()
                    .describe("Index among the new siblings; default last"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let target = parent_id(ctx, Some(&args.str("parent"))).await?;
                ctx.wiki
                    .move_page(
                        &p.id,
                        &p.version,
                        target.as_deref().unwrap_or("root"),
                        args.opt_int("position"),
                    )
                    .await
                    .map_err(failed(format!("Could not move \"{}\"", p.path)))?;
                match ctx.wiki.page(&p.id).await {
                    Ok(moved) => success(changed_out(&moved)),
                    Err(_) => success(json!({ "id": p.id, "moved": true })),
                }
            }),
        },
        Tool {
            name: "write_sort",
            description: "Reorder the children of a page, or the top level; answers with the ids in their new order. Pages keep their parent: to move one elsewhere, use write_move.",
            fields: vec![
                trimmed("parent")
                    .nonempty()
                    .describe("Parent id or path; \"/\" for the top level"),
                names("order")
                    .nonempty()
                    .describe("Every child id or path, in the new order"),
            ],
            handler: handler!(|ctx, args| {
                let parent = args.str("parent");
                let target = parent_id(ctx, Some(&parent)).await?;
                let mut ids = Vec::new();
                for reference in args.opt_strs("order").unwrap_or_default() {
                    let child = ctx
                        .wiki
                        .resolve(&reference)
                        .await
                        .map_err(failed(format!("Child \"{reference}\" not found")))?;
                    ids.push(child.id);
                }
                ctx.wiki
                    .sort(target.as_deref().unwrap_or("root"), &ids)
                    .await
                    .map_err(failed(format!(
                        "Could not sort the children of \"{parent}\""
                    )))?;
                success(json!({ "sorted": ids }))
            }),
        },
        Tool {
            name: "write_pin",
            description: "Pin a page so it stays above its unpinned siblings in the tree, or unpin it; answers with the page.",
            fields: vec![
                params::page(),
                boolean("pinned").describe("true pins, false unpins"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let pinned = args.opt_bool("pinned").unwrap_or(false);
                let verb = if pinned { "pin" } else { "unpin" };
                let r = ctx
                    .wiki
                    .pin(&p.id, &p.version, pinned)
                    .await
                    .map_err(failed(format!("Could not {verb} \"{}\"", p.path)))?;
                success(changed_out(&r))
            }),
        },
        Tool {
            name: "write_favorite",
            description: "Add a page to the signed-in user's favorites, or remove it; only that user sees them. List them with read_favorites.",
            fields: vec![
                params::page(),
                boolean("favorite").describe("true adds, false removes"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let favorite = args.opt_bool("favorite").unwrap_or(false);
                ctx.wiki
                    .favorite(&p.id, favorite)
                    .await
                    .map_err(failed(format!(
                        "Could not change the favorite \"{}\"",
                        p.path
                    )))?;
                success(json!({ "id": p.id, "path": p.path, "favorite": favorite }))
            }),
        },
        Tool {
            name: "write_convert",
            description: "Turn an existing page into a section (a folder-like page that holds children) or a section into a page; answers with the page. Use it when a page should get children; to create a new section, use write_create_page with kind section. A section that still has children stays one: move them out first with write_move.",
            fields: vec![params::page(), params::kind()],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let kind = args.str("kind");
                if p.kind == kind {
                    return Err(invalid(format!("\"{}\" is already a {kind}", p.path)));
                }
                ctx.wiki
                    .convert(&p.id, &p.version, &kind)
                    .await
                    .map_err(failed(format!(
                        "Could not convert \"{}\" to a {kind}",
                        p.path
                    )))?;
                match ctx.wiki.page(&p.id).await {
                    Ok(after) => success(changed_out(&after)),
                    Err(_) => success(json!({ "id": p.id, "kind": kind })),
                }
            }),
        },
        Tool {
            name: "write_copy",
            description: "Copy a page's content, tags, properties and files to a new page with a new title under a parent; the original stays. Answers with the copy's id and path.",
            fields: vec![
                params::page(),
                params::parent(),
                trimmed("title").nonempty().describe("Title of the copy"),
                slug(),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let target = parent_id(ctx, args.opt_str("parent").as_deref()).await?;
                let title = args.str("title");
                let copy_slug = match args.opt_str("slug").filter(|s| !s.is_empty()) {
                    Some(s) => s,
                    None => suggest_slug(ctx, &title, target.as_deref()).await?,
                };
                let r = ctx
                    .wiki
                    .copy(&p.id, target.as_deref(), &title, &copy_slug)
                    .await
                    .map_err(failed(format!("Could not copy \"{}\"", p.path)))?;
                success(changed_out(&r))
            }),
        },
        Tool {
            name: "write_refactor",
            description: "Rename or move a page and rewrite the links that point to it (needs link refactoring on; preview with read_refactor_preview).",
            fields: vec![
                params::page(),
                params::version(),
                trimmed("title")
                    .nonempty()
                    .optional()
                    .describe("New title (rename)"),
                slug().describe("New slug (rename)"),
                params::parent().describe("New parent id or path (move); \"/\" for the top level"),
                integer("position", Some(0), None)
                    .optional()
                    .describe("Index among the new siblings; default last"),
                boolean("rewriteLinks").optional().describe("Default true"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                stale(&p, &args.str("version"))?;
                let parent = args.opt_str("parent");
                let target = parent_id(ctx, parent.as_deref()).await?;
                let moving = parent.is_some();
                let (title, new_slug) = (args.opt_str("title"), args.opt_str("slug"));
                if !moving && title.is_none() && new_slug.is_none() {
                    return Err(invalid("give a new title, slug or parent"));
                }
                let mut input = json!({
                    "version": p.version,
                    "kind": if moving { "move" } else { "rename" },
                    "title": title.unwrap_or_else(|| p.title.clone()),
                    "slug": new_slug.unwrap_or_else(|| p.slug.clone()),
                });
                if moving {
                    input["parentId"] = json!(target.as_deref().unwrap_or("root"));
                }
                input["rewriteLinks"] = json!(args.opt_bool("rewriteLinks") != Some(false));
                if let Some(position) = args.opt_int("position") {
                    input["position"] = json!(position);
                }
                let verb = if moving { "move" } else { "rename" };
                let r = ctx
                    .wiki
                    .refactor_apply(&p.id, input)
                    .await
                    .map_err(failed(format!(
                        "Could not {verb} \"{}\" (is link refactoring on? see read_wiki)",
                        p.path
                    )))?;
                success(changed_out(&r))
            }),
        },
        Tool {
            name: "write_restore_revision",
            description: "Bring a page back to a past revision (the current state stays in the history).",
            fields: vec![
                params::page(),
                trimmed("revision")
                    .nonempty()
                    .describe("Revision id from read_revisions"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let revision = args.str("revision");
                let r = ctx
                    .wiki
                    .restore(&p.id, &revision)
                    .await
                    .map_err(failed(format!(
                        "Could not restore revision {revision} of \"{}\"",
                        p.path
                    )))?;
                success(changed_out(&r))
            }),
        },
        Tool {
            name: "write_upload_asset",
            description: "Attach a file to a page from base64 data; answers with the Markdown link to embed it.",
            fields: vec![
                params::page(),
                trimmed("filename")
                    .nonempty()
                    .pattern(FILE_NAME, "a file name, not a path")
                    .describe("Name to store it under, e.g. diagram.png"),
                text("base64")
                    .nonempty()
                    .describe("File content as base64; local paths are not read"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let filename = args.str("filename");
                // Check first so a wrong file is never stored.
                let data_url = Regex::new(r"^data:[^,]*;base64,").expect("valid regex");
                let spaces = Regex::new(r"\s+").expect("valid regex");
                let raw = args.str("base64");
                let data = spaces
                    .replace_all(&data_url.replace(&raw, ""), "")
                    .into_owned();
                let shape = Regex::new(r"^[A-Za-z0-9+/_-]+={0,2}$").expect("valid regex");
                let bytes = if shape.is_match(&data) {
                    decode_base64(&data)
                } else {
                    None
                };
                let Some(bytes) = bytes else {
                    return Err(invalid(
                        "base64 is not base64 data: send only the encoded bytes",
                    ));
                };
                let r = ctx
                    .wiki
                    .upload(&p.id, &filename, bytes)
                    .await
                    .map_err(failed(format!(
                        "Could not attach \"{filename}\" to \"{}\"",
                        p.path
                    )))?;
                let file = r
                    .get("file")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let image = Regex::new(r"(?i)\.(png|jpe?g|gif|webp|svg)$").expect("valid regex");
                let embed = if image.is_match(&filename) {
                    format!("![{filename}]({file})")
                } else {
                    format!("[{filename}]({file})")
                };
                success(
                    json!({ "file": file, "url": ctx.wiki.session.absolute(&file), "markdown": embed }),
                )
            }),
        },
        Tool {
            name: "write_rename_asset",
            description: "Rename a file attached to a page; answers with its new path and URL. Links to the old name in page content are not updated: fix them with write_edit_text.",
            fields: vec![
                params::page(),
                trimmed("from")
                    .nonempty()
                    .describe("Current file name, as read_assets lists it"),
                trimmed("to")
                    .nonempty()
                    .pattern(FILE_NAME, "a file name, not a path")
                    .describe("New file name"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let from = args.str("from");
                let r = ctx
                    .wiki
                    .rename_asset(&p.id, &from, &args.str("to"))
                    .await
                    .map_err(failed(format!(
                        "Could not rename \"{from}\" on \"{}\"",
                        p.path
                    )))?;
                let url = r
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                success(
                    json!({ "file": url, "url": ctx.wiki.session.absolute(&url), "note": "Links to the old name are not updated." }),
                )
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::decode_base64;

    #[test]
    fn decodes_base64_the_way_node_does() {
        assert_eq!(decode_base64("aGk=").unwrap(), b"hi");
        assert_eq!(decode_base64("aGk").unwrap(), b"hi");
        assert_eq!(decode_base64("-_8").unwrap(), [0xfb, 0xff]);
        assert_eq!(decode_base64("aGkx_").unwrap(), b"hi1");
    }
}
