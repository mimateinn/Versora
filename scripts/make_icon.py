"""Rasterise the Versora app icon from `assets/icon.svg`.

Writes:
  icon.png               256 px RGBA (the app + README load this; keep the path)
  assets/icon-512.png    512 px RGBA
  assets/favicon.ico     16 / 32 / 48 / 256
  assets/icon-animated.svg  same artwork + an 8 s CSS loop (badge lifts, stamps, settles; A dims);
                            identical to icon.svg at rest and under reduced motion

Why a tiny in-house rasteriser: the icon is flat fills only, and the repo has
no SVG renderer. Supported on purpose (not a general SVG engine):
  <rect x y width height rx fill fill-opacity>
  <path d="M L H V Q Z" (absolute only) fill fill-opacity fill-rule>
    fill-rule="evenodd": subpaths XOR (counters/holes); default: subpaths unioned
    (equals SVG nonzero for non-overlapping-hole shapes such as crossing strokes)
Painter order, supersampled anti-aliasing. If the SVG grows strokes, gradients,
arcs or relative commands, extend this first; it raises instead of guessing.
Pillow is a dev-only dependency of this script (not in requirements.txt).
"""

from __future__ import annotations

import re
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
SVG = ROOT / "assets" / "icon.svg"
OUT = ROOT / "icon.png"
OUT_512 = ROOT / "assets" / "icon-512.png"
OUT_ICO = ROOT / "assets" / "favicon.ico"
OUT_ANIM = ROOT / "assets" / "icon-animated.svg"

_ANIM_CSS = """<style>
  .badge-top { animation: stamp 8s infinite; }
  .source { animation: read 8s cubic-bezier(.22, 1, .36, 1) infinite; }
  @keyframes stamp {
    0%, 80% { transform: none; animation-timing-function: cubic-bezier(.22, 1, .36, 1); }
    83.25% { transform: translateY(-4px); animation-timing-function: cubic-bezier(.5, 0, .9, .6); }
    84.4% { transform: translateY(5px); animation-timing-function: cubic-bezier(.22, 1, .36, 1); }
    87.5%, 100% { transform: none; }
  }
  @keyframes read { 0%, 79% { opacity: 1; } 83%, 88% { opacity: .4; } 93%, 100% { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .badge-top, .source { animation: none; } }
</style>"""


def animated(svg: str) -> str:
    """Same paths as the master; badge + its glyph grouped so they can lift off the depth slab."""
    svg = svg.replace("</title>", "</title>\n  " + _ANIM_CSS, 1)
    svg = svg.replace('<path data-part="source-glyph"', '<path class="source" data-part="source-glyph"', 1)
    start = svg.index('<rect data-part="badge"')
    end = svg.index("/>", svg.index('data-part="target-glyph"')) + 2
    return svg[:start] + '<g class="badge-top">' + svg[start:end] + "</g>" + svg[end:]
SIZE = 256
SS = 8  # supersample per axis

_ATTR = re.compile(r'([a-zA-Z-]+)\s*=\s*"([^"]*)"')
_TOK = re.compile(r"[A-Za-z]|-?\d*\.?\d+(?:e-?\d+)?")


def _attrs(s: str) -> dict[str, str]:
    return dict(_ATTR.findall(s))


def _color(s: str) -> tuple[int, int, int]:
    s = s.lstrip("#")
    if len(s) == 3:
        s = "".join(c * 2 for c in s)
    return int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16)


def _subpaths(d: str) -> list[list[tuple[float, float]]]:
    toks = _TOK.findall(d)
    out: list[list[tuple[float, float]]] = []
    pts: list[tuple[float, float]] = []
    x = y = 0.0
    i, cmd = 0, ""

    def num() -> float:
        nonlocal i
        v = float(toks[i])
        i += 1
        return v

    while i < len(toks):
        if toks[i].isalpha():
            cmd = toks[i]
            i += 1
            if cmd == "Z":
                if pts:
                    out.append(pts)
                pts = []
                continue
        if cmd == "M":
            if pts:
                out.append(pts)
            x, y = num(), num()
            pts = [(x, y)]
            cmd = "L"
        elif cmd == "L":
            x, y = num(), num()
            pts.append((x, y))
        elif cmd == "H":
            x = num()
            pts.append((x, y))
        elif cmd == "V":
            y = num()
            pts.append((x, y))
        elif cmd == "Q":
            cx, cy, ex, ey = num(), num(), num(), num()
            for k in range(1, 17):
                t = k / 16
                pts.append(((1 - t) ** 2 * x + 2 * (1 - t) * t * cx + t * t * ex,
                            (1 - t) ** 2 * y + 2 * (1 - t) * t * cy + t * t * ey))
            x, y = ex, ey
        else:
            raise ValueError(f"unsupported path command {cmd!r} in icon SVG")
    if pts:
        out.append(pts)
    return out


def render(svg_text: str, size: int) -> Image.Image:
    vb = _attrs(re.search(r"<svg\b([^>]*)>", svg_text).group(1))["viewBox"].split()
    mx, my, vw, vh = (float(v) for v in vb)
    big = size * SS
    sx, sy = big / vw, big / vh
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    for tag, body in re.findall(r"<(rect|path)\b([^>]*)/?>", svg_text):
        a = _attrs(body)
        mask = Image.new("L", (big, big), 0)
        if tag == "rect":
            x0 = (float(a.get("x", 0)) - mx) * sx
            y0 = (float(a.get("y", 0)) - my) * sy
            x1 = x0 + float(a["width"]) * sx
            y1 = y0 + float(a["height"]) * sy
            ImageDraw.Draw(mask).rounded_rectangle(
                (x0, y0, x1 - 1, y1 - 1), radius=float(a.get("rx", 0)) * sx, fill=255)
        else:
            evenodd = a.get("fill-rule") == "evenodd"
            for sub in _subpaths(a["d"]):
                part = Image.new("L", (big, big), 0)
                ImageDraw.Draw(part).polygon([((px - mx) * sx, (py - my) * sy) for px, py in sub], fill=255)
                mask = ImageChops.difference(mask, part) if evenodd else ImageChops.lighter(mask, part)
        alpha = mask.reduce(SS)
        op = float(a.get("fill-opacity", 1))
        if op < 1:
            alpha = alpha.point(lambda v: round(v * op))
        layer = Image.new("RGBA", (size, size), (*_color(a.get("fill", "#000")), 0))
        layer.putalpha(alpha)
        canvas = Image.alpha_composite(canvas, layer)
    return canvas


def main() -> None:
    svg = SVG.read_text(encoding="utf-8")
    render(svg, SIZE).save(OUT, "PNG", optimize=True)
    render(svg, 512).save(OUT_512, "PNG", optimize=True)
    # Each ICO frame rendered at its own size (sharper than downscaling one master).
    sizes = [16, 32, 48, 256]
    frames = [render(svg, s) for s in sizes]
    frames[-1].save(OUT_ICO, format="ICO", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    OUT_ANIM.write_text(animated(svg), encoding="utf-8")
    for p in (OUT, OUT_512, OUT_ICO, OUT_ANIM):
        print(p.relative_to(ROOT), p.stat().st_size)


if __name__ == "__main__":
    main()
