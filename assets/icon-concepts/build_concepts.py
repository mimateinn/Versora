"""Concept SVGs + contact sheet for the Versora icon (sibling of Litora's "C soft-bound book").

Run: .venv/Scripts/python assets/icon-concepts/build_concepts.py
Litora's icon is read (read-only) from its repo for side-by-side comparison.
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from make_icon import render  # noqa: E402

LITORA = Path(r"H:\3 Apps\2026-08_litora\03 Src\build\icon.svg")

CREAM, TERRA, GROOVE, TAN, INK = "#ECE7D4", "#B95233", "#923D27", "#AA9E85", "#141415"
HEAD = '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="20 16 120 120">'
SLAB = f'<path fill="{INK}" d="M38.3 120H114.4Q128 120 128 107V120Q128 132 115.5 132H38.3Q32 132 32 126Q32 120 38.3 120Z"/>'
SPINE = (f'<path fill="{TERRA}" d="M46.6 28H50.8V125H32V42Q32 28 46.6 28Z"/>'
         f'<path fill="{GROOVE}" fill-opacity=".38" d="M48.2 33H49V121H48.2Z"/>')
BODY = f'<rect x="32" y="28" width="96" height="94" rx="14" fill="{CREAM}"/>'


def v_mono(fill: str, dx: float = 0, dy: float = 0) -> str:
    """Serif V in Litora's monogram weight: thick left stroke, hairline right, slab serifs."""
    pts = [(79, 76), (95, 76), (95, 78), (91, 78), (96.6, 97.5), (103.2, 78), (99, 78), (99, 76),
           (111, 76), (111, 78), (106, 78), (96.9, 107), (94.5, 107), (84, 78), (79, 78)]
    d = "M" + "L".join(f"{x + dx:g} {y + dy:g}" for x, y in pts) + "Z"
    return f'<path data-part="cover-monogram" fill="{fill}" d="{d}"/>'


CONCEPTS = {
    # A: Litora's book; the bookmark ribbon forks into two tails = one text, two languages.
    "a-forked-ribbon": [
        BODY, SPINE,
        f'<path fill="{TERRA}" d="M59.1 28H76.9V52L84 72H77.4L68 56.5L58.6 72H52.5L59.1 52Z"/>',
        v_mono(TAN), SLAB,
    ],
    # B: two leaves; a terracotta sheet sits behind the cream book, offset up-right = source/target.
    "b-two-leaves": [
        '<rect x="48" y="24" width="80" height="80" rx="12" fill="#B95233"/>',
        '<rect x="32" y="40" width="80" height="82" rx="12" fill="#ECE7D4"/>',
        '<path fill="#B95233" d="M44.6 40H49V125H32V52Q32 40 44.6 40Z"/>',
        v_mono(TAN, -12, 4),
        '<path fill="#141415" d="M38.3 120H100Q112 120 112 109V120Q112 132 101 132H38.3Q32 132 32 126Q32 120 38.3 120Z"/>',
    ],
    # C: bilingual facing edition; cover split cream | terracotta, the V straddles the seam in two tones.
    "c-split-cover": [
        BODY, SPINE,
        f'<path fill="{TERRA}" d="M95.9 28H114Q128 28 128 42V122H95.9Z"/>',
        f'<path fill="{TERRA}" d="M59.1 28H71.9V58L65.5 52L59.1 58Z"/>',
        v_mono(TAN),
        # right half of the V re-painted cream where it sits on terracotta (V clipped at x=95.9)
        f'<path fill="{CREAM}" d="M95.9 95.06L96.6 97.5L103.2 78H99V76H111V78H106L96.9 107H95.9Z"/>',
        SLAB,
    ],
}


def svg_of(parts: list[str], title: str) -> str:
    return HEAD + f"\n  <title>{title}</title>\n  " + "\n  ".join(parts) + "\n</svg>\n"


def font(px: int) -> ImageFont.ImageFont:
    for name in ("segoeui.ttf", "arial.ttf"):
        try:
            return ImageFont.truetype(name, px)
        except OSError:
            pass
    return ImageFont.load_default()


def sheet(rows: list[tuple[str, str]], out: Path) -> None:
    """Each row: 256 | 64 | 32 | 16 on light, 32 | 16 on dark, 16px blown up 8x (nearest)."""
    rh, W = 300, 1180
    img = Image.new("RGB", (W, 40 + rh * len(rows)), "#F4F2EC")
    dr = ImageDraw.Draw(img)
    dr.text((20, 10), "256 / 64 / 32 / 16 on light  |  32 / 16 on dark  |  16 px x8 (nearest)", fill="#555", font=font(18))
    for r, (label, svg) in enumerate(rows):
        y0 = 40 + r * rh
        dr.text((20, y0 + 6), label, fill="#222", font=font(20))
        x = 20
        for s in (256, 64, 32, 16):
            ic = render(svg, s)
            img.paste(ic, (x, y0 + 36 + (256 - s) // 2), ic)
            x += s + 24
        dark = Image.new("RGB", (130, 256), "#1B1F27")
        dx = 20
        for s in (32, 16):
            ic = render(svg, s)
            dark.paste(ic, (dx, (256 - s) // 2), ic)
            dx += s + 24
        img.paste(dark, (x, y0 + 36))
        x += 150
        big = render(svg, 16).resize((128, 128), Image.NEAREST)
        bg = Image.new("RGBA", (128, 128), "#F4F2EC")
        img.paste(Image.alpha_composite(bg, big).convert("RGB"), (x, y0 + 36 + 64))
        x += 148
        bigd = Image.alpha_composite(Image.new("RGBA", (128, 128), "#1B1F27"), big)
        img.paste(bigd.convert("RGB"), (x, y0 + 36 + 64))
    img.save(out)


def main() -> None:
    rows = [("Litora (reference, C soft-bound book)", LITORA.read_text(encoding="utf-8"))]
    for name, parts in CONCEPTS.items():
        svg = svg_of(parts, f"Versora concept {name}")
        (HERE / f"{name}.svg").write_text(svg, encoding="utf-8")
        render(svg, 512).save(HERE / f"{name}.png")
        rows.append((f"Versora {name}", svg))
    sheet(rows, HERE / "contact-sheet.png")
    print("ok", HERE / "contact-sheet.png")


if __name__ == "__main__":
    main()
