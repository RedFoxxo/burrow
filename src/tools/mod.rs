//! Tools, grouped by tier. The name prefix is the approval contract (`read_` / `write_` /
//! `delete_`). Tool names, parameter names and env vars are a contract with the Mamami plugin
//! (approval rules, skills): never rename them in 1.x.

mod delete;
mod read;
mod write;

use serde_json::{Value, json};

use crate::config::Config;
use crate::http::{BoxFuture, Clock, Logger, Session, SessionOptions, Transport, WikiErr};
use crate::schema::{self, Args, Field};
use crate::wiki::{Page, Wiki};

/// Everything a tool handler may use. Built once per server.
pub struct Ctx {
    pub config: Config,
    pub wiki: Wiki,
}

pub fn context(config: Config, transport: Box<dyn Transport>, log: Logger, now: Clock) -> Ctx {
    let session = Session::new(SessionOptions {
        base_url: config.base_url.clone(),
        username: config.username.clone(),
        password: config.password.clone(),
        transport,
        log,
        now,
    });
    Ctx {
        config,
        wiki: Wiki { session },
    }
}

/// What a tool answers: text, and whether it is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn to_json(&self) -> Value {
        let mut result = json!({ "content": [{ "type": "text", "text": self.text }] });
        if self.is_error {
            result["isError"] = json!(true);
        }
        result
    }
}

/// `Err` is an early answer (a failure, or a refusal before anything was sent).
pub type Answer = Result<ToolResult, ToolResult>;
pub type Handler = for<'a> fn(&'a Ctx, Args) -> BoxFuture<'a, Answer>;

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub fields: Vec<Field>,
    pub handler: Handler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Read,
    Write,
    Delete,
}

pub fn tier_of(name: &str) -> Option<Tier> {
    match name.split('_').next() {
        Some("read") => Some(Tier::Read),
        Some("write") => Some(Tier::Write),
        Some("delete") => Some(Tier::Delete),
        _ => None,
    }
}

/// The tools to register: only `read_` ones when `read_only`.
pub fn tools_for(read_only: bool) -> Vec<Tool> {
    let mut tools = read::tools();
    if !read_only {
        tools.extend(write::tools());
        tools.extend(delete::tools());
    }
    for (i, tool) in tools.iter().enumerate() {
        assert!(
            tier_of(tool.name).is_some(),
            "Tool \"{}\" has no tier prefix (read_/write_/delete_)",
            tool.name
        );
        assert!(
            tools[..i].iter().all(|t| t.name != tool.name),
            "Tool \"{}\" is registered twice",
            tool.name
        );
    }
    tools
}

/// The `tools/list` answer. Only the hints that differ from the MCP defaults (not read-only,
/// destructive, not idempotent, open world).
pub fn list(tools: &[Tool]) -> Value {
    let listed: Vec<Value> = tools
        .iter()
        .map(|t| {
            let annotations = match tier_of(t.name) {
                Some(Tier::Read) => json!({ "readOnlyHint": true, "openWorldHint": false }),
                Some(Tier::Write) => json!({ "destructiveHint": false, "openWorldHint": false }),
                _ => json!({ "destructiveHint": true, "openWorldHint": false }),
            };
            json!({ "name": t.name, "description": t.description, "inputSchema": schema::input_schema(&t.fields), "annotations": annotations })
        })
        .collect();
    json!({ "tools": listed })
}

/// Validates the arguments and runs the tool. Never fails: every outcome is an answer.
pub async fn call(ctx: &Ctx, tools: &[Tool], name: &str, args: &Value) -> ToolResult {
    let Some(tool) = tools.iter().find(|t| t.name == name) else {
        return ToolResult {
            text: format!("MCP error -32602: Tool {name} not found"),
            is_error: true,
        };
    };
    match schema::validate(&tool.fields, args) {
        Err(problems) => ToolResult {
            text: format!(
                "MCP error -32602: Input validation error: Invalid arguments for tool {name}: {problems}"
            ),
            is_error: true,
        },
        Ok(args) => (tool.handler)(ctx, args)
            .await
            .unwrap_or_else(|early| early),
    }
}

