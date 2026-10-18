# Changelog

## Unreleased

- The app speaks Bulgarian. It follows the language of your relay account (read from the relay's new optional
  `device_profile`), else the computer's language, else English. The window, tray, notifications, dialogs and
  validation messages are translated; MCP output for agents stays English.
- Collection links can be deleted: the link stops working and goes away with whatever it received that was not saved;
  saved values and kept files stay. The links table shows one line per link; the fields, the full link and Copy are
  in the panel beside it. Small files show in KB.

## 0.2.1

- Agents are told where collection link answers go: once the user saves a submission into a profile, they fill
  documents from that profile (`describe_profile`, `request_fill`) and never ask the user to paste the values into
  the chat. `list_collection_links` adds `saved_into` (profile ids) and `next`, the next step in plain words.
- First release with installers for the 0.2 line: the 0.2.0 macOS build failed in CI (the release workflow now
  installs its build tools with the workflow token).

## 0.2.0

- Collection links (Vault → Collection links): a short link to a web form on the relay where someone else types the
  fields of a profile and attaches files, up to 50 MB each. What they send is encrypted in their browser to this
  computer and arrives even while Blankey is locked or offline. Review it on Activity or beside the link, save it
  into a profile, and open or export its files; it stays until you discard it. Kept files are encrypted in the vault
  and listed on Documents. Filter links by what waits for review, and search them.
- Agents can draft collection links (`create_collection_link`, confirmed in the app) and list them
  (`list_collection_links`); they never see what was sent.
- Signing in to a relay asks for its `device_resource`, for relays that keep the app's tokens apart from agents'.
- Every `mcp-*` header is forwarded to the MCP server, so MCP 2026-07-28 clients such as ChatGPT can connect
  through a relay.
- The vault moves to schema 7. Blankey 0.1.0 cannot open a vault that 0.2.0 has opened.

## 0.1.0

First release.

- Encrypted vault: AES-GCM per field, Argon2id password, recovery key, optional Touch ID unlock
- Templates from Word documents, fillable PDFs and Typst
- Fill sets: every blank mapped to a vault field, a typed value or a default, with a live document preview
- MCP server over HTTP for Claude Code, Codex and others; stdio bridge for Claude Desktop
- Tray app for macOS, Windows and Linux
- Connect web agents such as claude.ai through a relay: sign in from the tray or the new Connections page, allow
  each agent in the app, revoke it any time. Custom relays are supported (relay protocol v1, `relay/README.md`).
- `config.toml` is read as TOML; new keys `relay_enabled` and `relay_url`.
- Typst templates no longer download packages while rendering real data: a template could build a package name
  from a vault value and send it to the package registry. Only example renders download; real renders use the cache.
- `.rpm` packages for Fedora and RHEL 10 clones, built by the release workflow next to the `.deb`.
