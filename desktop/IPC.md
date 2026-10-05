# Desktop IPC contract

The static UI lives in the native Tauri WebView2 window. It uses `window.__TAURI__.core.invoke` only through `ui/backend.js`. No HTTP server, browser fallback, Python subprocess, general shell or unrestricted frontend filesystem API is present.

User preferences retain the legacy snake_case names: `theme`, `ui_lang`, `ui_lang_follow`, `provider`, `model_by_provider`, `chain` ({id,model,effort,enabled}), `concurrency`, `per_provider`, `purpose`, `project`, `source_type` (`file|folder|zip`), `content_mode`, `target_lang`, `source_choice`, `target_other`. Rust sanitizes/merges them; unknown values never reach process argv.

| Command | Arguments | Result |
| --- | --- | --- |
| `get_state` | none | `{settings,providers,purposes,projects,glossary,version,dataDir,testMode,selected,job}` |
| `pick_files` | `{kind:'files'|'folder'|'zip'}` | Native dialog, then array `{path,name,size,kind}`; paths registered by Rust |
| `clear_selection` | none | Clear selected inputs |
| `choose_output_dir` | none | Native folder dialog; path or null |
| `save_settings` | `{settings: preferencePatch}` | Clean merged settings |
| `save_provider` | `{provider:{id,model?,effort?,apiKey?,baseUrl?,cliPath?}}` | Masked provider status; keys never returned |
| `delete_provider` | `{id}` | Remove saved credential, masked state |
| `probe_provider` | `{id}` | Native tool/key availability (no paid call) |
| `test_provider` | `{id}` | Explicit user-clicked real transport test; paid request may occur |
| `create_project` | `{name}` | Canonical project id, existing terms preserved |
| `load_glossary` | `{project}` | Array `{term,translation}` |
| `save_glossary` | `{project,entries}` | Saved canonical project terms |
| `save_purpose` | `{purpose:{id:'custom',instructions}}` | New custom prompt version |
| `import_legacy_data` | none | Native folder chooser and non-destructive copy/import result |
| `start_translation` | `{request:{inputPaths?,sourceLanguage?,targetLanguage,purposeId,providerId,model?,project?,outputDir?,mode?}}` | `{jobId}`; input paths must belong to native registered selection; source files unchanged |
| `get_job` | `{jobId}` | Job snapshot |
| `cancel_job` | `{jobId}` | Request cancellation, preserve actual completed files |
| `retry_job` | `{jobId}` | New job id for failed/not-finished inputs only |
| `open_path` | `{path}` | Open an owned output path in Explorer/native associated app |
| `export_result` | `{jobId,zip:boolean}` | Native save dialog, export actual completed file or completed-files ZIP |

Providers returned to UI: `{id,name,kind,configured,status,model,effort,enabled,keyPresent,cliPath,baseUrl,detail}`. Secrets remain backend-only and will use Windows user-scope DPAPI storage. Purposes: `{id,version,instructions}`. No deletion of shipped prompt presets.

Job snapshot: `{id,status,total,done,saved,failed,skipped,current,message,outputDir,files:[{input,name,relative,status,output?,error?}]}`. Status is `running|cancelling|done|stopped|error`; per-file state is `pending|running|saved|failed|skipped|cancelled`. UI may poll `get_job` at 200ms, and Rust also emits `job-progress` with the same snapshot. Controls remain disabled while running/cancelling except Cancel.

Opt-in Demo is enabled only by `SFTS_DEMO=1` (or explicitly documented test setting), returns tagged test text and never claims real translation. Production has no fake provider fallback. Desktop regression runs use a separate `VERSORA_DATA_DIR` test directory and native WebView2 CDP; no normal browser page substitutes for native launch.

Installed executable/resources will be separated from `%LOCALAPPDATA%/Versora` user data. The updater must preserve all user data, stop the app before replacing binaries, and honor release channel/version order. Final shutdown/update receipt contract is documented after executable integration, not implied by this IPC design.

