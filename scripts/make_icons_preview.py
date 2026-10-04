"""Write assets/icons-preview.html: every src/icons.py glyph at 16/20/24 (+48) on light and dark,
with hover / appear / action states, plus the static and animated app icon. Dev-only page.

Run: .venv/Scripts/python scripts/make_icons_preview.py
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from src.icons import ICON_CSS, ICONS  # noqa: E402

OUT = ROOT / "assets" / "icons-preview.html"

# which state column demonstrates which contract class
ACTION = {"check": "vi-anim-check", "alert": "vi-anim-alert", "swap": "vi-play", "sun": "vi-play",
          "moon": "vi-play"}


# filmstrip: (icon, trigger class on the host, frame times in ms)
FILM = [
    ("sun", "is-hover", (0, 60, 120, 180, 240, 320)), ("sun", "vi-play", (0, 60, 120, 180, 260, 380)),
    ("moon", "is-hover", (0, 60, 120, 180, 240, 320)), ("moon", "vi-play", (0, 60, 120, 180, 260, 380)),
    ("check", "vi-anim-check", (0, 80, 160, 240, 320, 500)), ("alert", "vi-anim-alert", (0, 60, 120, 180, 260, 360)),
    ("swap", "is-hover", (0, 60, 120, 180, 240, 320)), ("swap", "vi-play", (0, 60, 120, 180, 260, 380)),
    ("download", "is-hover", (0, 60, 120, 180, 240, 320)), ("upload", "is-hover", (0, 60, 120, 180, 240, 320)),
    ("retry", "is-hover", (0, 60, 120, 180, 260, 380)), ("close", "is-hover", (0, 40, 80, 120, 180, 240)),
    ("plus", "is-hover", (0, 40, 80, 120, 180, 240)), ("settings", "is-hover", (0, 60, 120, 180, 240, 320)),
    ("folder_open", "is-hover", (0, 60, 120, 180, 240, 320)), ("file", "vi-anim-in", (0, 40, 80, 120, 180, 240)),
    ("spinner", "", (0, 150, 300, 450, 600, 750)), ("dot", "", (0, 260, 520, 780, 1040, 1300)),
]


def film() -> str:
    rows = []
    for name, cls, times in FILM:
        cells = "".join(
            f'<td><span class="vi-host frame" data-cls="{cls}" data-t="{t}">{sized(ICONS[name], 48)}</span>'
            f"<small>{t}</small></td>"
            for t in times
        )
        rows.append(f"<tr><th>{name}<br><small>{cls or 'always'}</small></th>{cells}</tr>")
    return f'<section class="panel light film"><h2>motion, frozen frames (ms)</h2><table>{"".join(rows)}</table></section>'


def sized(svg: str, px: int) -> str:
    return svg.replace('width="16" height="16"', f'width="{px}" height="{px}"', 1)


def panel(theme: str) -> str:
    rows = []
    for name, svg in ICONS.items():
        act = ACTION.get(name, "vi-anim-in")
        rows.append(
            f'<tr><th>{name}</th>'
            + "".join(f"<td>{sized(svg, px)}</td>" for px in (16, 20, 24))
            + f'<td><span class="vi-host is-hover">{sized(svg, 24)}</span></td>'
            + f'<td><span class="replay {act}" data-cls="{act}">{sized(svg, 24)}</span></td>'
            + f"<td>{sized(svg, 48)}</td></tr>"
        )
    head = "<tr><th></th><th>16</th><th>20</th><th>24</th><th>hover</th><th>appear / action</th><th>48</th></tr>"
    return f'<section class="panel {theme}"><h2>{theme}</h2><table>{head}{"".join(rows)}</table></section>'


PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Versora icons preview</title>
<style>
body {{ margin: 0; font: 13px/1.4 "Segoe UI", system-ui, sans-serif; background: #F4F2EC; color: #222; }}
header {{ padding: 16px 20px; display: flex; gap: 24px; align-items: center; flex-wrap: wrap; }}
header img {{ display: block; }}
.panels {{ display: flex; gap: 0; flex-wrap: wrap; }}
.panel {{ padding: 12px 20px 20px; flex: 1 1 420px; }}
.panel.light {{ background: #F4F2EC; color: #2B2B2E; --vi-accent: #4677B5; }}
.panel.dark {{ background: #1B1F27; color: #E6E2D6; --vi-accent: #6E97CF; }}
h2 {{ margin: 4px 0 8px; font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: .06em; opacity: .6; }}
table {{ border-collapse: collapse; }}
th {{ font-weight: 500; text-align: left; padding: 4px 12px 4px 0; opacity: .7; }}
td {{ padding: 4px 12px; text-align: center; vertical-align: middle; }}
td svg {{ display: inline-block; vertical-align: middle; }}
.replay, .vi-host {{ display: inline-block; }}
.film td {{ padding: 4px 10px; }} .film small {{ display: block; opacity: .5; font-size: 11px; }}
button {{ font: inherit; padding: 6px 12px; border-radius: 8px; border: 1px solid #ccc; background: #fff; }}
{css}
</style></head><body>
<header>
  <img src="icon.svg" width="128" height="128" alt="static icon">
  <img src="icon-animated.svg" width="128" height="128" alt="animated icon">
  <img src="icon.svg" width="32" height="32" alt=""><img src="icon.svg" width="16" height="16" alt="">
  <button id="replay" type="button">Replay appear / action</button>
</header>
<div class="panels">{light}{dark}</div>
{film}
<script>
document.getElementById('replay').addEventListener('click', () => {{
  document.querySelectorAll('.replay').forEach((el) => {{
    const c = el.dataset.cls; el.classList.remove(c); void el.offsetWidth; el.classList.add(c);
  }});
}});
// filmstrip: start each frame's trigger, then freeze it at its own time
document.querySelectorAll('.frame').forEach((el) => {{
  const c = el.dataset.cls, svg = el.querySelector('svg');
  if (c === 'vi-anim-in' || c === 'vi-anim-check' || c === 'vi-anim-alert' || c === 'vi-play') {{
    void el.offsetWidth; el.classList.add(c);
  }} else if (c) {{ void el.offsetWidth; el.classList.add(c); }}
  void el.offsetWidth;
  svg.getAnimations({{ subtree: true }}).forEach((a) => {{ a.pause(); a.currentTime = Number(el.dataset.t); }});
}});
// ?t=MS freezes every hover / appear / action animation MS milliseconds in (for screenshots).
const t = new URLSearchParams(location.search).get('t');
if (t !== null) {{
  const hov = [...document.querySelectorAll('.is-hover')], rep = [...document.querySelectorAll('.replay')];
  hov.forEach((el) => el.classList.remove('is-hover'));
  rep.forEach((el) => el.classList.remove(el.dataset.cls));
  void document.body.offsetWidth;
  hov.forEach((el) => el.classList.add('is-hover'));
  rep.forEach((el) => el.classList.add(el.dataset.cls));
  document.getAnimations().forEach((a) => {{ a.pause(); a.currentTime = Number(t); }});
}}
</script>
</body></html>
"""


def main() -> None:
    OUT.write_text(PAGE.format(css=ICON_CSS, light=panel("light"), dark=panel("dark"), film=film()), encoding="utf-8")
    print(OUT.relative_to(ROOT), OUT.stat().st_size)


if __name__ == "__main__":
    main()
