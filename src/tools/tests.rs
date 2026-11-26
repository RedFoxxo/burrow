use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;
use crate::config::Config;
use crate::http::{Body, Request, Response, Transport, system_clock};

fn config() -> Config {
    Config {
        base_url: "https://wiki.test".into(),
        username: String::new(),
        password: String::new(),
        read_only: false,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Sent {
    method: &'static str,
    path: String,
    body: Value,
}

/// A wiki without sign-in that knows one page; `sent` keeps every change burrow made.
#[derive(Clone, Default)]
struct FakeWiki {
    sent: Arc<Mutex<Vec<Sent>>>,
}

fn page() -> Value {
    json!({ "id": "p1", "title": "Deploy", "slug": "deploy", "path": "deploy", "version": "v1", "position": 0, "kind": "page", "children": null, "content": "# Deploy\n\nooo\n", "tags": null, "properties": { "owner": "ed" } })
}

impl Transport for FakeWiki {
    fn send(&self, request: Request) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(async move {
            let path = url::Url::parse(&request.url).unwrap().path().to_string();
            let method = request.method.as_str();
            if method != "GET" {
                let body = match &request.body {
                    Body::Json(text) => serde_json::from_str(text).unwrap(),
                    Body::File { name, bytes } => json!({ "file": name, "bytes": bytes }),
                    Body::Empty => Value::Null,
                };
                self.sent.lock().unwrap().push(Sent {
                    method,
                    path: path.clone(),
                    body,
                });
            }
            let ok = |body: Value| Response {
                status: 200,
                body: Ok(body.to_string()),
                ..Response::default()
            };
            Ok(match (method, path.as_str()) {
                ("GET", "/api/config") => ok(json!({ "authDisabled": true })),
                ("GET", "/api/tree") => {
                    let mut child = page();
                    child.as_object_mut().unwrap().remove("content");
                    ok(
                        json!({ "id": "root", "title": "", "slug": "", "path": "", "version": "", "position": 0, "kind": "section", "children": [child] }),
                    )
                }
                ("GET", "/api/pages/p1") => ok(page()),
                ("PUT", "/api/pages/p1") => {
                    let mut saved = page();
                    saved["version"] = json!("v2");
                    ok(saved)
                }
                ("POST", "/api/pages/p1/assets") => ok(json!({ "file": "/assets/p1/a.png" })),
                _ => Response {
                    status: 404,
                    body: Ok("Page not found".into()),
                    ..Response::default()
                },
            })
        })
    }
}

fn connect(wiki: &FakeWiki) -> (Ctx, Vec<Tool>) {
    (
        context(
            config(),
            Box::new(wiki.clone()),
            Arc::new(|_| {}),
            system_clock(),
        ),
        tools_for(false),
    )
}

#[tokio::test]
async fn read_tree_under_slash_lists_the_whole_wiki() {
    let (ctx, tools) = connect(&FakeWiki::default());
    let r = call(&ctx, &tools, "read_tree", &json!({ "under": "/" })).await;
    assert_eq!(
        r,
        ToolResult {
            text: "- Deploy (deploy) id=p1".into(),
            is_error: false
        }
    );
}

#[tokio::test]
async fn write_properties_with_nothing_to_set_or_remove_sends_nothing() {
    let wiki = FakeWiki::default();
    let (ctx, tools) = connect(&wiki);
    let r = call(
        &ctx,
        &tools,
        "write_properties",
        &json!({ "page": "p1", "set": {} }),
    )
    .await;
    assert!(r.is_error && r.text.starts_with("Not sent"), "{r:?}");
    assert!(wiki.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn write_properties_keeps_the_others_and_removes_what_was_asked() {
    let wiki = FakeWiki::default();
    let (ctx, tools) = connect(&wiki);
    let r = call(
        &ctx,
        &tools,
        "write_properties",
        &json!({ "page": "p1", "set": { "team": "core" }, "remove": ["nope"] }),
    )
    .await;
    assert!(!r.is_error, "{r:?}");
    let sent = wiki.sent.lock().unwrap();
    assert_eq!(
        sent[0].body["properties"],
        json!({ "owner": "ed", "team": "core" })
    );
    assert_eq!(sent[0].body["version"], json!("v1"));
    assert_eq!(sent[0].body["tags"], json!([]));
}

#[tokio::test]
async fn write_upload_asset_takes_a_data_url_and_refuses_what_is_not_base64() {
    let wiki = FakeWiki::default();
    let (ctx, tools) = connect(&wiki);
    let bytes = [0x89u8, 0x50, 0x4e, 0x47, 0, 255];
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let r = call(&ctx, &tools, "write_upload_asset", &json!({ "page": "p1", "filename": "a.png", "base64": format!("data:image/png;base64,{encoded}") })).await;
    assert!(!r.is_error, "{r:?}");
    assert_eq!(
        serde_json::from_str::<Value>(&r.text).unwrap()["markdown"],
        json!("![a.png](/assets/p1/a.png)")
    );
    assert_eq!(
        *wiki.sent.lock().unwrap(),
        vec![Sent {
            method: "POST",
            path: "/api/pages/p1/assets".into(),
            body: json!({ "file": "a.png", "bytes": bytes })
        }]
    );

    let bad = call(
        &ctx,
        &tools,
        "write_upload_asset",
        &json!({ "page": "p1", "filename": "a.png", "base64": "this is not base64!" }),
    )
    .await;
    assert!(
        bad.is_error && bad.text.starts_with("Not sent: ") && bad.text.contains("base64"),
        "{bad:?}"
    );
    assert_eq!(wiki.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn write_edit_text_refuses_a_passage_that_occurs_twice_even_overlapping() {
    let wiki = FakeWiki::default();
    let (ctx, tools) = connect(&wiki);
    // "ooo" holds "oo" twice, overlapping, as indexOf counts it; "ep" occurs once.
    let twice = call(
        &ctx,
        &tools,
        "write_edit_text",
        &json!({ "page": "p1", "version": "v1", "find": "oo", "replace": "x" }),
    )
    .await;
    assert!(twice.text.contains("more than once"), "{twice:?}");
    let stale = call(
        &ctx,
        &tools,
        "write_edit_text",
        &json!({ "page": "p1", "version": "v0", "find": "ep", "replace": "x" }),
    )
    .await;
    assert!(stale.text.contains("changed since your read"), "{stale:?}");
    let once = call(
        &ctx,
        &tools,
        "write_edit_text",
        &json!({ "page": "p1", "version": "v1", "find": "ep", "replace": "x" }),
    )
    .await;
    assert!(!once.is_error, "{once:?}");
    assert_eq!(
        wiki.sent.lock().unwrap()[0].body["content"],
        json!("# Dxloy\n\nooo\n")
    );
}

#[tokio::test]
async fn read_page_is_compact_and_omits_content_on_request() {
    let (ctx, tools) = connect(&FakeWiki::default());
    let full = call(&ctx, &tools, "read_page", &json!({ "page": " p1 " })).await;
    assert_eq!(
        full.text,
        r##"{"id":"p1","path":"deploy","title":"Deploy","kind":"page","version":"v1","properties":{"owner":"ed"},"content":"# Deploy\n\nooo\n"}"##
    );
    let bare = call(
        &ctx,
        &tools,
        "read_page",
        &json!({ "page": "p1", "content": false }),
    )
    .await;
    assert!(!bare.text.contains("content"));
    let missing = call(&ctx, &tools, "read_page", &json!({ "page": "nope" })).await;
    assert_eq!(
        missing.text,
        "Page \"nope\" not found or not readable\nLeafWiki: No page has the id or path \"nope\"\nstatus: 404\nrequest: GET /api/pages/by-path"
    );
}

#[test]
fn read_only_exposes_exactly_the_read_tools() {
    let names: Vec<&str> = tools_for(false).iter().map(|t| t.name).collect();
    let read: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.starts_with("read_"))
        .collect();
    assert_eq!(
        tools_for(true).iter().map(|t| t.name).collect::<Vec<_>>(),
        read
    );
}

/// The tool list as released (captured from 1.1.0): tool order, names, descriptions, schemas
/// and hints are a contract with the Mamami plugin's approval rules and with every model
/// prompt. A change here is deliberate: update `tests/golden/` with it.
#[test]
fn lists_the_released_tools() {
    for (read_only, golden) in [
        (false, include_str!("../../tests/golden/tools.json")),
        (
            true,
            include_str!("../../tests/golden/tools-read-only.json"),
        ),
    ] {
        let expected: Value = serde_json::from_str(golden).unwrap();
        let actual = list(&tools_for(read_only));
        let (expected, actual) = (
            expected["tools"].as_array().unwrap(),
            actual["tools"].as_array().unwrap(),
        );
        assert_eq!(actual.len(), expected.len());
        // As text, so key order counts too (map equality ignores it).
        for (a, e) in actual.iter().zip(expected) {
            assert_eq!(a.to_string(), e.to_string(), "{}", e["name"]);
        }
    }
}

#[tokio::test]
async fn refuses_invalid_arguments_and_unknown_tools() {
    let (ctx, tools) = connect(&FakeWiki::default());
    let refused = call(
        &ctx,
        &tools,
        "write_move",
        &json!({ "page": "p", "parent": "q", "position": -1 }),
    )
    .await;
    assert!(
        refused.is_error
            && refused
                .text
                .contains("Too small: expected number to be >=0 at position"),
        "{refused:?}"
    );
    let unknown = call(&ctx, &tools, "nope", &json!({})).await;
    assert_eq!(
        unknown,
        ToolResult {
            text: "MCP error -32602: Tool nope not found".into(),
            is_error: true
        }
    );
}

#[test]
fn page_has_children_advice_follows_the_refused_request() {
    let refused = |request: &str| {
        let e = crate::http::WikiErr {
            status: 400,
            message: "Page has children, use recursive delete".into(),
            code: Some("page_has_children".into()),
            body: String::new(),
            request: Some(request.into()),
        };
        failure("Could not do it", Some(&e)).text
    };
    let delete = refused("DELETE /api/pages/p1?version=v1");
    assert!(
        delete.contains("pass recursive: true only if the user asked"),
        "{delete}"
    );
    // Converting a section has no recursive option: the advice is to move the children.
    let convert = refused("POST /api/pages/convert/p1");
    assert!(convert.contains("write_move"), "{convert}");
    assert!(!convert.contains("recursive: true"), "{convert}");
}
