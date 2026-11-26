# Changelog

Versions follow [Semantic Versioning](https://semver.org/). The version lives in
`Cargo.toml`.

## 2.0.2 — 2026-11-26

- **Fix:** the Claude Desktop bundle (`burrow.mcpb`) failed to start burrow
  ("Permission denied") where the host extracts files without their execute
  bit, as Claude Desktop on macOS does. On macOS and Linux the bundle now starts
  burrow through `/bin/sh` and a launcher that restores the execute bit and
  clears the download's quarantine flag. Each release now starts the bundle
  that way on macOS and Linux before publishing it.

## 2.0.1 — 2026-11-26

No change to the server. New ways to get it:

- **Install scripts:** `curl -fsSL …/install.sh | sh` (Linux, macOS) and
  `irm …/install.ps1 | iex` (Windows) install or update burrow and add it to
  your PATH. See "Install" in the README.
- **Claude Desktop:** each release has `burrow.mcpb`, a one-click bundle for
  every system that asks for the wiki URL and your account.
- **MCP Registry:** listed as `io.github.RedFoxxo/burrow`, updated by every
  release.
- The Rust crate is named `burrow-mcp` (the name `burrow` is taken on
  crates.io); the program is still `burrow`.

## 2.0.0 — 2026-11-26

The same server, rewritten in Rust: one executable without Node.js, about 8 MB
of memory instead of about 76 MB plus `npx`'s own process.

- **Breaking: installed as a file, not through npm.** Each release on GitHub has
  binaries for Linux (x64, arm64; static, any distribution), macOS (Apple
  silicon, Intel) and Windows (x64), with `SHA256SUMS`. Clients start
  `burrow` instead of `npx -y @redfoxxo/burrow@1`; the npm package stays at
  1.1.0. See "Install" in the README.
- Tools, parameters, descriptions, answers, environment variables and sign-in
  behaviour are unchanged: `tools/list` is the 1.1.0 list, key order included,
  and the same real-wiki tests pass.
- `burrow --version` and `burrow --help`.
- Starts in milliseconds and needs no network until the first tool call (`npx`
  checked the registry on every start).

## 1.1.0 — 2026-11-19

- Stops signing in after LeafWiki rejects the password (until restarted), so a
  wrong password can no longer lock the account (5 failures lock it for 15
  minutes).
- A redirect answer names where it pointed instead of `HTTP 302`; burrow still
  never follows one. A network failure names its cause (`ECONNREFUSED`, DNS,
  TLS) instead of `fetch failed`, and a connection lost mid-answer is an error
  answer instead of a crash.
- A call that got 401 because a sign-in finished while it was under way is
  repeated without signing in again.
- Refuses a request whose path URL parsing would rewrite (a `..` file or
  revision name) instead of sending it to another route.
- `LEAFWIKI_URL` with a user name or password in it, and a
  `LEAFWIKI_USERNAME`/`LEAFWIKI_PASSWORD` that is an unexpanded `${VAR}`, are
  refused at startup with a clear message (the password is not echoed).
- `read_tree` with `under: "/"` lists the whole wiki instead of failing.
- `write_upload_asset` accepts a `data:` URL and refuses text that is not
  base64 instead of uploading a corrupted file.
- `write_properties` with an empty `set` and nothing to remove sends nothing.
- Slugs follow LeafWiki's rule (letters, digits, single dashes);
  `write_refactor` and `read_refactor_preview` refuse an empty title or slug.
- `read_assets` answers `file` (the path to link) instead of the misnamed
  `markdown`; `read_wiki` reports features that are off as `false`; search
  hits leave out `kind` for plain pages and `offset` when it is 0.
- `tools/list` is about 28% smaller (23.4 KB to 16.8 KB, about 4,200 tokens):
  no `$schema`, `execution` or default-valued hints, shorter parameter
  descriptions.
- Dependencies: `@modelcontextprotocol/sdk` 1.32.2, `zod` 4.6.7 and `vitest`
  5.1.1, fixing the advisories `npm audit` reported.

## 1.0.0 — 2026-11-19

- First release, tested against LeafWiki v0.14.2: 39 tools (20 read, 17 write,
  2 delete) covering pages, tree, search, tags, properties, links, revisions,
  favorites, assets and link refactoring.
- Signs in with a personal account (`LEAFWIKI_USERNAME`, `LEAFWIKI_PASSWORD`),
  refreshes the session, and signs in again after a 401.
- `BURROW_READ_ONLY=true` registers only the read tools.
