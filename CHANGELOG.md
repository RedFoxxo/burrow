# Changelog

Versions follow [Semantic Versioning](https://semver.org/). The version lives in
`package.json` and `src/version.ts` (a test keeps them equal).

## 1.0.0 — unreleased

- First release, tested against LeafWiki v0.14.2: 39 tools (20 read, 17 write,
  2 delete) covering pages, tree, search, tags, properties, links, revisions,
  favorites, assets and link refactoring.
- Signs in with a personal account (`LEAFWIKI_USERNAME`, `LEAFWIKI_PASSWORD`),
  refreshes the session, and signs in again after a 401.
- `BURROW_READ_ONLY=true` registers only the read tools.
