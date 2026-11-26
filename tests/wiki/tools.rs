use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::client::Burrow;
use crate::leafwiki::{Account, EDITOR, VIEWER, Wiki};
use crate::{assert_has, assert_re, one, str_of};

/// One scenario, in order: each step works on the pages the earlier ones made.
#[test]
#[ignore = "starts a real LeafWiki, downloaded on the first run: cargo test -- --include-ignored"]
fn every_tool_against_a_real_wiki() {
    let wiki = Wiki::start(&[]);
    let mut ed = Burrow::connect(&wiki.url, Some(&EDITOR), &[]);
    pages(&mut ed);
    edits(&mut ed);
    paths(&mut ed);
    search(&mut ed);
    links(&mut ed);
    history(&mut ed);
    structure(&mut ed);
    refactor(&mut ed);
    assets(&mut ed);
    delete(&mut ed);
    accounts(&wiki, &mut ed);

    // Every registered tool must be exercised above.
    let missing: Vec<String> = ed
        .tools()
        .into_iter()
        .filter(|n| !ed.touched.contains(n))
        .collect();
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "tools never called against the real wiki"
    );
}

fn read(ed: &mut Burrow, page: &str) -> Value {
    ed.must("read_page", json!({ "page": page })).json()
}

/// Create, read, update with versions, and refuse a stale version.
fn pages(ed: &mut Burrow) {
    let created = ed
        .must(
            "write_create_page",
            json!({ "title": "Runbooks", "kind": "section" }),
        )
        .json();
    assert_has(&created, json!({ "path": "runbooks", "kind": "section" }));

    let deploy = ed
        .must(
            "write_create_page",
            json!({
                "parent": "runbooks",
                "title": "Deploy Guide",
                "content": "# Deploy\n\nStep one.\n",
                "tags": ["ops"],
                "properties": { "owner": "leszek" },
            }),
        )
        .json();
    assert_eq!(deploy["path"], "runbooks/deploy-guide");

    let page = read(ed, "runbooks/deploy-guide");
    assert_has(
        &page,
        json!({
            "title": "Deploy Guide",
            "content": "# Deploy\n\nStep one.\n",
            "tags": ["ops"],
            "properties": { "owner": "leszek" },
            "updatedBy": "editor",
        }),
    );
    assert!(
        ed.must("read_page", json!({ "page": page["id"], "content": false }))
            .json()
            .get("content")
            .is_none()
    );

    let updated = ed
        .must(
            "write_update_page",
            json!({ "page": page["id"], "version": page["version"], "content": "# Deploy\n\nStep one.\nStep two.\n" }),
        )
        .json();
    assert_ne!(updated["version"], page["version"]);
    let after = read(ed, "runbooks/deploy-guide");
    assert_has(
        &after,
        json!({ "tags": ["ops"], "properties": { "owner": "leszek" }, "title": "Deploy Guide" }),
    );

    let stale = ed.call(
        "write_update_page",
        json!({ "page": page["id"], "version": page["version"], "content": "lost update" }),
    );
    assert!(stale.is_error);
    assert_re(&stale.text, "changed since your read");
    assert!(str_of(&read(ed, "runbooks/deploy-guide")["content"]).contains("Step two."));
}

