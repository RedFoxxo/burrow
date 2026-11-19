# burrow

An MCP server for [LeafWiki](https://github.com/perber/leafwiki): AI clients
read, search, write and organize wiki pages. It signs in with your own LeafWiki
account, so every change shows up under your name.

Tested against LeafWiki **v0.14.2**.

## Run it

```sh
npx -y @redfoxxo/burrow@1
```

Requires Node.js 22.12 or newer. An MCP client starts it over stdio:

```json
{
  "mcpServers": {
    "leafwiki": {
      "command": "npx",
      "args": ["-y", "@redfoxxo/burrow@1"],
      "env": {
        "LEAFWIKI_URL": "https://wiki.example.com",
        "LEAFWIKI_USERNAME": "${LEAFWIKI_USERNAME}",
        "LEAFWIKI_PASSWORD": "${LEAFWIKI_PASSWORD}"
      }
    }
  }
}
```

| Variable | Required | Purpose |
|---|---|---|
| `LEAFWIKI_URL` | yes | Wiki root, e.g. `https://wiki.example.com`. `https` only, except `localhost` |
| `LEAFWIKI_USERNAME` | when the wiki has sign-in | Your LeafWiki username or email |
| `LEAFWIKI_PASSWORD` | when the wiki has sign-in | Its password. Keep it in your environment, never in a shared file |
| `BURROW_READ_ONLY` | no | `true` registers only the `read_` tools |

burrow signs in on the first tool call and keeps the session alive the way the
web app does. Accounts with two-factor codes (TOTP) can't sign in through
burrow yet; LeafWiki's API keys will replace the password once they are ready.

## Tools

Names start with their tier, so one approval rule covers a tier: `read_` changes
nothing, `write_` changes pages, `delete_` removes them. Pages are named by id
or by path (`docs/runbooks/deploy`).

| Tier | Tools |
|---|---|
| read | `read_wiki`, `read_tree`, `read_page`, `read_lookup_path`, `read_find_title`, `read_search`, `read_search_status`, `read_tags`, `read_pages_by_tags`, `read_properties`, `read_pages_by_property`, `read_links`, `read_broken_links` (admins), `read_revisions`, `read_revision`, `read_compare_revisions`, `read_favorites`, `read_assets`, `read_slug`, `read_refactor_preview` |
| write | `write_create_page`, `write_ensure_path`, `write_update_page`, `write_edit_text`, `write_append`, `write_tags`, `write_properties`, `write_move`, `write_sort`, `write_pin`, `write_favorite`, `write_convert`, `write_copy`, `write_refactor`, `write_restore_revision`, `write_upload_asset`, `write_rename_asset` |
| delete | `delete_page`, `delete_asset` |

Rules the server enforces:

- **No lost updates.** `write_update_page`, `write_edit_text`, `write_refactor`
  and `delete_page` need the `version` from your last read. If someone changed
  the page since, nothing is sent and the answer says who changed it.
- **No surprise deletes.** `delete_page` refuses a page with children unless
  `recursive: true`.
- **No local files.** `write_upload_asset` takes base64 data; burrow never reads
  your disk.

Revisions and link refactoring are LeafWiki options (`--enable-revision`,
`--enable-link-refactor`); `read_wiki` says which are on.

## Development

See [DEVELOPMENT.md](DEVELOPMENT.md).

## License

MIT
