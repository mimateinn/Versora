#!/usr/bin/env python3
"""Dev-only screenshots of the main UI states (same setup as check_alignment.py).

Start the app with the demo provider, slowed down so a run can be caught in progress:
    SFTS_DEMO=1 SFTS_DEMO_DELAY=3 streamlit run app.py --server.port 8511
    python scripts/shoot.py --out <dir> [--url http://localhost:8511/] [--themes light dark]

Writes <state>-<theme>.png at 1440x900, and runs check_alignment's measurements on each: idle, file-chosen, translating-file, file-done,
file-error, translating-batch, batch-done, settings-purposes, settings-keys, settings-order, settings-glossary, settings-appearance.
The provider must be Demo in prefs (Auto would pick a local CLI).
"""

from __future__ import annotations

import argparse
import sys
import tempfile
import zipfile
from pathlib import Path

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_alignment import ROOT, check, settle  # noqa: E402

SEG = ".st-key-source_type_seg button"  # 0 file, 1 folder, 2 zip
UPLOAD = "[data-testid=stFileUploaderDropzone] input[type=file]"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--url", default="http://localhost:8511/")
    ap.add_argument("--themes", nargs="+", default=["light", "dark"])
    args = ap.parse_args()
    out: Path = args.out
    out.mkdir(parents=True, exist_ok=True)
    tmp_context = tempfile.TemporaryDirectory(prefix="versora_shoot_")
    tmp = Path(tmp_context.name)
    sample = ROOT / "samples" / "release-notes.md"
    broken = tmp / "quarterly-report.docx"
    broken.write_bytes(b"not a real docx")  # extraction fails -> the single-file error card
    zipped = ROOT / "samples" / "game-mod.zip"
    partial = tmp / "mixed-documents.zip"
    with zipfile.ZipFile(partial, "w") as archive:
        archive.writestr("notes.txt", "Release meeting notes.\n")
        archive.writestr("damaged.docx", b"not a real docx")
        archive.writestr("art/logo.png", b"unsupported image")
    broken_zip = tmp / "damaged-archive.zip"
    broken_zip.write_bytes(b"not a real zip")
    bad: list[str] = []

    with sync_playwright() as p:
        browser = p.chromium.launch()
        for theme in args.themes:
            page = browser.new_page(viewport={"width": 1440, "height": 900})

            def fresh(query: str = "page=translate", mode: int | None = 0) -> None:
                page.goto(f"{args.url}?{query}&theme={theme}")
                page.wait_for_selector(".st-key-nav_translate", timeout=30000)
                settle(page)
                if mode is not None:
                    page.locator(SEG).nth(mode).click()
                    settle(page)

            def shot(name: str) -> None:
                page.mouse.move(1, 1)  # no hover state
                path = out / f"{name}-{theme}.png"
                page.screenshot(path=str(path))
                if check(page, path.name):
                    bad.append(path.name)

            def pick(path: Path) -> None:
                page.locator(UPLOAD).set_input_files(str(path))
                settle(page)

            def start(timeout: int = 30000) -> None:
                """Click Translate; a click that lands mid-rerun can be dropped, so click once more."""
                page.locator(".st-key-start_translate button").click()
                try:
                    page.wait_for_selector(".sfts-run, .st-key-card_error", timeout=10000)
                except Exception:
                    page.locator(".st-key-start_translate button").click()
                    page.wait_for_selector(".sfts-run, .st-key-card_error", timeout=timeout)

            def start_and_catch_run(name: str, wait_ms: int) -> None:
                start()
                page.wait_for_timeout(wait_ms)
                shot(name)

            fresh()
            shot("idle")
            pick(sample)
            shot("file-chosen")
            start_and_catch_run("translating-file", 900)
            busy_fields = page.locator(".st-key-quickbar [role=combobox]").all()
            assert len(busy_fields) == 4 and all(f.is_disabled() for f in busy_fields), "busy fields must be locked"
            run_title_x = page.locator(".sfts-run").evaluate("""e => {
                const text = [...e.childNodes].find(n => n.nodeType === Node.TEXT_NODE && n.textContent.trim());
                const r = document.createRange(); r.selectNodeContents(text); return r.getBoundingClientRect().x;
            }""")
            page.emulate_media(reduced_motion="reduce")
            page.wait_for_timeout(100)
            assert page.evaluate("document.getAnimations().every(a => a.playState !== 'running' || a.effect.getComputedTiming().iterations !== Infinity)"), "reduced motion must stop all loops"
            shot("reduced-motion-file")
            page.emulate_media(reduced_motion="no-preference")
            page.wait_for_selector(".st-key-card_result .sfts-done", timeout=180000)
            settle(page)
            shot("file-done")
            done_title_x = page.locator(".sfts-done-title").first.evaluate("e => e.getBoundingClientRect().x")
            assert abs(run_title_x - done_title_x) <= 1, f"status title jumps: {run_title_x:.1f} -> {done_title_x:.1f}"
            print(f"ok  {theme} run/done title: {run_title_x:.1f} -> {done_title_x:.1f}px")

            fresh()
            pick(broken)
            start()
            page.wait_for_selector(".st-key-card_error", timeout=60000)
            settle(page)
            page.locator(".st-key-card_error [data-testid=stExpander] summary").click()
            settle(page)
            shot("file-error")

            if zipped.is_file():
                fresh(mode=2)
                pick(zipped)
                start_and_catch_run("translating-batch", 4000)
                page.wait_for_selector(".st-key-card_result .sfts-done", timeout=300000)
                settle(page)
                shot("batch-done")

            fresh(mode=2)
            pick(partial)
            start()
            page.wait_for_selector(".st-key-retry_failed button", timeout=180000)
            settle(page)
            shot("batch-partial")
            before_retry = page.locator(".st-key-card_result").inner_text()
            page.locator(".st-key-retry_failed button").click()
            settle(page)
            page.wait_for_selector(".st-key-retry_failed button", timeout=180000)
            settle(page)
            shot("batch-retry")
            after_retry = page.locator(".st-key-card_result").inner_text()
            assert "1 saved" in before_retry and "1 saved" in after_retry, "retry must preserve the saved file"

            fresh(mode=2)
            pick(broken_zip)
            start()
            page.wait_for_selector(".st-key-card_error", timeout=60000)
            settle(page)
            shot("batch-error")

            for pane in ("purposes", "keys", "order", "glossary", "appearance"):
                fresh(f"page=settings&pane={pane}", mode=None)
                shot(f"settings-{pane}")
                if pane == "keys":
                    add = page.locator(".st-key-svc_add_openai button")
                    if add.count():
                        add.click()
                        settle(page)
                        shot("settings-add-online")

            fresh()  # leave the prefs on single-file mode
            page.close()
        browser.close()
    tmp_context.cleanup()
    print("PASS" if not bad else f"FAIL {bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
