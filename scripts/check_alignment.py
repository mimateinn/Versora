#!/usr/bin/env python3
"""Dev-only layout check (needs `pip install playwright` + `playwright install chromium`).

Start the app first, with the demo provider so the batch state can run:
    SFTS_DEMO=1 streamlit run app.py --server.port 8511
    python scripts/check_alignment.py [--url http://localhost:8511/]

For each page in light and dark it measures, from bounding boxes:
  edges     top-level blocks share one left and one right edge (±1px)
  gaps      vertical gaps between top-level blocks are equal (±1px)
  heights   every visible control is 34 or 42px tall (±0.5px)
  rows      fields in one row start at the same top (±1px)
  spill     text blocks stay inside the box the layout gave them (±1px)
Exit code 1 if anything is off.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = (34, 42)

MEASURE = """() => {
  const vis = el => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden'; };
  const box = el => { const r = el.getBoundingClientRect(); return {x: r.left, r: r.right, y: r.top, b: r.bottom, h: r.height}; };
  const main = document.querySelector('[data-testid="stMainBlockContainer"] > div > [data-testid="stVerticalBlock"]')
            || document.querySelector('[data-testid="stMainBlockContainer"] [data-testid="stVerticalBlock"]');
  // measure what is drawn: the card / column row / markdown body inside each wrapper
  const inner = el => el.matches('[class*="st-key-card_"]') ? el
    : (el.querySelector('[class*="st-key-card_"], [data-testid="stHorizontalBlock"], [data-testid="stMarkdownContainer"] > div') || el);
  const blocks = [...main.children].filter(el => vis(el) && getComputedStyle(el).position !== 'absolute')
    .map(inner).map(el => ({name: (el.className.match(/st-key-[\\w-]+/) || [el.dataset.testid || el.tagName])[0], ...box(el)}));
  const ctrlSel = ['[data-testid^="stBaseButton"]', '[data-testid="stSelectbox"] [role="group"]',
    '[data-testid="stTextInputRootElement"]', '[data-testid="stButtonGroup"] [role="radiogroup"]', '.sfts-filechip'];
  const controls = [];
  for (const sel of ctrlSel) for (const el of document.querySelectorAll(sel)) {
    if (!vis(el) || el.closest('[data-testid="stToast"]')) continue;
    const key = (el.closest('[class*="st-key-"]')?.className.match(/st-key-[\\w-]+/) || [sel])[0];
    controls.push({name: key, ...box(el)});
  }
  const rows = [];
  for (const row of document.querySelectorAll('[class*="st-key-quickbar"] [data-testid="stHorizontalBlock"]')) {
    const tops = [...row.querySelectorAll('[data-testid="stSelectbox"] [role="group"], [data-testid^="stBaseButton"]')]
      .filter(vis).map(el => el.getBoundingClientRect().top);
    if (tops.length) rows.push(tops);
  }
  // header row: brand, tabs and theme button share one centre line
  const mid = el => { const r = el.getBoundingClientRect(); return r.top + r.height / 2; };
  const head = ['.sfts-brand', '.st-key-nav_translate button', '.st-key-nav_settings button', '.st-key-theme_toggle button']
    .map(s => document.querySelector(s)).filter(Boolean).map(mid);
  if (head.length) rows.push(head);
  // a block's drawn content must sit inside the box the layout reserved for it
  const spill = [];
  for (const md of document.querySelectorAll('[data-testid="stElementContainer"] [data-testid="stMarkdownContainer"]')) {
    if (md.closest('button, [data-testid="stSlider"]') || !vis(md)) continue;
    const host = md.closest('[data-testid="stElementContainer"]');
    if (getComputedStyle(host).position === 'absolute') continue;
    const a = md.getBoundingClientRect(), h = host.getBoundingClientRect();
    if (a.bottom > h.bottom + 1 || a.top < h.top - 1)
      spill.push(`${(md.textContent || '').trim().slice(0, 24)}: content ${a.top.toFixed(0)}..${a.bottom.toFixed(0)}, box ${h.top.toFixed(0)}..${h.bottom.toFixed(0)}`);
  }
  return {blocks, controls, rows, spill};
}"""


def check(page, label: str) -> list[str]:
    m = page.evaluate(MEASURE)
    bad = []
    blocks = m["blocks"]
    if blocks:
        left, right = blocks[0]["x"], blocks[0]["r"]
        for b in blocks:
            if abs(b["x"] - left) > 1 or abs(b["r"] - right) > 1:
                bad.append(f"edges   {b['name']}: x={b['x']:.1f}..{b['r']:.1f}, page {left:.1f}..{right:.1f}")
        gaps = [round(n["y"] - p["b"], 1) for p, n in zip(blocks, blocks[1:])]
        if gaps and max(gaps) - min(gaps) > 1:
            bad.append(f"gaps    between top-level blocks: {gaps}")
    for c in m["controls"]:
        if not any(abs(c["h"] - a) <= 0.5 for a in ALLOWED):
            bad.append(f"heights {c['name']}: {c['h']:.1f}px")
    for s in m["spill"]:
        bad.append(f"spill   {s}")
    for tops in m["rows"]:
        if max(tops) - min(tops) > 1:
            bad.append(f"rows    tops/centres differ: {[round(t, 1) for t in tops]}")
    print(f"{'ok ' if not bad else 'BAD'} {label}: {len(blocks)} blocks, {len(m['controls'])} controls")
    for line in bad:
        print("      " + line)
    return bad


def settle(page) -> None:
    page.wait_for_timeout(500)
    page.wait_for_function(
        "!document.querySelector('[data-testid=stStatusWidget] [data-testid=stStatusWidgetRunningIcon]')",
        timeout=60000,
    )
    page.wait_for_selector(".sfts-footer", timeout=30000)
    # entrance animations move boxes by 4px; wait until every finite one has ended
    page.wait_for_function(
        "document.getAnimations().every(a => a.playState !== 'running' || a.effect.getComputedTiming().iterations === Infinity)",
        timeout=10000,
    )
    page.wait_for_timeout(200)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://localhost:8511/")
    args = ap.parse_args()
    failures = 0
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for theme in ("light", "dark"):
            page = browser.new_page(viewport={"width": 1440, "height": 900})
            for query in ("page=translate", "page=settings&pane=appearance", "page=settings&pane=translation",
                          "page=settings&pane=keys", "page=settings&pane=glossary"):
                page.goto(f"{args.url}?{query}&theme={theme}")
                page.wait_for_selector(".st-key-nav_translate", timeout=30000)
                settle(page)
                failures += bool(check(page, f"{theme:5} {query}"))
            sample = ROOT / "samples" / "game-mod.zip"
            if sample.is_file():
                page.goto(f"{args.url}?page=translate&theme={theme}")
                page.wait_for_selector(".st-key-source_type_seg button", timeout=30000)
                settle(page)
                page.locator(".st-key-source_type_seg button").nth(2).click()
                settle(page)
                page.locator("[data-testid=stFileUploaderDropzone] input[type=file]").set_input_files(str(sample))
                settle(page)
                failures += bool(check(page, f"{theme:5} zip picked"))
                page.locator(".st-key-start_translate button").click()
                page.wait_for_selector(".sfts-done", timeout=120000)
                settle(page)
                failures += bool(check(page, f"{theme:5} batch done"))
            page.close()
        browser.close()
    print("PASS" if not failures else f"FAIL ({failures} page states)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
