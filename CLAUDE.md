# CLAUDE.md

**burrow** is an MCP server for LeafWiki, written for Mamami. Goal: every
LeafWiki operation a non-admin editor can usefully do is a tool. Admin-only
screens (users, backups, snapshots, branding, API keys, import) are left out on
purpose.

## Architecture

- `src/http.rs`: the only code that talks to LeafWiki. Cookie session, CSRF,
  refresh, one repeat after a 401 (none when a sign-in finished meanwhile).
  Never follows redirects; a rejected password or TOTP account is remembered and
  not retried. Every outcome is a `Result`; never panic for HTTP failures. The
  `Transport` trait is the seam the tests replace.
- `src/wiki.rs`: one method per route. `resolve(ref)` takes an id or a path.
  Pages and tree nodes are typed (only the fields read); other answers stay JSON
  and keep LeafWiki's key order.
- `src/tools/{read,write,delete}.rs`: tools, grouped by tier. The name prefix is
  the approval contract (`read_` / `write_` / `delete_`); `tools_for` refuses a
  tool without one. Tool names, parameter names and env vars are a contract
  with the Mamami plugin (approval rules, skills): never rename them in 2.x.
- `src/schema.rs`: one parameter definition gives the JSON schema the model
  reads and the argument validation. The schema has no `$schema`, `execution`,
  default-valued hints or other noise: every byte there is model tokens on
  every request. `tests/golden/` pins the whole `tools/list`.
- `src/mcp.rs`: MCP over stdio, by hand: `initialize`, `ping`, `tools/list`,
  `tools/call` (concurrent). Nothing else is needed for a tools-only server.
- `src/format.rs`: answers stay short: compact JSON without empty fields, the
  tree as indented lines, search excerpts without `<b>`, revision diffs as
  unified lines.

## Stack

Rust (edition 2024, stable): `tokio` (one thread), `reqwest` with rustls and
ring, `serde_json`, `regex`. No other runtime dependency. `tests/wiki/` runs
the built executable over stdio against a real LeafWiki (`cargo test --
--include-ignored`).

## LeafWiki API facts (v0.14.2)

There is no complete API description: `docs/api/swagger.json` on LeafWiki's
`main` covers 12 read routes and is absent from the v0.14.2 tag. Routes come
from `internal/wiki/*/routes.go` at the tag; shapes from a running instance.

- **Sign-in**: `GET /api/config` sets the CSRF cookie and answers
  `authDisabled`. `POST /api/auth/login {identifier, password}` sets the access
  (15 min) and refresh (7 days) cookies and answers `accessTokenExpiresAt`
  (unix seconds), or `{requiresTotp, loginChallengeToken}` for TOTP accounts.
  `POST /api/auth/refresh-token` renews both (the refresh token rotates); 422
  without a refresh cookie. Cookies are `leafwiki_csrf`, `leafwiki_at`,
  `leafwiki_rt` over http and `__Host-`-prefixed over https.
- **Lockout**: 5 failed sign-ins lock the account for 15 minutes (401
  `auth_account_locked`); login is also limited to 10 per 5 minutes per IP.
  A wrong password answers 401 `auth_invalid_credentials`.
- **API keys** are experimental in v0.14.2 (`--enable-api-key-management`):
  `Authorization: Bearer lw_…`, CSRF skipped, rights = role(user) ∩ role(key).
  burrow does not use them yet.
- **CSRF**: every non-GET under an authenticated route needs `X-CSRF-Token`
  equal to the CSRF cookie (also sent back in the `X-Csrf-Token` response
  header of config, login and refresh). Without it: 403
  `{"error":"Invalid CSRF token"}`.
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
- **Slugs**: `^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$`, some reserved (`users`,
  `login`, `settings`, ...); `internal/core/tree/slug_service.go`.
- **Revisions** (`--enable-revision`): list `{nextCursor, revisions}`; one
  revision `{revision, content, assets}`; compare answers both full snapshots
  plus `contentChanged` (burrow computes the diff). Without the option every
  revision route answers 404 `Page not found`.
- **Convert** answers 204 with no body. **Refactor** routes exist only with
  `--enable-link-refactor`.
- **Assets**: multipart field `file`; answers `/assets/<pageId>/<name>`
  (relative to the wiki root). Rename body uses `old_filename`/`new_filename`.
- **Roles**: viewers read; editors and admins write; `/links/broken` is admin-only.
