# CLAUDE.md

**burrow** is an MCP server for LeafWiki, written for Mamami. Goal: every
LeafWiki operation a non-admin editor can usefully do is a tool. Admin-only
screens (users, backups, snapshots, branding, API keys, import) are left out on
purpose.

## Architecture

- `src/http/session.ts`: the only code that talks to LeafWiki. Cookie session,
  CSRF, refresh, one repeat after a 401. Every outcome is a `Result`; never throw
  for HTTP failures.
- `src/wiki/client.ts`: one method per route. `resolve(ref)` takes an id or a path.
- `src/wiki/types.ts`: response shapes as observed, only the fields read.
- `src/tools/{read,write,delete}.ts`: tools, grouped by tier. The name prefix is
  the approval contract (`read_` / `write_` / `delete_`); `server.ts` refuses a
  tool without one.
- `src/format.ts`: answers stay short: compact JSON without empty fields, the
  tree as indented lines, search excerpts without `<b>`, revision diffs as
  unified lines.

## Stack

TypeScript (strict, ESM), Node `>=22.12`, `@modelcontextprotocol/sdk`, `zod`,
`vitest`. Versions pinned exactly. No other runtime dependency.

## LeafWiki API facts (v0.14.2)

There is no complete API description: `docs/api/swagger.json` on LeafWiki's
`main` covers 12 read routes and is absent from the v0.14.2 tag. Routes come
from `internal/wiki/*/routes.go` at the tag; shapes from a running instance.

- **Sign-in**: `GET /api/config` sets the `leafwiki_csrf` cookie and answers
  `authDisabled`. `POST /api/auth/login {identifier, password}` sets
  `leafwiki_at` (15 min) and `leafwiki_rt` (7 days) and answers
  `accessTokenExpiresAt` (unix seconds), or `{requiresTotp, loginChallengeToken}`
  for TOTP accounts. `POST /api/auth/refresh-token` renews `leafwiki_at`.
- **CSRF**: every non-GET under an authenticated route needs `X-CSRF-Token`
  equal to the `leafwiki_csrf` cookie (also sent back in the `X-Csrf-Token`
  response header). Without it: 403 `{"error":"Invalid CSRF token"}`.
- **Errors**: `{"error":"text"}`, `{"error":{"code","message","template"}}`, or
  plain text (`Page not found`). Codes seen: `page_version_conflict`,
  `page_has_children`, `auth_invalid_credentials`.
- **Versions**: every page has `version` (a timestamp string). `PUT /pages/:id`,
  move, pin, convert and `DELETE /pages/:id?version=` need the current one.
  `PUT /pages/:id` requires `title` and `slug` and replaces `tags` and
  `properties`, so burrow always sends the full page.
- **Create**: `POST /pages {parentId?, title, slug, kind}` makes a page whose
  content is `# Title\n`; content, tags and properties need a second `PUT`.
- **Paths** have no leading slash in pages and the tree, but by-title,
  revisions and broken links answer with one.
- **Top level**: move and refactor take `parentId: "root"`.
- **Revisions** (`--enable-revision`): list `{nextCursor, revisions}`; one
  revision `{revision, content, assets}`; compare answers both full snapshots
  plus `contentChanged` (burrow computes the diff). Without the option every
  revision route answers 404 `Page not found`.
- **Convert** answers 204 with no body. **Refactor** routes exist only with
  `--enable-link-refactor`.
- **Assets**: multipart field `file`; answers `/assets/<pageId>/<name>`
  (relative to the wiki root). Rename body uses `old_filename`/`new_filename`.
- **Roles**: viewers read; editors and admins write; `/links/broken` is admin-only.
