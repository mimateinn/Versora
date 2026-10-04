# v0.2 verification

> Read when reviewing the v0.2 UI; frozen technical evidence and limits; keep below 20 KB.

## Scope

Verified on Windows, 2026-10-05, with Streamlit 1.65.0 and the offline **Demo** provider.
Product source: `0281bd2985c14f12b9cf466dbb021cc1f9f7d97c` on `feat/versora-ui`.
All 50 frozen source/test/locale manifest entries still match (0 mismatches).
Final image and README commits do not change this product source.
This is technical delivery **pending owner acceptance**, not a release or merge approval.

## Executed gates

```text
compileall exit=0
85 passed, 64 warnings in 18.80s
```

The full suite ran with `.venv\Scripts\python -m pytest -q tests`. The warnings are reported, not hidden.
`scripts/check_alignment.py` passed **16/16** light/dark cases. Its final lines were:

```text
ok  dark  batch done: 4 blocks, 13 controls
      geometry edges=stHorizontalBlock:288.0..1152.0, st-key-card_main:288.0..1152.0, st-key-card_result:288.0..1152.0, DIV:288.0..1152.0; gaps=[24, 24.0, 24.0]; heights=[34, 42]; row_deltas=[0]; spill=0
PASS
```

Each product-code change was followed by terminating the old 8511 listener and restarting with `SFTS_DEMO=1`, Demo selected, before visual judgment. Final captures also used a fresh server. `scripts/shoot.py` passed **34/34** states. Shared edges are x288–1152, section gaps 24px, controls 34/42px, measured row deltas and overflow zero. Running/completed title x stays 341.

## Screenshot coverage

All 34 state images are original **1440 × 900** browser captures, 17 per theme:

- Input: `idle`, `file-chosen`.
- Running: `translating-file`, `translating-batch`, `reduced-motion-file`.
- Results/recovery: `file-done`, `file-error`, `batch-done`, `batch-error`, `batch-partial`, `batch-retry`.
- Settings: `settings-purposes`, `settings-keys` (visible label **Translators**), `settings-add-online`, `settings-order`, `settings-glossary`, `settings-appearance`.

Each stem has `-light.png` and `-dark.png` variants here. Five byte-identical README aliases are `translate-light.png`, `translate-dark.png`, `translating.png` (batch/light), `batch-done.png` and `settings.png`. Earlier `ui-*.png` captures are retained, not represented as final v0.2 evidence.

The owner comparison preserves v0.1 pixels and displays old/new side by side; old captures have a different viewport. The root personally inspected all 34 r5 state captures and eight feedback captures, plus the eight key final idle/running/done/settings captures after the unchanged-source final re-shoot.

## Independent review

The owner explicitly replaced Sonnet with **GPT-6.1 Sol** after Claude quota exhaustion. Four brand-new, fresh-context, read-only critics received the requirements, L1–L5 locks, frozen images and raw evidence, not previous scores. Every critic individually opened all 42 r5 PNGs. Alignment counted in every lens.

| Lens | Score | Verdict |
| --- | --- | --- |
| Looks / Litora consistency | 8/10 | PASS |
| Motion and feedback | 8/10 | PASS |
| Ease of use | 8/10 | PASS |
| Looks like an AI template | 8/10 | PASS |

An independent fresh-context GPT-6.1 Sol code reviewer returned **PASS for r5 changed-code scope**, recomputing all 50 manifest entries with zero mismatches. The reviewer did not rerun tests or live providers. PASS does not certify the whole security contract or release.

Non-blocking observations remain: dark prose weight, uneven disabled styling, field-focus consistency, silent clipboard rejection, damaged-input retry emphasis, partial-download wording and old authentication breadcrumbs. They are preserved in the raw reviews rather than disguised as fixed.

## Browser feedback

- Busy fields, navigation and actions are disabled; Cancel stays usable. Actual Cancel and Cancelling states were observed in both themes.
- Cancelling a batch after one file finished reports **Stopped / 1 saved / 7 not finished / 1 skipped**, with one retry for outstanding files; it does not claim All done. Saved output is retained.
- Both single-file browser downloads are 604 bytes and byte-equal to disk. Stopped-batch ZIPs contain the saved file only. Native Chromium artifacts, SDK copies and disk outputs were checked; no HTTP substitute was used.
- Reduced motion has zero running infinite loops in both themes. The keyboard button ring is 2px with a 3px offset.

## Limits — read before release

**Known Medium baseline residual:** the Codex CLI preset selects a read-only sandbox but retains shell tools and inherited MCP configuration. Main `6fb88b4` already had this boundary. The original no-tool-access requirement is **not satisfied**. No actual credential disclosure/exploitation was demonstrated; that scenario was NOT_RUN. Close it before claiming translation-only tool isolation. Claude's tool-disable configuration is not evidence that Codex is similarly isolated.

**NOT_RUN in this takeover:** live API credentials/responses, live CLI compatibility and process-tree cancellation, real-model translation quality, native output-folder launch, portable packaging, comprehensive keyboard/screen-reader use, other locales/viewports and owner acceptance. Historical live-provider notes are not current proof. Synchronous HTTP and local extraction/parsing retain existing cancellation ceilings; instantaneous abort and universal format fidelity are not claimed. Keys remain plaintext in local `.env`, as the UI states.

Raw gates, feedback, manifest, reviewer identities and verbatim results live in the project `05 Notes/gauntlet/r5/`; final capture output is in `05 Notes/gauntlet/final/`. The owner delivery includes copies in its `outputs/evidence/` folder. No PR merge or later Chinese-conversion/filename feature work is included.
