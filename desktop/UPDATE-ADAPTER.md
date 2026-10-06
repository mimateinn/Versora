# Rust desktop updater integration

This adapts the suite semantics in the other task's `UPDATE-CONTRACT.md`; it does not import the historical Python updater or change Litora.

| Required fact | Native desktop contract |
| --- | --- |
| Framework | Tauri **2.11.5**, WebView2; static embedded frontend; Rust IO/providers; no Python or HTTP server |
| App version | Cargo workspace **0.3.1**, `env!("CARGO_PKG_VERSION")`; Tauri config must match |
| Windows distribution | Existing NSIS produces `Versora-<strict-semver>-win32-x64-setup.exe`; EXE `versora.exe` |
| Executable installation | Per-user `%LOCALAPPDATA%/Programs/Versora`; application files only |
| User data | `%LOCALAPPDATA%/Versora`; isolated `VERSORA_DATA_DIR` only for tests/explicit local profile |
| Preserve | projects/.sfts-ui.json, projects/*/glossary.json, credentials.dpapi, data/prompts/custom.md, data/outputs/, updater state/journal |
| Legacy import | Native chooser, original files untouched; recognized .env credentials converted to Windows user-scope DPAPI; no overwrite of current data |
| Renderer bridge | `updates_get_preferences`, `updates_set_preferences`, `updates_get_state`, `updates_check`, `updates_download`, `updates_cancel`, `updates_later`, `updates_request_install`, `updates_open_official_release`, `updates_health_ack` (unchanged names; `updates_download` now starts a background download, `updates_request_install` accepts `confirmed`) |
| Event | `suite-update-state-changed`, state JSON (also emitted with throttled download progress); GUI uses text, SVG and aria-live |
| Fixed source | owner `mimateinn`, repo `Versora`; HTTPS GitHub releases API, app-specific fixed official Releases page; no arbitrary renderer URLs/commands |
| Policy | startup/manual/periodic while app runs, 1-168h; default **periodic every 4 h, first check ~10 s after the UI is healthy, stable, auto download+install on**; strictly newer SemVer and highest healthy-startup marker. Profiles still on the old default (startup/24 h) migrate once; explicit manual/periodic and channel choices are kept, and the never-enabled download toggle turns on |
| Safe exit | Cancel owned job, wait for actual cleanup/output persistence, then exit. Busy or unsaved work needs the user's in-app confirmation (`confirmed`); pending persistence always refuses. The verified installer is started from this path only: on request with `/S /UPDATE /RELAUNCH`, or silently on quit (`/S /UPDATE`) whenever a verified package is ready, however it was downloaded; a running check or download is cancelled first |
| Health hook | UI explicitly ACKs after successful state restore and first render; marker is not advanced by discovery/download. The ACK also reports `justUpdated` once (previous last-seen version < current) for the "已更新到 {v}" notice and prunes obsolete packages |
| Trust | Tauri signer (minisign) public key compiled from `src-tauri/updater-public-key.txt`. **The existing publisher key is committed; installed Production builds require a matching signed package.** Unpackaged/dev builds (no production `.versora-installation` marker beside `versora.exe`) never contact the network |

## Trust, download and install model

- **Discovery** is unchanged: the fixed GitHub releases API, strict SemVer, channel by pre-release, pinned asset
  `Versora-<v>-win32-x64-setup.exe` with its exact official download URL and size. The release `body` is kept as
  bounded plain text (8 KiB, control/bidi characters removed) for "有咩新"; the renderer only inserts it escaped.
- **Download** (only with a trust key, only in the installed app): `<asset>.sig` first (16 KiB cap; missing = error,
  release page offered), then the package streamed into an owned `updates/.download-*.part` file. The stream is
  bounded by the release size, cancellable (`updates_cancel`), stalls fail after 60 s, redirects only follow HTTPS
  GitHub hosts. Any failure deletes the partial file. A failed background download is retried on the metadata
  backoff ladder (1, 2, 4 … 60 min); an explicit cancel is not retried.
- **Verification**: `minisign-verify` 0.2.5 (the major used by `tauri-plugin-updater` 2.13.1), prehashed Ed25519
  only. The `.pub`/`.sig` files are base64 of minisign text, exactly as `tauri signer` writes them. After the global
  signature verifies, the signed trusted comment must name `version:<v>` (signed with `--app-version`) and, if it
  names a file, `file:Versora-<v>-win32-x64-setup.exe`. Only then is the package renamed into `updates/`.
- **Install**: the package and signature are re-read and verified again right before launch, with a Windows handle
  that denies write/delete sharing held until the installer process exists. The installer runs detached with
  `/S /UPDATE` (+ `/RELAUNCH`); `versora.nsi` then waits up to 30 s for `versora.exe` to be released (without
  `/UPDATE` it still refuses at once), installs through its unchanged transaction/rollback code and, with
  `/RELAUNCH`, starts the installed app as the user (after a failed update it reopens the restored version; when
  the update is refused before anything changed, such as `versora.exe` still locked after 30 s, it reopens the
  unchanged program).
- **Readiness**: a verified package stays ready (pill, install button, install on quit) through later metadata
  checks, failed or running; only a different candidate or a failed re-verification clears it.
- **State**: `updates/state.json` keeps schema 1 byte-compatible so an older build can still start after a manual
  downgrade. New facts (`lastSeenVersion`, release notes, migration marker) live in `updates/updater.json`.
- Not provided: Authenticode signing, automatic rollback after a bad update, delta updates.

## One-time key setup (Dickson, on Windows)

Use the existing keypair in `C:\Users\dicks\.tauri\` (do not generate a new one; never commit the private key):

1. Paste the contents of `C:\Users\dicks\.tauri\versora-updater.key.pub` (one base64 line) below the comments
   in `desktop/src-tauri/updater-public-key.txt` and commit it. Builds from that commit on can verify and install
   updates; older builds (including 0.3.0-preview.1) keep offering the release page.
2. In GitHub → mimateinn/Versora → Settings → Secrets and variables → Actions, add:
   - `TAURI_SIGNING_PRIVATE_KEY` = the full contents of `C:\Users\dicks\.tauri\versora-updater.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = the password from the password file next to it
3. Keep an offline backup of the `.key` file and password. Losing them means installed apps cannot accept new
   updates until users reinstall a build with a new public key.

## How to release

1. Set the same version in `desktop/Cargo.toml` (`[workspace.package] version`) and
   `desktop/src-tauri/tauri.conf.json`, update `Cargo.lock` (`cargo metadata` or a build), commit and push.
2. Tag and push: `git tag -a v0.4.0 -m "<release notes shown as 有咩新>"` then `git push origin v0.4.0`.
   A pre-release version (`v0.4.0-preview.1`) becomes a GitHub prerelease, offered only on 預覽版.
3. `.github/workflows/versora-release.yml` (windows-latest) checks the tag against both versions, runs the updater
   tests, builds the static-CRT release exe and the production NSIS installer (`Build-CiPackage.ps1` →
   `Build-WindowsPackage.ps1`), signs it with `npx @tauri-apps/cli@2.12.1 signer sign --app-version <v>`, verifies
   the signature with the app's own code against the committed key, and publishes a release with
   `Versora-<v>-win32-x64-setup.exe` and `.sig` (created as a draft, published after both uploads).
   Re-run for an existing tag via *Run workflow* with the tag name. The annotated tag message becomes the release
   notes (otherwise GitHub generates them); edit the release body afterwards if needed.

`versora-ci.yml` runs on `feat/rust-desktop` pushes, pull requests and manual dispatch: the UI source-freeze check, `cargo test`
(including `update_policy`), the release build, an isolated Test-mode installer and the `/UPDATE` wait and
`/RELAUNCH` handoff on the runner.
