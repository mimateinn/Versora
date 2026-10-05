# Desktop feature map

| Module / feature | Files / layer | Contract | Dependencies / callers |
| --- | --- | --- | --- |
| Numbered framing and source shape | `crates/versora-core/src/codec.rs`, pure | Shape, numbered, parse_numbered, DedupPlan | Runtime translation adapter |
| Terminology and safe output names | core `glossary.rs`, `names.rs`, pure | Term, glossary block, validated tags/ids | Native store and jobs |
| Real API and CLI transports | engine `providers.rs`, `cli.rs`, adapters | ProviderConfig, ProviderError, CancelToken, complete/probe | Translation router; async HTTPS/native processes |
| Bounded recovery, ordering and gates | engine `translate.rs`, runtime | TranslationOptions, translate_texts | Core framing plus provider adapters |
| Format extraction and writeback | engine `formats/`, adapters | Document, extract/write_document | Jobs; no Python or disk mutation in format logic |
| Preferences, credentials, projects and migration | native `store.rs`, persistence | IPC.md settings and provider state | DPAPI, allowlisted user-data paths |
| Native file selection and jobs | native `jobs.rs`, commands | Registered input paths; job snapshots | Formats, providers, atomic new output writes |
| Native lifecycle and updater boundary | native `main.rs` | Tauri command allowlist; shutdown receipts | Jobs/store; native window |
| Desktop UI and user journey | `ui/`, static frontend | `IPC.md`; backend.js only | Native Rust commands; 12 locale catalogs/design tokens |
| Windows distribution | `scripts/`, packaging | EXE, NSIS installer, hashes, build metadata | Existing compiler/SDK/NSIS; no Python runtime |

The Python PR4 source remains behavior reference at 79af2e7. It is not bundled or invoked by the desktop product. Planned OpenCC/filename translation and translation memory are outside current implemented parity. Heavy-format fidelity, paid live compatibility and CLI tool isolation require explicit evidence; parser/unit passes are not their sign-off.

Distribution gates: use a Windows GUI release build with explicit Tauri custom-protocol and static CRT, then inspect its actual PE imports. The installer uses the existing NSIS toolchain and per-user install scope. Detect the preinstalled Evergreen WebView2 prerequisite; downloading a runtime requires separate authorization. Preserve both `%LOCALAPPDATA%/Versora` and Tauri's `%LOCALAPPDATA%/com.mimateinn.versora` cache on upgrade/uninstall. Never include developer/test profiles, generated translations or credentials in package or source commits.

The original repository does not declare a project license. The new Cargo manifests intentionally make no additional license grant. Published installers must include actual third-party license notices and the corresponding cached sources required by their licenses, without assigning new terms to the original application/UI.
