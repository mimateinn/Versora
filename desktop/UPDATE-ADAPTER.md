# Rust desktop updater integration

This adapts the suite semantics in the other task's `UPDATE-CONTRACT.md`; it does not import the historical Python updater or change Litora.

| Required fact | Native desktop contract |
| --- | --- |
| Framework | Tauri **2.11.5**, WebView2; static embedded frontend; Rust IO/providers; no Python or HTTP server |
| App version | Cargo workspace **0.3.0-preview.1**, `env!("CARGO_PKG_VERSION")`; Tauri config must match |
| Windows distribution | Existing NSIS produces `Versora-<strict-semver>-win32-x64-setup.exe`; EXE `versora.exe` |
| Executable installation | Per-user `%LOCALAPPDATA%/Programs/Versora`; application files only |
| User data | `%LOCALAPPDATA%/Versora`; isolated `VERSORA_DATA_DIR` only for tests/explicit local profile |
| Preserve | projects/.sfts-ui.json, projects/*/glossary.json, credentials.dpapi, data/prompts/custom.md, data/outputs/, updater state/journal |
| Legacy import | Native chooser, original files untouched; recognized .env credentials converted to Windows user-scope DPAPI; no overwrite of current data |
| Renderer bridge | `updates_get_preferences`, `updates_set_preferences`, `updates_get_state`, `updates_check`, `updates_download`, `updates_cancel`, `updates_later`, `updates_request_install`, `updates_open_official_release`, `updates_health_ack` |
| Event | `suite-update-state-changed`, state JSON; GUI uses text, SVG and aria-live |
| Fixed source | owner `mimateinn`, repo `Versora`; HTTPS GitHub releases API, app-specific fixed official Releases page; no arbitrary renderer URLs/commands |
| Policy | startup/manual/periodic while app runs, 1-168h, default startup/stable/download off; strictly newer SemVer and highest healthy-startup marker |
| Safe exit | Cancel owned job, wait for actual cleanup/output persistence, then exit; busy/unsaved/pending-persistence guards preserve the app before installer handoff |
| Health hook | UI explicitly ACKs after successful state restore and first render; marker is not advanced by discovery/download |
| Trust | **No reviewed publisher public key provided. No automatic package download/install capability enabled.** |

Until trust, signed-version binding, bounded native download/cancel, real installer locks/privileges, recovery and healthy-startup gates are independently proven, use the normal NSIS installer/manual official release-page route. A checksum is integrity evidence, not publisher authentication. No production key/certificate is generated for this migration.

The future Tauri updater adapter must use native updater artifacts and signatures, an existing reviewed trust anchor and `requireSignedVersion`; it must not reuse Litora's historical RSA format. Download and install stay separate. Current metadata discovery/manual fallback does not certify signed downloading, installation or rollback.
