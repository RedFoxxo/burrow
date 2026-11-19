# Changelog

Versions follow [Semantic Versioning](https://semver.org/). The version lives in
`package.json` and `src/version.ts` (a test keeps them equal).

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
