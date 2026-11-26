use std::thread::sleep;
use std::time::Duration;

use serde_json::json;

use crate::client::Burrow;
use crate::leafwiki::{EDITOR, Wiki};
use crate::{assert_re, str_of};

fn count(log: &[String], prefix: &str) -> usize {
    log.iter().filter(|l| l.starts_with(prefix)).count()
}

// A long-running MCP server outlives LeafWiki's 15-minute access token and,
// over days, the refresh token. This wiki shortens both so the real refresh
// call (cookies, CSRF) and the fallback to a new sign-in run for real.
#[test]
#[ignore = "starts a real LeafWiki, downloaded on the first run: cargo test -- --include-ignored"]
fn keeps_working_after_the_access_token_and_then_the_refresh_token_run_out() {
    let wiki = Wiki::start(&["--access-token-timeout=3s", "--refresh-token-timeout=6s"]);
    let mut ed = Burrow::connect(&wiki.url, Some(&EDITOR), &[]);
    let page = ed
        .must("write_create_page", json!({ "title": "Session" }))
        .json();
    assert_eq!(count(&ed.log(), "signed in"), 1);

    // Access token gone, refresh token valid: one refresh, no new sign-in.
    sleep(Duration::from_secs(4));
    ed.must("read_page", json!({ "page": page["id"] }));
    ed.must(
        "write_append",
        json!({ "page": page["id"], "text": "after refresh" }),
    );
    let log = ed.log();
    assert_eq!(count(&log, "signed in"), 1);
    assert_eq!(count(&log, "refresh failed"), 0);

    // Both gone: the refresh fails and burrow signs in again.
    sleep(Duration::from_secs(7));
    ed.must(
        "write_append",
        json!({ "page": page["id"], "text": "after sign-in" }),
    );
    let log = ed.log();
    assert!(
        count(&log, "refresh failed") > 0,
        "no failed refresh in the log:\n{}",
        log.join("\n")
    );
    assert_eq!(count(&log, "signed in"), 2);
    let content = ed.must("read_page", json!({ "page": page["id"] })).json();
    assert_re(
        str_of(&content["content"]),
        r"after refresh\n\nafter sign-in$",
    );
}
