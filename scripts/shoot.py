#!/usr/bin/env python3
"""Dev-only screenshots of the main UI states (same setup as check_alignment.py).

Start the app with the demo provider, slowed down so a run can be caught in progress:
    SFTS_DEMO=1 SFTS_DEMO_DELAY=3 streamlit run app.py --server.port 8511
    python scripts/shoot.py --out <dir> [--url http://localhost:8511/] [--themes light dark]

Writes <state>-<theme>.png at 1440x900, and runs check_alignment's measurements on each: idle, file-chosen, translating-file, file-done,
file-error, translating-batch, batch-done, settings-translation, settings-keys, settings-appearance,
settings-order (the Translators pane scrolled to its Order card).
The provider must be Demo in prefs (Auto would pick a local CLI).
"""

from __future__ import annotations

import argparse
import sys
import tempfile
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
    tmp = Path(tempfile.mkdtemp(prefix="versora_shoot_"))
    sample = ROOT / "samples" / "release-notes.md"
    broken = tmp / "quarterly-report.docx"
    broken.write_bytes(b"not a real docx")  # extraction fails -> the single-file error card
    zipped = ROOT / "samples" / "game-mod.zip"
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
            page.wait_for_selector(".st-key-card_result .sfts-done", timeout=180000)
            settle(page)
            shot("file-done")

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

            for pane in ("translation", "appearance", "keys"):  # keys last: the Order shot scrolls it
                fresh(f"page=settings&pane={pane}", mode=None)
                shot(f"settings-{pane}")
            page.locator(".st-key-card_order").scroll_into_view_if_needed()
            page.wait_for_timeout(300)
            shot("settings-order")

            fresh()  # leave the prefs on single-file mode
            page.close()
        browser.close()
    print("PASS" if not bad else f"FAIL {bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
