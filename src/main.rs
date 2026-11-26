//! burrow: an MCP server for LeafWiki.

mod config;
mod format;
mod http;
mod mcp;
mod schema;
mod tools;
mod wiki;

use std::collections::HashMap;
use std::sync::Arc;

use http::{Http, Logger, system_clock};

const HELP: &str = "burrow: MCP server for LeafWiki, spoken over stdio by an MCP client.

Settings come from the environment:
  LEAFWIKI_URL       wiki root, e.g. https://wiki.example.com (required)
  LEAFWIKI_USERNAME  your LeafWiki username or email
  LEAFWIKI_PASSWORD  its password
  BURROW_READ_ONLY   true registers only the read_ tools

Options:
  --version  print the version
  --help     print this help

https://github.com/RedFoxxo/burrow";

// One thread: burrow waits on the network and stdio, it does not compute.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("--version" | "-V") => return println!("burrow {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => return println!("{HELP}"),
        Some(other) => {
            eprintln!("burrow: unknown option {other}\n\n{HELP}");
            std::process::exit(2);
        }
    }

    let env: HashMap<String, String> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    let config = match config::load_config(&env) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("no other TLS provider is installed");
    // stdout belongs to the MCP transport; everything else goes to stderr.
    let log: Logger = Arc::new(|line| eprintln!("burrow: {line}"));
    let ready = format!(
        "burrow: ready on stdio ({}{})",
        config.base_url,
        if config.read_only { ", read-only" } else { "" }
    );
    let tools = Arc::new(tools::tools_for(config.read_only));
    // Signs in on the first tool call, so a wiki that is briefly down does not stop the client.
    let ctx = Arc::new(tools::context(
        config,
        Box::new(Http::new()),
        log,
        system_clock(),
    ));
    eprintln!("{ready}");
    mcp::serve(ctx, tools).await;
}
