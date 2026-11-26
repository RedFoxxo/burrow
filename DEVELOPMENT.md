# Developing burrow

`CLAUDE.md` holds the architecture and the LeafWiki API facts found on a real
instance. Read it before changing behaviour.

## Commands

burrow is a Rust crate (stable Rust). The real-wiki tests drive the built
executable through the official MCP client, so they need Node.js 22.12 or newer.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test          # unit tests, no network
npm ci              # once: the real-wiki test harness
npm run typecheck   # the harness
npm run test:wiki   # release build, then every tool against a real LeafWiki
```

Run all of them before every commit; CI runs the same, plus `cargo test` on
Windows and macOS.

`cargo test` covers the configuration, the sign-in and session handling against
a fake LeafWiki (`src/http.rs`), tool behaviour that needs no real wiki
(`src/tools/tests.rs`), argument validation and the answer formats.
`lists_the_released_tools` compares `tools/list` with `tests/golden/`: tool
names, parameters, descriptions and hints are a contract with the Mamami plugin
and with every model prompt, so a change there is deliberate and updates the
golden files with it.

`npm run test:wiki` builds `target/release/burrow`, downloads the LeafWiki
release named in `tests/wiki/leafwiki.ts` into `.cache/leafwiki/`, checks its
sha256, and starts it on a free local port with throwaway data, revisions and
link refactoring on, plus an editor and a viewer account. Each test starts
burrow over stdio. The last test fails if a tool was never called, so a new
tool needs a test there. `BURROW_BIN=<path> npx vitest run --config
vitest.wiki.config.ts` runs the same tests against another build.

Tool definitions are sent to the model on every request. After changing a tool
description or schema, compare the size of `tools/list` before and after, and
update the token figures under "Tools" in `README.md`.

## Moving to a new LeafWiki release

1. Change `LEAFWIKI_VERSION` in `tests/wiki/leafwiki.ts`.
2. `npm run test:wiki`. Fix what broke; record new API facts in `CLAUDE.md`.
3. Update "Tested against" in `README.md` and the changelog.

## Releasing

1. Bump `version` in `Cargo.toml` (the only place it lives) and run
   `cargo build` so `Cargo.lock` follows. Turn `## Unreleased` in the changelog
   into `## X.Y.Z — YYYY-MM-DD`.
2. Commit, tag `vX.Y.Z`, push both.
3. The `release` workflow checks that the tag matches `Cargo.toml` and the
   changelog, runs the CI checks on the tag, builds the five binaries and
   `burrow.mcpb` (`mcpb/build.sh`), starts the bundle on macOS and Linux
   (`mcpb/smoke.sh`), publishes a GitHub Release with the files, `SHA256SUMS`
   and the changelog section, then publishes `server.json` to the MCP Registry
   with the new version, bundle URL and checksum.
4. `cargo publish` puts the crate `burrow-mcp` on crates.io (needs the
   crates.io sign-in; not automated yet).

The MCP Registry signs in with the workflow's own GitHub identity, so no token
is stored; it only accepts names under `io.github.RedFoxxo/`. `server.json` in
the repository holds the listing's text and settings; the workflow fills in
the release.

`mcpb/manifest.json` is the bundle's manifest (the settings Claude Desktop asks
for); the build adds the version and the tool list from `tests/golden/`. On
macOS and Linux the bundle starts `/bin/sh server/launch.sh`, never the binary
directly: Claude Desktop on macOS extracts bundle files without the execute bit
(mcpb issue #294) and may keep the quarantine flag, and the launcher fixes both
and picks the Linux build for the CPU. `mcpb/smoke.sh` reproduces those
conditions.

The asset names (`burrow-linux-x64`, `burrow-linux-arm64`, `burrow-macos-x64`,
`burrow-macos-arm64`, `burrow-windows-x64.exe`) are a contract with
`install.sh` and `install.ps1`: never rename them. Linux builds are static
(musl), so one file runs on every distribution.

## Install scripts

`install.sh` (Linux, macOS) and `install.ps1` (Windows PowerShell 5.1 and
later) download a release file, check it against `SHA256SUMS` and install it.
Users run them from `main` (`curl … | sh`, `irm … | iex`), so a change there
reaches everyone at once; CI runs both against the latest release on Linux,
macOS and Windows, twice each.

The Mamami installer runs these scripts too. Their default folders
(`~/.local/bin`, `%LOCALAPPDATA%\Programs\burrow`) and settings
(`BURROW_VERSION`, `BURROW_INSTALL_DIR`, `BURROW_NO_MODIFY_PATH`) are a
contract with it: keep them.