/// Edit one passage, append; tags and properties keep the rest.
fn edits(ed: &mut Burrow) {
    let p = read(ed, "runbooks/deploy-guide");
    let edit = |find: &str, replace: &str| json!({ "page": p["id"], "version": p["version"], "find": find, "replace": replace });
    assert_re(
        &ed.call("write_edit_text", edit("Step", "x")).text,
        "more than once",
    );
    assert_re(
        &ed.call("write_edit_text", edit("nope", "x")).text,
        "not in",
    );
    ed.must("write_edit_text", edit("Step two.", "Step 2: roll out."));
    ed.must(
        "write_append",
        json!({ "page": p["id"], "text": "## Rollback\n\nRevert." }),
    );
    let p = read(ed, "runbooks/deploy-guide");
    assert_eq!(
        p["content"],
        "# Deploy\n\nStep one.\nStep 2: roll out.\n\n## Rollback\n\nRevert."
    );

    let tagged = ed
        .must(
            "write_tags",
            json!({ "page": p["id"], "add": ["howto", "ops"], "remove": [] }),
        )
        .json();
    let mut tags: Vec<&str> = tagged["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(str_of)
        .collect();
    tags.sort_unstable();
    assert_eq!(tags, ["howto", "ops"]);
    assert_eq!(
        ed.must("write_tags", json!({ "page": p["id"], "remove": ["ops"] }))
            .json()["tags"],
        json!(["howto"])
    );
    let props = ed.must(
        "write_properties",
        json!({ "page": p["id"], "set": { "team": "core" }, "remove": ["owner"] }),
    );
    assert_eq!(props.json()["properties"], json!({ "team": "core" }));
    assert!(str_of(&read(ed, "runbooks/deploy-guide")["content"]).contains("Revert."));
}

/// Paths: lookup, ensure, find by title, slug, tree.
fn paths(ed: &mut Burrow) {
    let lookup = ed
        .must(
            "read_lookup_path",
            json!({ "path": "docs/team/onboarding" }),
        )
        .json();
    assert_has(&lookup, json!({ "exists": false }));
    let ensured = ed
        .must(
            "write_ensure_path",
            json!({ "path": "docs/team/onboarding", "title": "Onboarding" }),
        )
        .json();
    assert_has(
        &ensured,
        json!({ "path": "docs/team/onboarding", "title": "Onboarding" }),
    );
    let lookup = ed
        .must(
            "read_lookup_path",
            json!({ "path": "/docs/team/onboarding/" }),
        )
        .json();
    assert_has(&lookup, json!({ "exists": true }));

    assert_has(
        &ed.must("read_find_title", json!({ "title": "deploy guide" }))
            .json(),
        json!([{ "title": "Deploy Guide" }]),
    );
    let slug = ed
        .must(
            "read_slug",
            json!({ "title": "Deploy Guide", "parent": "runbooks" }),
        )
        .json();
    assert_ne!(slug["slug"], "deploy-guide");

    let whole = ed.must("read_tree", json!({})).text;
    assert_re(&whole, r"(?m)^- Runbooks \(runbooks\) \[section\] id=");
    assert_re(
        &whole,
        r"(?m)^ {2}- Deploy Guide \(runbooks/deploy-guide\) id=",
    );
    assert_re(
        &ed.must("read_tree", json!({ "under": "docs", "depth": 1 }))
            .text,
        r"^- docs \(docs\) \[section\] id=\S+\n {2}- team \(docs/team\) \[section\] id=\S+ \+1 below$",
    );
}

/// Search, tags, properties, index status.
fn search(ed: &mut Burrow) {
    for _ in 0..50 {
        if ed.must("read_search_status", json!({})).json()["active"] != true {
            break;
        }
    }
    let hits = ed
        .must("read_search", json!({ "query": "rollback" }))
        .json();
    let first = &hits["pages"][0];
    assert_has(
        first,
        json!({ "path": "runbooks/deploy-guide", "title": "Deploy Guide" }),
    );
    assert!(!str_of(&first["excerpt"]).contains('<'));

    let tags = ed.must("read_tags", json!({})).json();
    assert!(
        tags.as_array()
            .unwrap()
            .contains(&json!({ "tag": "howto", "count": 1 })),
        "{tags}"
    );
    let by_tags = ed
        .must("read_pages_by_tags", json!({ "tags": ["howto"] }))
        .json();
    assert_has(&by_tags, json!([{ "path": "runbooks/deploy-guide" }]));
    let keys = ed.must("read_properties", json!({})).json();
    assert!(
        keys.as_array()
            .unwrap()
            .contains(&json!({ "key": "team", "count": 1 })),
        "{keys}"
    );
    let by_property = ed
        .must(
            "read_pages_by_property",
            json!({ "key": "team", "value": "core" }),
        )
        .json();
    assert_eq!(one(&by_property)["properties"], json!({ "team": "core" }));
}

/// Links both ways, and broken links need an admin.
fn links(ed: &mut Burrow) {
    let target = read(ed, "docs/team/onboarding");
    ed.must(
        "write_append",
        json!({ "page": target["id"], "text": "See [deploy](/runbooks/deploy-guide) and [gone](/nowhere)." }),
    );
    let links = ed
        .must("read_links", json!({ "page": target["id"] }))
        .json();
    assert!(str_of(&one(&links["linksTo"])["path"]).contains("deploy-guide"));
    assert_eq!(links["brokenOut"], json!(["/nowhere"]));
    let back = ed
        .must("read_links", json!({ "page": "runbooks/deploy-guide" }))
        .json();
    assert_eq!(one(&back["linkedFrom"])["id"], target["id"]);
    let broken = ed.call("read_broken_links", json!({}));
    assert!(broken.is_error);
    assert_re(&broken.text, "status: 403");
}

/// List, read, compare and restore revisions.
fn history(ed: &mut Burrow) {
    let p = read(ed, "runbooks/deploy-guide");
    let revisions = ed.must("read_revisions", json!({ "page": p["id"] })).json();
    let list = revisions["revisions"].as_array().unwrap();
    assert!(list.len() >= 3, "{revisions}");
    let first = &list.last().unwrap()["id"];
    let old = ed
        .must(
            "read_revision",
            json!({ "page": p["id"], "revision": first }),
        )
        .json();
    assert_eq!(old["content"], "# Deploy Guide\n");

    let diff = ed
        .must(
            "read_compare_revisions",
            json!({ "page": p["id"], "base": first }),
        )
        .json();
    assert_re(str_of(&diff["diff"]), r"(?m)^-# Deploy Guide$");
    assert_re(str_of(&diff["diff"]), r"(?m)^\+## Rollback$");

    let restored = ed
        .must(
            "write_restore_revision",
            json!({ "page": p["id"], "revision": first }),
        )
        .json();
    assert_ne!(restored["version"], p["version"]);
    assert_eq!(
        read(ed, "runbooks/deploy-guide")["content"],
        "# Deploy Guide\n"
    );
    let newest = ed
        .must("read_revisions", json!({ "page": p["id"], "limit": 1 }))
        .json();
    one(&newest["revisions"]);
    assert!(newest["next"].is_string(), "{newest}");
}

/// Move, sort, pin, convert, copy, favorites.
fn structure(ed: &mut Burrow) {
    let a = ed
        .must(
            "write_create_page",
            json!({ "parent": "runbooks", "title": "Alpha" }),
        )
        .json();
    let b = ed
        .must(
            "write_create_page",
            json!({ "parent": "runbooks", "title": "Beta" }),
        )
        .json();

    assert_eq!(
        ed.must("write_move", json!({ "page": a["id"], "parent": "docs" }))
            .json()["path"],
        "docs/alpha"
    );
    assert_eq!(
        ed.must("write_move", json!({ "page": a["id"], "parent": "/" }))
            .json()["path"],
        "alpha"
    );
    ed.must(
        "write_move",
        json!({ "page": a["id"], "parent": "runbooks", "position": 0 }),
    );

    ed.must(
        "write_sort",
        json!({ "parent": "runbooks", "order": [b["id"], "runbooks/deploy-guide", a["id"]] }),
    );
    let tree = ed.must("read_tree", json!({ "under": "runbooks" })).text;
    let order: Vec<&str> = tree
        .lines()
        .skip(1)
        .map(|l| l.trim().split(" (").next().unwrap_or_default())
        .collect();
    assert_eq!(order, ["- Beta", "- Deploy Guide", "- Alpha"]);

    assert_eq!(
        ed.must("write_pin", json!({ "page": a["id"], "pinned": true }))
            .json()["pinned"],
        true
    );
    assert!(
        ed.must("write_pin", json!({ "page": a["id"], "pinned": false }))
            .json()
            .get("pinned")
            .is_none()
    );

    assert_eq!(
        ed.must(
            "write_convert",
            json!({ "page": b["id"], "kind": "section" })
        )
        .json()["kind"],
        "section"
    );
    assert_re(
        &ed.call(
            "write_convert",
            json!({ "page": b["id"], "kind": "section" }),
        )
        .text,
        "already a section",
    );

    let copy = ed.must(
        "write_copy",
        json!({ "page": "runbooks/deploy-guide", "parent": "docs", "title": "Deploy Copy" }),
    );
    assert_eq!(copy.json()["path"], "docs/deploy-copy");

    ed.must(
        "write_favorite",
        json!({ "page": a["id"], "favorite": true }),
    );
    assert_eq!(
        one(&ed.must("read_favorites", json!({})).json())["id"],
        a["id"]
    );
    ed.must(
        "write_favorite",
        json!({ "page": a["id"], "favorite": false }),
    );
    assert_eq!(ed.must("read_favorites", json!({})).json(), json!([]));
}

/// Refactor: preview, then rename rewriting links.
fn refactor(ed: &mut Burrow) {
    let onboarding = read(ed, "docs/team/onboarding");
    let p = read(ed, "runbooks/deploy-guide");
    let preview = ed
        .must(
            "read_refactor_preview",
            json!({ "page": p["id"], "slug": "deploying" }),
        )
        .json();
    assert_eq!(preview["to"], "/runbooks/deploying");
    assert_eq!(one(&preview["pages"])["id"], onboarding["id"]);

    let renamed = ed.must(
        "write_refactor",
        json!({ "page": p["id"], "version": p["version"], "slug": "deploying" }),
    );
    assert_eq!(renamed.json()["path"], "runbooks/deploying");
    assert!(str_of(&read(ed, "docs/team/onboarding")["content"]).contains("/runbooks/deploying"));
}

/// Upload, list, rename, delete.
fn assets(ed: &mut Burrow) {
    let page = "runbooks/deploying";
    let up = ed
        .must(
            "write_upload_asset",
            json!({ "page": page, "filename": "diagram.png", "base64": STANDARD.encode("not really a png") }),
        )
        .json();
    assert_re(
        str_of(&up["markdown"]),
        r"^!\[diagram\.png\]\(/assets/.+/diagram\.png\)$",
    );
    let listed = ed.must("read_assets", json!({ "page": page })).json();
    let file = one(&listed);
    assert_eq!(file["name"], "diagram.png");
    assert_re(str_of(&file["url"]), r"^http://127\.0\.0\.1:\d+/assets/");
    let renamed = ed.must(
        "write_rename_asset",
        json!({ "page": page, "from": "diagram.png", "to": "flow.png" }),
    );
    assert_re(str_of(&renamed.json()["file"]), r"flow\.png$");
    ed.must(
        "delete_asset",
        json!({ "page": page, "filename": "flow.png" }),
    );
    assert_eq!(
        ed.must("read_assets", json!({ "page": page })).json(),
        json!([])
    );
}

/// Delete refuses a stale version, and children unless recursive.
fn delete(ed: &mut Burrow) {
    let docs = read(ed, "docs");
    assert_re(
        &ed.call("delete_page", json!({ "page": "docs", "version": "old" }))
            .text,
        "changed since your read",
    );
    let refused = ed.call(
        "delete_page",
        json!({ "page": "docs", "version": docs["version"] }),
    );
    assert!(refused.is_error);
    assert_re(&refused.text, "page_has_children");
    assert_re(&refused.text, "only if the user asked");
    let deleted = ed.must(
        "delete_page",
        json!({ "page": "docs", "version": docs["version"], "recursive": true }),
    );
    assert_has(
        &deleted.json(),
        json!({ "deleted": "docs", "recursive": true }),
    );
    assert!(
        ed.call("read_page", json!({ "page": "docs/team/onboarding" }))
            .is_error
    );
}

/// Accounts and modes: the signed-in account, a viewer, read-only, bad and missing credentials.
fn accounts(wiki: &Wiki, ed: &mut Burrow) {
    let me = ed.must("read_wiki", json!({})).json();
    assert_has(
        &me,
        json!({ "user": "editor", "role": "editor", "revisions": true, "linkRefactor": true }),
    );

    let mut viewer = Burrow::connect(&wiki.url, Some(&VIEWER), &[]);
    viewer.must("read_page", json!({ "page": "runbooks/deploying" }));
    let denied = viewer.call(
        "write_append",
        json!({ "page": "runbooks/deploying", "text": "x" }),
    );
    assert!(denied.is_error);
    assert_re(&denied.text, "status: 403");

    let mut read_only = Burrow::connect(&wiki.url, Some(&EDITOR), &[("BURROW_READ_ONLY", "true")]);
    let reads: Vec<String> = ed
        .tools()
        .into_iter()
        .filter(|n| n.starts_with("read_"))
        .collect();
    assert_eq!(read_only.tools(), reads);

    let wrong = Account {
        username: "editor",
        password: "wrong",
    };
    let mut bad = Burrow::connect(&wiki.url, Some(&wrong), &[]);
    assert_re(
        &bad.call("read_tree", json!({})).text,
        r#"Sign-in as "editor" failed.*LEAFWIKI_PASSWORD"#,
    );
    let mut none = Burrow::connect(&wiki.url, None, &[]);
    assert_re(
        &none.call("read_tree", json!({})).text,
        "set LEAFWIKI_USERNAME and LEAFWIKI_PASSWORD",
    );
}
