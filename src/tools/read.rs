use serde_json::{Map, Value, json};

use super::{Tool, failed, failure, params, parent_id, success, with_page};
use crate::format::{
    changed_out, compact, day, field, find_node, line_diff, page_out, plain, revision_out,
    tree_lines,
};
use crate::schema::{boolean, integer, names, text, trimmed};
use crate::wiki::Page;

/// Declares a handler: `handler!(|ctx, args| { … })` with `?` for early answers.
macro_rules! handler {
    (|$ctx:ident, $args:ident| $body:block) => {{
        #[allow(unused_variables)]
        fn handle<'a>($ctx: &'a $crate::tools::Ctx, $args: $crate::schema::Args) -> $crate::http::BoxFuture<'a, $crate::tools::Answer> {
            Box::pin(async move $body)
        }
        handle
    }};
}
pub(super) use handler;

fn limit(name: &'static str, max: i64, description: &'static str) -> crate::schema::Field {
    integer(name, Some(1), Some(max))
        .optional()
        .describe(description)
}

/// JavaScript truthiness, for LeafWiki's flags.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// A list LeafWiki may send as `null`.
fn list(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

/// Copies the fields that are present, under new names; JSON.stringify leaves out missing ones.
fn pick(value: &Value, fields: &[(&str, &str)]) -> Value {
    let mut out = Map::new();
    for (to, from) in fields {
        if let Some(v) = value.get(*from) {
            out.insert((*to).into(), v.clone());
        }
    }
    Value::Object(out)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "read_wiki",
            description: "Who is signed in (username, role) and which wiki features are on (revisions, link refactoring, upload size).",
            fields: vec![],
            handler: handler!(|ctx, args| {
                let (config, me) = tokio::join!(ctx.wiki.config(), ctx.wiki.me());
                let c = config.map_err(failed("Could not read the wiki settings"))?;
                let me = me.ok().filter(|m| !m.is_null());
                let user = match &me {
                    Some(m) => field(m, "username"),
                    None if truthy(&field(&c, "authDisabled")) => {
                        json!("(no sign-in on this wiki)")
                    }
                    None => Value::Null,
                };
                let max_upload = c
                    .get("maxAssetUploadSizeBytes")
                    .and_then(Value::as_f64)
                    .filter(|b| *b != 0.0)
                    .map(|b| json!((b / 1_048_576.0).round() as i64))
                    .unwrap_or(Value::Null);
                let mut out = compact([
                    ("url", json!(ctx.wiki.session.base_url)),
                    ("user", user),
                    (
                        "role",
                        me.as_ref().map(|m| field(m, "role")).unwrap_or(Value::Null),
                    ),
                    ("maxUploadMB", max_upload),
                ]);
                // Features stay in the answer when off: a missing key would read as "unknown".
                out["revisions"] = json!(truthy(&field(&c, "enableRevision")));
                out["linkRefactor"] = json!(truthy(&field(&c, "enableLinkRefactor")));
                out["publicAccess"] = json!(truthy(&field(&c, "publicAccess")));
                out["readOnly"] = json!(ctx.config.read_only);
                success(out)
            }),
        },
        Tool {
            name: "read_tree",
            description: "Page hierarchy as indented lines with path and id, for the whole wiki or below one page.",
            fields: vec![
                trimmed("under")
                    .optional()
                    .describe("Page id or path to list below; omit for the whole wiki"),
                integer("depth", Some(1), Some(20))
                    .optional()
                    .describe("Levels shown, default 3; deeper ones are counted"),
            ],
            handler: handler!(|ctx, args| {
                let tree = ctx
                    .wiki
                    .tree()
                    .await
                    .map_err(failed("Could not read the page tree"))?;
                let depth = args.opt_int("depth").unwrap_or(3) as usize;
                let under = args.opt_str("under").filter(|u| !u.is_empty());
                let mut node: &Page = &tree;
                if let Some(under) = under {
                    if let Some(id) = parent_id(ctx, Some(&under)).await? {
                        node = find_node(&tree, &id).ok_or_else(|| {
                            failure(format!("\"{under}\" is not in the tree"), None)
                        })?;
                    }
                }
                success(json!(tree_lines(node, depth)))
            }),
        },
        Tool {
            name: "read_page",
            description: "One page: Markdown content, tags, properties, authors and the version needed to change it.",
            fields: vec![
                params::page(),
                boolean("content")
                    .optional()
                    .describe("false omits the content"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                success(page_out(&p, args.opt_bool("content") != Some(false)))
            }),
        },
        Tool {
            name: "read_lookup_path",
            description: "Whether a path exists, segment by segment, and whether it can be created.",
            fields: vec![trimmed("path").nonempty()],
            handler: handler!(|ctx, args| {
                let path = args.str("path");
                success(
                    ctx.wiki
                        .lookup(&path)
                        .await
                        .map_err(failed(format!("Could not look up \"{path}\"")))?,
                )
            }),
        },
        Tool {
            name: "read_find_title",
            description: "Pages whose title matches exactly (case-insensitive), with path and id.",
            fields: vec![trimmed("title").nonempty()],
            handler: handler!(|ctx, args| {
                let title = args.str("title");
                let r = ctx
                    .wiki
                    .by_title(&title)
                    .await
                    .map_err(failed(format!("Could not search titles for \"{title}\"")))?;
                success(Value::Array(list(&field(&r, "matches"))))
            }),
        },
        Tool {
            name: "read_search",
            description: "Full-text search over page titles, content and tags, best match first, with excerpts.",
            fields: vec![
                trimmed("query").nonempty(),
                integer("offset", Some(0), None).optional(),
                limit("limit", 100, "Default 20"),
            ],
            handler: handler!(|ctx, args| {
                let query = args.str("query");
                let d = ctx
                    .wiki
                    .search(&query, args.opt_int("offset"), args.opt_int("limit"))
                    .await
                    .map_err(failed(format!("Search for \"{query}\" failed")))?;
                let items = list(&field(&d, "items"));
                let offset = d.get("offset").and_then(Value::as_f64).unwrap_or(f64::NAN);
                let count = d.get("count").and_then(Value::as_f64).unwrap_or(f64::NAN);
                let pages: Vec<Value> = items
                    .iter()
                    .map(|i| {
                        let kind = field(i, "kind");
                        compact([
                            ("id", field(i, "page_id")),
                            ("path", field(i, "path")),
                            (
                                "title",
                                json!(plain(i.get("title").and_then(Value::as_str).unwrap_or(""))),
                            ),
                            ("kind", if kind == "section" { kind } else { Value::Null }),
                            ("tags", field(i, "tags")),
                            (
                                "excerpt",
                                json!(plain(
                                    i.get("excerpt").and_then(Value::as_str).unwrap_or("")
                                )),
                            ),
                        ])
                    })
                    .collect();
                success(compact([
                    ("total", field(&d, "count")),
                    (
                        "offset",
                        Some(field(&d, "offset"))
                            .filter(truthy)
                            .unwrap_or(Value::Null),
                    ),
                    (
                        "more",
                        if offset + (items.len() as f64) < count {
                            json!(true)
                        } else {
                            Value::Null
                        },
                    ),
                    ("pages", Value::Array(pages)),
                    ("tags", field(&d, "tag_facets")),
                ]))
            }),
        },
        Tool {
            name: "read_search_status",
            description: "Whether the search index is still building, and how many pages it holds.",
            fields: vec![],
            handler: handler!(|ctx, args| {
                success(
                    ctx.wiki
                        .indexing()
                        .await
                        .map_err(failed("Could not read the search index status"))?,
                )
            }),
        },
        Tool {
            name: "read_tags",
            description: "Tags in use with how many pages carry each.",
            fields: vec![
                text("filter")
                    .optional()
                    .describe("Only tags containing this text"),
                limit("limit", 500, "Default 50"),
            ],
            handler: handler!(|ctx, args| {
                let r = ctx
                    .wiki
                    .tags(args.opt_str("filter").as_deref(), args.opt_int("limit"))
                    .await
                    .map_err(failed("Could not list tags"))?;
                success(if r.is_null() { json!([]) } else { r })
            }),
        },
        Tool {
            name: "read_pages_by_tags",
            description: "Pages carrying all of the given tags.",
            fields: vec![names("tags").nonempty()],
            handler: handler!(|ctx, args| {
                let tags = args.opt_strs("tags").unwrap_or_default();
                let r = ctx
                    .wiki
                    .pages_by_tags(&tags)
                    .await
                    .map_err(failed("Could not list pages by tag"))?;
                let pages: Vec<Value> = list(&r)
                    .iter()
                    .map(|p| {
                        compact([
                            ("id", field(p, "id")),
                            ("path", field(p, "path")),
                            ("title", field(p, "title")),
                            ("tags", field(p, "tags")),
                            ("updated", day(p.get("updatedAt").and_then(Value::as_str))),
                            (
                                "updatedBy",
                                p.get("lastAuthor")
                                    .map(|a| field(a, "username"))
                                    .unwrap_or(Value::Null),
                            ),
                        ])
                    })
                    .collect();
                success(Value::Array(pages))
            }),
        },
        Tool {
            name: "read_properties",
            description: "Property keys in use (structured page metadata) with how many pages set each.",
            fields: vec![
                text("filter")
                    .optional()
                    .describe("Only keys containing this text"),
                limit("limit", 500, "Default 50"),
            ],
            handler: handler!(|ctx, args| {
                let r = ctx
                    .wiki
                    .property_keys(args.opt_str("filter").as_deref(), args.opt_int("limit"))
                    .await
                    .map_err(failed("Could not list property keys"))?;
                success(if r.is_null() { json!([]) } else { r })
            }),
        },
        Tool {
            name: "read_pages_by_property",
            description: "Pages that set a property, optionally to one value.",
            fields: vec![trimmed("key").nonempty(), text("value").optional()],
            handler: handler!(|ctx, args| {
                let key = args.str("key");
                let r = ctx
                    .wiki
                    .pages_by_property(&key, args.opt_str("value").as_deref())
                    .await
                    .map_err(failed(format!(
                        "Could not list pages with property \"{key}\""
                    )))?;
                let pages: Vec<Value> = list(&r)
                    .iter()
                    .map(|p| {
                        let mut properties = Map::new();
                        for (k, v) in p
                            .get("properties")
                            .and_then(Value::as_object)
                            .into_iter()
                            .flatten()
                        {
                            if let Some(value) = v.get("value") {
                                properties.insert(k.clone(), value.clone());
                            }
                        }
                        compact([
                            ("id", field(p, "id")),
                            ("path", field(p, "path")),
                            ("title", field(p, "title")),
                            ("properties", Value::Object(properties)),
                            ("updated", day(p.get("updatedAt").and_then(Value::as_str))),
                        ])
                    })
                    .collect();
                success(Value::Array(pages))
            }),
        },
        Tool {
            name: "read_links",
            description: "A page's links: pages linking to it, pages it links to, and broken links both ways.",
            fields: vec![params::page()],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let r = ctx.wiki.links(&p.id).await.map_err(failed(format!(
                    "Could not read the links of \"{}\"",
                    p.path
                )))?;
                let to = |l: &Value| {
                    compact([
                        ("id", field(l, "to_page_id")),
                        ("path", field(l, "to_path")),
                        ("title", field(l, "to_page_title")),
                    ])
                };
                let from = |l: &Value| {
                    compact([
                        ("id", field(l, "from_page_id")),
                        ("path", field(l, "from_path")),
                        ("title", field(l, "from_page_title")),
                    ])
                };
                success(compact([
                    (
                        "linkedFrom",
                        Value::Array(list(&field(&r, "backlinks")).iter().map(from).collect()),
                    ),
                    (
                        "linksTo",
                        Value::Array(list(&field(&r, "outgoings")).iter().map(to).collect()),
                    ),
                    (
                        "brokenOut",
                        Value::Array(
                            list(&field(&r, "broken_outgoings"))
                                .iter()
                                .map(|l| field(l, "to_path"))
                                .collect(),
                        ),
                    ),
                    (
                        "brokenIn",
                        Value::Array(
                            list(&field(&r, "broken_incoming"))
                                .iter()
                                .map(from)
                                .collect(),
                        ),
                    ),
                ]))
            }),
        },
        Tool {
            name: "read_broken_links",
            description: "Every broken link in the wiki (admins only).",
            fields: vec![],
            handler: handler!(|ctx, args| {
                let r = ctx
                    .wiki
                    .broken_links()
                    .await
                    .map_err(failed("Could not list broken links"))?;
                let links: Vec<Value> = list(&field(&r, "links"))
                    .iter()
                    .map(|l| {
                        pick(
                            l,
                            &[
                                ("from", "from_path"),
                                ("fromId", "from_page_id"),
                                ("to", "to_path"),
                            ],
                        )
                    })
                    .collect();
                success(Value::Array(links))
            }),
        },
        Tool {
            name: "read_revisions",
            description: "A page's revision history, newest first (needs revisions on).",
            fields: vec![
                params::page(),
                limit("limit", 100, "Default 20"),
                text("cursor")
                    .optional()
                    .describe("next from the previous answer"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let r = ctx
                    .wiki
                    .revisions(
                        &p.id,
                        Some(args.opt_int("limit").unwrap_or(20)),
                        args.opt_str("cursor").as_deref(),
                    )
                    .await
                    .map_err(failed(format!(
                        "Could not read the history of \"{}\" (are revisions on? see read_wiki)",
                        p.path
                    )))?;
                success(compact([
                    (
                        "revisions",
                        Value::Array(
                            list(&field(&r, "revisions"))
                                .iter()
                                .map(revision_out)
                                .collect(),
                        ),
                    ),
                    (
                        "next",
                        Some(field(&r, "nextCursor"))
                            .filter(truthy)
                            .unwrap_or(Value::Null),
                    ),
                ]))
            }),
        },
        Tool {
            name: "read_revision",
            description: "One past revision of a page with its full content.",
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
                    .revision(&p.id, &revision)
                    .await
                    .map_err(failed(format!(
                        "Could not read revision {revision} of \"{}\"",
                        p.path
                    )))?;
                let mut out = revision_out(&field(&r, "revision"));
                if let Some(content) = r.get("content") {
                    out["content"] = content.clone();
                }
                success(out)
            }),
        },
        Tool {
            name: "read_compare_revisions",
            description: "Line diff between two revisions of a page, or between one revision and the latest.",
            fields: vec![
                params::page(),
                trimmed("base").nonempty().describe("Older revision id"),
                trimmed("target")
                    .optional()
                    .describe("Newer revision id; omit for the latest"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let base = args.str("base");
                let to = match args.opt_str("target").filter(|t| !t.is_empty()) {
                    Some(t) => t,
                    None => {
                        let latest =
                            ctx.wiki
                                .latest_revision(&p.id)
                                .await
                                .map_err(failed(format!(
                                    "Could not find the latest revision of \"{}\"",
                                    p.path
                                )))?;
                        latest.get("id").map(text_of).unwrap_or_default()
                    }
                };
                let r = ctx
                    .wiki
                    .compare(&p.id, &base, &to)
                    .await
                    .map_err(failed(format!(
                        "Could not compare revisions of \"{}\"",
                        p.path
                    )))?;
                let (b, t) = (field(&r, "base"), field(&r, "target"));
                let (br, tr) = (field(&b, "revision"), field(&t, "revision"));
                let diff = if truthy(&field(&r, "contentChanged")) {
                    let text = |s: &Value| {
                        s.get("content")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    };
                    line_diff(&text(&b), &text(&t), 2)
                } else {
                    Some("(content unchanged)".into())
                };
                let changed = |key: &str| {
                    let (from, to) = (field(&br, key), field(&tr, key));
                    if from != to {
                        json!(format!("{} → {}", text_of(&from), text_of(&to)))
                    } else {
                        Value::Null
                    }
                };
                success(compact([
                    ("base", revision_out(&br)),
                    ("target", revision_out(&tr)),
                    ("title", changed("title")),
                    ("path", changed("path")),
                    ("assetChanges", field(&r, "assetChanges")),
                    (
                        "diff",
                        json!(diff.unwrap_or_else(|| {
                            "Too large to diff here: read both revisions with read_revision.".into()
                        })),
                    ),
                ]))
            }),
        },
        Tool {
            name: "read_favorites",
            description: "The signed-in user's favorite pages.",
            fields: vec![],
            handler: handler!(|ctx, args| {
                let r = ctx
                    .wiki
                    .favorites()
                    .await
                    .map_err(failed("Could not list favorites"))?;
                let pages: Vec<Page> =
                    serde_json::from_value(field(&r, "pages")).unwrap_or_default();
                success(Value::Array(pages.iter().map(changed_out).collect()))
            }),
        },
        Tool {
            name: "read_assets",
            description: "Files attached to a page, with the URL to embed or link them.",
            fields: vec![params::page()],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let r = ctx.wiki.assets(&p.id).await.map_err(failed(format!(
                    "Could not list the files of \"{}\"",
                    p.path
                )))?;
                let files: Vec<Value> = list(&field(&r, "files"))
                    .iter()
                    .map(|f| {
                        let file = text_of(f);
                        json!({ "name": file.rsplit('/').next().unwrap_or(""), "file": file, "url": ctx.wiki.session.absolute(&file) })
                    })
                    .collect();
                success(Value::Array(files))
            }),
        },
        Tool {
            name: "read_slug",
            description: "The slug LeafWiki would give a title, unique under its parent (editors only).",
            fields: vec![trimmed("title").nonempty(), params::parent()],
            handler: handler!(|ctx, args| {
                let title = args.str("title");
                let parent = parent_id(ctx, args.opt_str("parent").as_deref()).await?;
                success(
                    ctx.wiki
                        .slug(&title, parent.as_deref(), None)
                        .await
                        .map_err(failed(format!("Could not suggest a slug for \"{title}\"")))?,
                )
            }),
        },
        Tool {
            name: "read_refactor_preview",
            description: "Which pages link to a page and would be rewritten if it were renamed or moved (needs link refactoring on); changes nothing.",
            fields: vec![
                params::page(),
                trimmed("title")
                    .nonempty()
                    .optional()
                    .describe("New title (rename)"),
                trimmed("slug")
                    .nonempty()
                    .optional()
                    .describe("New slug (rename)"),
                params::parent().describe("New parent id or path (move); \"/\" for the top level"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let parent = args.opt_str("parent");
                let target = parent_id(ctx, parent.as_deref()).await?;
                let moving = parent.is_some();
                let mut input = json!({
                    "kind": if moving { "move" } else { "rename" },
                    "title": args.opt_str("title").unwrap_or_else(|| p.title.clone()),
                    "slug": args.opt_str("slug").unwrap_or_else(|| p.slug.clone()),
                });
                if moving {
                    input["parentId"] = json!(target.as_deref().unwrap_or("root"));
                }
                let d = ctx
                    .wiki
                    .refactor_preview(&p.id, input)
                    .await
                    .map_err(failed(format!("Could not preview the change of \"{}\" (is link refactoring on? see read_wiki)", p.path)))?;
                let pages: Vec<Value> = list(&field(&d, "affectedPages"))
                    .iter()
                    .map(|a| {
                        compact([
                            ("id", field(a, "fromPageId")),
                            ("path", field(a, "fromPath")),
                            ("links", field(a, "matchedPaths")),
                            ("warnings", field(a, "warnings")),
                        ])
                    })
                    .collect();
                success(compact([
                    ("from", field(&d, "oldPath")),
                    ("to", field(&d, "newPath")),
                    (
                        "links",
                        d.get("counts")
                            .map(|c| field(c, "matchedLinks"))
                            .unwrap_or(Value::Null),
                    ),
                    ("pages", Value::Array(pages)),
                    ("warnings", field(&d, "warnings")),
                ]))
            }),
        },
    ]
}
