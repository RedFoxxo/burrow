# Developing burrow

`CLAUDE.md` holds the architecture and the LeafWiki API facts found on a real
instance. Read it before changing behaviour.

## Commands

```sh
npm ci
npm run typecheck   # src and tests
npm test            # unit tests, no network
npm run test:wiki   # every tool against a real LeafWiki (downloads the pinned release once)
npm run build       # compile to build/
```

Run all four before every commit.

`npm run test:wiki` downloads the LeafWiki release named in
`tests/wiki/leafwiki.ts` into `.cache/leafwiki/`, checks its sha256, and starts
it on a free local port with throwaway data, revisions and link refactoring on,
plus an editor and a viewer account. The last test fails if a registered tool
was never called, so a new tool needs a test there.

## Moving to a new LeafWiki release

1. Change `LEAFWIKI_VERSION` in `tests/wiki/leafwiki.ts`.
2. `npm run test:wiki`. Fix what broke; record new API facts in `CLAUDE.md`.
3. Update "Tested against" in `README.md` and the changelog.

## Releasing

1. Bump the version in `package.json` and `src/version.ts` (a test keeps them
   equal), and date the changelog entry.
2. Commit, tag `vX.Y.Z`, push both.
3. `npm publish` (needs the `@redfoxxo` npm sign-in).
