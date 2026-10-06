# Native desktop UI regression

`native_journey.py` attaches to the actual executable's WebView2 CDP context. It
rejects ordinary browser pages and absent Tauri IPC. Existing Playwright is a
development dependency only; none of this tooling is shipped with the app.

Run the executable with a fresh `VERSORA_DATA_DIR`, explicit `SFTS_DEMO=1`, and
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`. Launch the
script using the existing Python/Playwright environment with `--evidence`,
`--data-dir`, and optionally `--export-path`. Launch `versora.exe <fixturefile>`
through the same legitimate Open With entry used for ordinary files. Use
`--manual-dialogs` to exercise the actual Windows picker and Save As dialogs.
The script never injects a pretend provider, replaces the Tauri API, starts a
web server, or bypasses input registration.

`launch_native.ps1 -FixtureType chunks` writes an ordinary structured JSON
document and passes it through the executable's real Open With entry. Pair it
with `native_journey.py --expect-chunks` to observe intermediate Rust chunk
counts and the single-file progress bar before completion. The journey hashes
registered source files before and after translation and records output SHA256.

The journey covers native startup; all six Settings panes in light/dark;
all 12 languages loaded with matching catalog headings; glossary creation and actual persistence; versioned custom
instructions; blank API credential submission; retained translation choices;
42px quick-bar controls; median-of-three navigation time; and a complete
offline Demo file-selection and actual disk-output flow. The journey reports
picker and Save As as `NOT_RUN` unless `--manual-dialogs` is used and the actual
dialogs are handled; separate native helper results have their own evidence.
`--release-build` enforces
the 500ms navigation median gate only when testing the compiled release. Updates
policy persistence and the truthful disabled download/install gates (no trust key
committed) are verified. Signed download/verification/install handoff is covered by
`src-tauri/tests/update_policy.rs` and the CI installer handoff check; health rollback
remains unverified.

Demo is explicitly a tagged offline test, not evidence of real translation.
Live provider compatibility, quotas, model quality and CLI tool restrictions
require separate authorized tests. Batch failure/retry and cancellation use
the same real Rust commands; test them with a multi-file fixture and a slow
test transport while leaving the actual model/provider claims separate.

`progress_state.test.mjs` checks the titlebar strip's actual state selector with
Node's built-in test runner. It covers startup, busy responses, single-file
chunks, batch files, completion/error/stop, cancellation and updater byte counts.
Run `node --test desktop/tests/ui/progress_state.test.mjs` from the repository root.

`launch_progress_segments.ps1` launches a digest-pinned checkout executable with
a new private Demo/WebView profile and an ordinary Open With JSON input.
`native_progress_segments.py --launch-proof <proof> --evidence <new-output-folder>`
then checks native progress and cancellation, the permanent 2px titlebar slot on
all six settings panes, actual 800x640/1060x800 HWND resizing, retained source
buttons, interrupted transitions, latest-click persistence, stable widths and
4px vertically centered icon/label spacing. It checks the app's Reduce Motion
preference and a WebView CSS reduced-motion request without changing Windows
display settings. Add `--baseline` only with the pinned original build to record
the old behavior. Full-window PNGs use the existing guarded PrintWindow helper;
transition samples are actual computed positions. All evidence remains ignored.

Manual owner journey: open Versora from its desktop shortcut, choose an existing
file in the Windows chooser, configure a translator in Settings, translate,
then save the result with Save As or open the output folder. The app stays in
its own desktop window and keeps original input files unchanged.

`native_checkpoint.py` records a native startup or offline Demo result, including
the authoritative Rust `usedDemo` flag. `capture_native_window.ps1` verifies the
owned executable, PID, title and HWND before using PrintWindow to capture the
actual titlebar, frame and WebView pixels. It never adds a frame or composites
images. `resize_native_minimum.ps1` changes the actual owned window size;
`native_minimum.py` checks all six panes at that native size without viewport
emulation.

`native_dialogs.ps1` uses scoped .NET UIAutomation on an owned Windows file
dialog when the Computer Use JavaScript API is unavailable. It refreshes the
process and dialog identity before actions. It does not confirm overwrite
warnings. Its Save As route delegates to the focused verified helper below.
A successful custom-path save must not be inferred from ValuePattern accepting
text; the shell can retain its cached filename and MRU directory. Record actual cancellation and backend responsiveness
separately from a successful exported file and its hash.

For focused Save As checks, `inspect_dialog_controls.ps1` records editor ancestry,
handles and the current-folder breadcrumb. `choose_native_save.ps1` requires the
actual `FileNameControlHost` editor and verifies text through WM_GETTEXT, focus
commit and the target folder. `-NativeCharacters` addresses WM_CHAR only to that
freshly verified owned Edit HWND, so shell filename changes are observed without
global keyboard input. Navigate to an existing synthetic directory first, then
set its basename. `native_save_as_journey.py` verifies cancellation, actual native
result feedback and disk hashes. Collision checks must also verify that the old
synthetic file hash is unchanged and that the returned suffixed path is different.
`confirm_native_overwrite.ps1` defaults to No; its explicitly authorized synthetic
fixture branch checks the exact folder, prompt basename and registered original
hash before confirmation. No user file is eligible for that branch.

`font_assets_metadata.py` checks static TrueType family/style/weight, glyph samples
and official WOFF2 signatures using the standard library. `native_font_journey.py`
attaches to the actual native WebView2, loads all four bundled Libron faces and
records rendered platform-font evidence. It checks twelve locales across all six
Settings panes and translation controls in light/dark at an actually resized
800 logical-pixel client window, including visible button text bounds. Offscreen glyph probes are
removed before product screenshots. No viewport/DPI emulation is used. Record
actual Windows DPI separately; unavailable monitor scales remain NOT_RUN.
Capture/resize helpers accept the exact approved production install outside the
checkout only with both `ExpectedExecutable` and `ExpectedExecutableSHA256`;
default checkout identity checks remain enforced. The launcher uses a separate
WebView profile under the fresh test evidence profile to avoid sharing an active
application's engine state.

Keep run-specific checkpoints under the excluded `evidence/` directory. Retain
failed checkpoints, then increment the evidence directory on a retry. Evidence profiles,
WebView data, fixture outputs and machine-specific process records are local test
material and must be excluded from source commits and downloadable packages.

## Custom titlebar regression

`native_chrome_journey.py --cdp <loopback-endpoint> --launch-proof <launch-proof.json>
--evidence <private-output-folder>` validates the actual Tauri window: minimize,
maximize/restore, monitor work area, blank-header drag/double-click, interactive
drag exclusions, eight native resize directions and the configured 800x640
minimum. It also checks the reserved bottom-left footer and scrolling content,
light/dark chrome and the native unsaved-close Cancel/confirm path. Each Win32
input action checks the owned executable digest, creation time and HWND.
Captures use real PrintWindow pixels. Windows Snap Layout hover is not certified
for the HTML caption buttons.

The first Translators-pane entry performs only native CLI version/login probes.
Manual Recheck remains available. A successful login does not establish a
transport connection; only a successful explicit Test can display the connected
status. Test may use provider quota and is never called automatically. Four
action positions are reserved so status updates do not move buttons.

The UI uses the traced local Litora 12px capsule scrollbar, 280ms color-only theme
transition, 180ms control release/80ms press, 220ms route entry and 200ms card
fade. Theme changes retain the current page, draft controls and focus. The existing
HTML confirmation uses a 200ms backdrop and 380ms sheet entrance; native Windows
close confirmations retain their own platform behavior. Settings selection markers
interpolate for 180ms on retained rail controls. The reserved footer wraps naturally
without entering the content scroller. The explicit Reduce Motion preference keeps a 160ms route-only fade;
the OS reduced-motion request disables it. Runtime evidence and screenshots
belong in the ignored evidence directory, not the source package.
