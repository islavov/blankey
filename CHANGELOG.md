# Changelog

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