/// Text answers stay as they are; anything else is compact JSON.
pub fn success(data: Value) -> Answer {
    let text = match data {
        Value::String(s) => s,
        other => other.to_string(),
    };
    Ok(ToolResult {
        text,
        is_error: false,
    })
}

/// What the AI should do next, for the LeafWiki errors that have one answer.
fn next_step(code: &str) -> Option<&'static str> {
    match code {
        "page_version_conflict" => Some(
            "Someone changed the page after you read it: read it again, check their change, and only then retry with the new version.",
        ),
        "page_has_children" => Some(
            "The page has child pages: delete them first, or pass recursive: true only if the user asked to delete everything below it.",
        ),
        _ => None,
    }
}

/// A failed call: one summary line, LeafWiki's message and status, and the next step when there is one.
pub fn failure(summary: impl Into<String>, error: Option<&WikiErr>) -> ToolResult {
    let summary = summary.into();
    let mut lines = vec![summary.clone()];
    if let Some(e) = error {
        if !e.message.is_empty() && !summary.contains(&e.message) {
            lines.push(format!("LeafWiki: {}", e.message));
        }
        if e.status != 0 {
            let code = e
                .code
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            lines.push(format!("status: {}{code}", e.status));
        }
        let next = match e.code.as_deref() {
            Some(code) => next_step(code),
            None if e.status == 403 => Some(
                "The signed-in account may not do this (editors and admins can change pages; some reads are admin-only).",
            ),
            None => None,
        };
        if let Some(next) = next {
            lines.push(next.to_string());
        }
        if let Some(request) = &e.request {
            lines.push(format!("request: {request}"));
        }
    }
    ToolResult {
        text: lines.join("\n"),
        is_error: true,
    }
}

/// `failure` for `map_err`: the summary, then LeafWiki's side.
pub fn failed(summary: impl Into<String>) -> impl FnOnce(WikiErr) -> ToolResult {
    let summary = summary.into();
    move |e| failure(summary, Some(&e))
}

/// A call rejected before anything was sent.
pub fn invalid(message: impl AsRef<str>) -> ToolResult {
    failure(format!("Not sent: {}", message.as_ref()), None)
}

/// Resolves a page reference, or answers with why it could not.
pub async fn with_page(ctx: &Ctx, reference: &str) -> Result<Page, ToolResult> {
    ctx.wiki.resolve(reference).await.map_err(failed(format!(
        "Page \"{reference}\" not found or not readable"
    )))
}

/// The id of a parent reference; `None` for the top level.
pub async fn parent_id(ctx: &Ctx, reference: Option<&str>) -> Result<Option<String>, ToolResult> {
    match reference {
        None | Some("") | Some("/") => Ok(None),
        Some(r) => ctx
            .wiki
            .resolve(r)
            .await
            .map(|p| Some(p.id))
            .map_err(failed(format!("Parent \"{r}\" not found"))),
    }
}

/// Checks the caller's version against the page's current one before changing anything.
pub fn stale(page: &Page, given: &str) -> Result<(), ToolResult> {
    if page.version == given {
        return Ok(());
    }
    let by = page
        .metadata
        .last_author
        .as_ref()
        .and_then(|u| u.username.as_deref())
        .map(|u| format!(" by {u}"))
        .unwrap_or_default();
    Err(failure(
        format!(
            "Not sent: \"{}\" changed since your read (your version {given}, now {}{by}). Read it again, check the change, then retry with the new version.",
            page.path, page.version
        ),
        None,
    ))
}

/// Shared parameters.
pub mod params {
    use crate::schema::{Field, one_of, trimmed};

    pub fn page() -> Field {
        trimmed("page").nonempty().describe("Page id or path")
    }
    pub fn parent() -> Field {
        trimmed("parent")
            .optional()
            .describe("Parent id or path; omit for the top level")
    }
    pub fn kind() -> Field {
        one_of("kind", &["page", "section"]).describe("section: a folder-like page for children")
    }
    pub fn version() -> Field {
        trimmed("version")
            .nonempty()
            .describe("version from your last read of the page")
    }
}

#[cfg(test)]
mod tests;
