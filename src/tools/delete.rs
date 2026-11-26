use serde_json::json;

use super::read::handler;
use super::{Tool, failed, params, stale, success, with_page};
use crate::schema::{boolean, trimmed};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "delete_page",
            description: "Delete a page; a page with children is refused unless recursive is true.",
            fields: vec![
                params::page(),
                params::version(),
                boolean("recursive")
                    .optional()
                    .describe("Also delete every page below it; only when the user asked for that"),
            ],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                stale(&p, &args.str("version"))?;
                let recursive = args.opt_bool("recursive") == Some(true);
                ctx.wiki
                    .remove(&p.id, &p.version, recursive)
                    .await
                    .map_err(failed(format!("Could not delete \"{}\"", p.path)))?;
                success(json!({ "deleted": p.path, "id": p.id, "recursive": recursive }))
            }),
        },
        Tool {
            name: "delete_asset",
            description: "Delete a file attached to a page.",
            fields: vec![params::page(), trimmed("filename").nonempty()],
            handler: handler!(|ctx, args| {
                let p = with_page(ctx, &args.str("page")).await?;
                let filename = args.str("filename");
                ctx.wiki
                    .delete_asset(&p.id, &filename)
                    .await
                    .map_err(failed(format!(
                        "Could not delete \"{filename}\" from \"{}\"",
                        p.path
                    )))?;
                success(json!({ "deleted": filename, "page": p.path }))
            }),
        },
    ]
}
