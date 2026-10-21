# Auto-update on all platforms

## Context
Today Blankey users only get new versions by downloading installers from GitHub Releases themselves. We want the app to
find new releases on its own. On macOS and Windows it downloads them in the background and asks the user to restart
(a restart locks the vault, so the user picks the moment). Tauri's updater can't replace deb/rpm installs, so Linux
builds only say a new version exists and link to the release.

Current setup: Tauri 2 with `release.yml` building macOS aarch64 (ad-hoc signed .app/.dmg), Windows NSIS (currentUser)
and MSI, and deb/rpm per distro, all uploaded to the GitHub release for tag `vX.Y.Z` (repo `islavov/blankey`).

## Approach

### 1. Signing key and config
- Generate an updater keypair once with `cargo tauri signer generate`. Store the private key and its password as the
  GitHub secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The public key goes in the
  config. This is a manual step for the user; the key must never be committed.
- `src-tauri/tauri.conf.json`: add `plugins.updater` with `pubkey`, `requireSignedVersion: true` and
  `windows.installMode: "passive"`. The endpoint lives in code (`updates.rs`), so `BLANKEY_UPDATE_URL` can override it.
- `bundle.createUpdaterArtifacts` is passed by `build_mac.sh` and `build_windows.ps1` only when
  `TAURI_SIGNING_PRIVATE_KEY` is set, so local release builds and the deb/rpm jobs don't need the key.
- `src-tauri/Cargo.toml`: add `tauri-plugin-updater = "2"` and `semver = "1"` (already in Cargo.lock through tauri).
  The JS side calls our own commands, so no JS updater permissions are needed in `capabilities/default.json`.

### 2. Build scripts and release workflow
- `scripts/build_mac.sh`: also copy `bundle/macos/Blankey.app.tar.gz` and its `.sig` to `dist/`.
- `scripts/build_windows.ps1`: also copy the NSIS and MSI `.sig` files (Tauri v2 signs the installers themselves).
- `release.yml`: pass the two signing secrets to the macos and windows jobs. In `upload`, run a new
  `scripts/updater-manifest.sh` before `gh release upload`. It writes `dist/latest.json` with:
  - `version`, `pub_date`, and `notes` (that version's section of CHANGELOG.md)
  - `platforms`: `darwin-aarch64` (app.tar.gz), `windows-x86_64-nsis`, `windows-x86_64-msi`, and a plain
    `windows-x86_64` that points to NSIS
  - each entry's `url` is `https://github.com/islavov/blankey/releases/download/$TAG/<file>` and its `signature` is
    the contents of the `.sig` file
  - Linux gets no platform entry; Linux reads only `version`.
- Only a published, non-prerelease release becomes `releases/latest`, so prereleases are a safe test channel.

### 3. Rust: `src-tauri/src/app/updates.rs` (new)
- An `Updates` state, managed like `Telemetry` in `app/mod.rs:249`. Its status enum is
  `Idle | Checking | Available { version, notes, url } | Downloading | Ready { version } | Failed(String)`.
- Background loop on the tauri async runtime: first check about 30 s after launch, then every 6 h. It doesn't run in
  debug builds unless `BLANKEY_UPDATE_URL` is set; that variable also overrides the endpoint through
  `app.updater_builder().endpoints(...)` for local testing.
- macOS and Windows: `updater().check()`, then `update.download()`. Keep the bytes and the `Update` in state, set
  `Ready`, send one notification per version (`tauri_plugin_notification`, as used today) and call `tray::refresh`.
  `install()` runs `update.install(bytes)` and then `handle.restart()`.
- Linux (`cfg(target_os = "linux")`): fetch `latest.json` with `ureq`, as `telemetry.rs` does, and compare
  `version` with `CARGO_PKG_VERSION` using semver. If newer, set `Available` with the release page URL
  (`.../releases/tag/vX.Y.Z`) and notify once.
- Config (`src-tauri/src/config.rs`): `auto_update: bool`, default true, with an opt-out toggle like `telemetry`.
  When it's off, nothing runs automatically, but "Check now" still works.
- Register the plugin in `app/mod.rs` next to the others (`.plugin(tauri_plugin_updater::Builder::new().build())`).
  It's registered on Linux too, where it's simply unused.

### 4. Tray and UI
- `src-tauri/src/app/tray.rs` `menu()`: when the status is `Ready`, add "Restart to update to {version}" above Quit.
  When it's `Available` (Linux), add "Download Blankey {version}…", which opens the URL with `open`. Handle both in
  `on_menu`.
- `src-tauri/src/app/commands.rs`: `update_status`, `update_check`, `update_install` and `update_auto_set`, following
  `telemetry_status`/`telemetry_set`.
- `ui/js/pages/connections.js`, next to Statistics: a "Version" row showing the current version, the update status, a
  "Check now" button, a "Restart to update" button (or "Download" on Linux), and an "Automatic updates" checkbox.
- `ui/i18n/bg.json`: add the new strings (`scripts/i18n-check.mjs` enforces this).

### 5. Docs
- CHANGELOG entry for the next version. Copy this plan to `docs/auto-update-plan.md`.
- Known limitation, noted in the changelog: the macOS build is ad-hoc signed and its code signature changes with
  every build, so after an update macOS may ask for keychain access again before Touch ID unlock works, just like a
  manual reinstall today. Developer ID signing would fix this; it's out of scope.

## Verification
- Unit tests in `updates.rs` with English test data: manifest parsing and version comparison (newer, same, older,
  malformed), and the Linux "available" decision. No network and nothing slow.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `npm test`.
- Check/download flow: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir and `BLANKEY_UPDATE_URL`
  pointing at a local `latest.json` (served by `python3 -m http.server`) whose version is higher. Confirm the
  notification, the tray item and the Connections page row.
- Full install (needs bundled apps): run `release.yml` with a prerelease test tag twice, for example 0.2.6-rc.1 and
  then rc.2. Install rc.1 and point it at rc.2's `latest.json` through `BLANKEY_UPDATE_URL`. Confirm it downloads,
  restarts into rc.2, and that the vault and keychain unlock still work on macOS and Windows (NSIS, and MSI if used).
