"""Inline SVG glyphs for v2 chrome, drawn in Litora's UI icon grammar. English identifiers only. No emoji.

Grammar (same as Litora's atrium-icons): viewBox 0 0 24 24, stroke 1.7, round cap + join,
fill none, currentColor. Optical frame: circles r 7.6 (4.4..19.6), squares and lines
5.4..18.6, containers corner radius 1.2-1.6, every glyph centred on (12, 12).
Accent parts (check tick, spinner arc, progress dot) paint with ``var(--vi-accent, currentColor)``;
the UI sets ``--vi-accent`` (Versora ink blue) in its theme, no hex lives here.

Animation contract -- inject ``ICON_CSS`` once (e.g. in theme.py). Every icon root carries
``class="vi vi-<name>"``. Motion is transform / opacity / stroke-dashoffset only, ease
``cubic-bezier(.22,1,.36,1)``, 140-380 ms, and ``prefers-reduced-motion: reduce`` turns it all off.

Hover (any ancestor that is ``button``, ``a``, ``label``, ``[role=button]`` or ``.vi-host``, on
``:hover`` / ``:focus-visible`` / ``.is-hover``):
  SUN rays turn 30deg; MOON tilts; DOWNLOAD arrow drops; UPLOAD arrow lifts; RETRY turns once;
  SETTINGS knobs slide along their rails; SWAP arrows part; FOLDER_OPEN flap opens.
Press: every icon dips to .86 while its host is ``:active``. For a full click motion, add ``.is-press``
  to the host (or svg) once per click from JS and remove it on ``animationend``; this is Litora's press
  set verbatim (cubic-bezier(.3,.7,.3,1)): pop by default, drop (download/upload/folder), lean
  (settings/book/key), spin 560 ms (retry/swap/globe), open (folder_open/file/zip).
  ``.vi-drag`` on an ancestor (drop zone during drag-over) lifts UPLOAD and keeps it lifted.
One-shot on appear (put the class on the svg or any ancestor; replays whenever the node is
re-created, so add it only where a fresh appearance is meaningful):
  ``.vi-anim-check``  CHECK ring settles and the tick draws itself
  ``.vi-anim-alert``  ALERT's "!" springs up once
  ``.vi-anim-in``     any icon fades and scales in
On action (add ``.vi-play`` for one frame-pair, e.g. after a click; remove it to re-arm):
  SWAP flips 180deg; SUN rays spin in; MOON swings in.
Always running while present: SPINNER rotates; DOT pulses a ring (progress / busy dot). Under reduced
motion these two keep a slow opacity fade so a busy state is still visible.
"""

from __future__ import annotations

from urllib.parse import quote

_ATTRS = (
    'viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" '
    'stroke-linecap="round" stroke-linejoin="round"'
)
_SVG_OPEN = f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" {_ATTRS} aria-hidden="true">'


def _svg(inner: str, name: str = "") -> str:
    cls = f"vi vi-{name}" if name else "vi"
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" {_ATTRS} '
        f'class="{cls}" aria-hidden="true">{inner}</svg>'
    )


def _mask_uri(inner: str) -> str:
    raw = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" '
        'stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">'
        f"{inner}</svg>"
    )
    return "data:image/svg+xml," + quote(raw, safe="")


def _p(d: str, cls: str = "", extra: str = "") -> str:
    c = f' class="{cls}"' if cls else ""
    return f'<path{c} d="{d}"{extra}/>'


_RING = "M12 4.4a7.6 7.6 0 1 0 0 15.2 7.6 7.6 0 0 0 0-15.2"

# ---- redrawn originals -------------------------------------------------------------------
_MONITOR = _p(
    "M6 5.2h12a1.5 1.5 0 0 1 1.5 1.5v7.4a1.5 1.5 0 0 1-1.5 1.5H6a1.5 1.5 0 0 1-1.5-1.5V6.7A1.5 1.5 0 0 1 6 5.2z"
    "M9.2 18.8h5.6M12 15.6v3.2"
)
_GLOBE = _p(
    f"{_RING}M4.8 9.6h14.4M4.8 14.4h14.4M12 4.4c1.6 2.1 2.6 4.7 2.6 7.6s-1 5.5-2.6 7.6"
    "c-1.6-2.1-2.6-4.7-2.6-7.6s1-5.5 2.6-7.6"
)
_KEY = _p("M8.8 7.4a4.6 4.6 0 1 0 0 9.2 4.6 4.6 0 0 0 0-9.2M13.4 12h6.2M16.4 12v3.6M19.6 12v2.8")
_BOOK = _p(
    "M12 6.8C10.1 5.5 7.9 4.9 5.4 5a.8.8 0 0 0-.8.8v10.8a.8.8 0 0 0 .8.8c2.5-.1 4.7.5 6.6 1.8"
    " 1.9-1.3 4.1-1.9 6.6-1.8a.8.8 0 0 0 .8-.8V5.8a.8.8 0 0 0-.8-.8c-2.5-.1-4.7.5-6.6 1.8zM12 6.8v12.4"
)
_FILE = _p(
    "M7.4 4.4h6.2l4.4 4.4v9.6a1.2 1.2 0 0 1-1.2 1.2H7.4a1.2 1.2 0 0 1-1.2-1.2V5.6a1.2 1.2 0 0 1 1.2-1.2z"
    "M13.4 4.4v3.8a.8.8 0 0 0 .8.8H18"
)
_FOLDER = _p(
    "M4.4 7.4a1.5 1.5 0 0 1 1.5-1.5h3.2l1.8 2h7.2a1.5 1.5 0 0 1 1.5 1.5v7.3a1.5 1.5 0 0 1-1.5 1.5H5.9"
    "a1.5 1.5 0 0 1-1.5-1.5z"
)
_ZIP = _p(
    "M6.6 5.4h10.8a1.2 1.2 0 0 1 1.2 1.2v1.2a1.2 1.2 0 0 1-1.2 1.2H6.6a1.2 1.2 0 0 1-1.2-1.2V6.6"
    "a1.2 1.2 0 0 1 1.2-1.2zM6.4 9v8.4a1.2 1.2 0 0 0 1.2 1.2h8.8a1.2 1.2 0 0 0 1.2-1.2V9M10.4 12.2h3.2"
)
_BUBBLE = _p(
    "M6.2 4.9h11.6a1.6 1.6 0 0 1 1.6 1.6v7.8a1.6 1.6 0 0 1-1.6 1.6h-6L7.4 19.3v-3.4H6.2a1.6 1.6 0 0 1-1.6-1.6"
    "V6.5a1.6 1.6 0 0 1 1.6-1.6z"
)
_GAME = _p(
    "M9.4 6.8h5.2a5.2 5.2 0 0 1 0 10.4H9.4a5.2 5.2 0 0 1 0-10.4zM8.6 9.8v4.4M6.4 12h4.4M15.2 10.8h.01M17.2 13.2h.01"
)
_SUN = _p("M12 8.8a3.2 3.2 0 1 0 0 6.4 3.2 3.2 0 0 0 0-6.4") + _p(
    "M12 4.2v1.6M12 18.2v1.6M4.2 12h1.6M18.2 12h1.6M6.5 6.5l1.1 1.1M16.4 16.4l1.1 1.1"
    "M6.5 17.5l1.1-1.1M16.4 7.6l1.1-1.1",
    "vi-rays",
)
_MOON = _p("M18.6 14.6A7.2 7.2 0 1 1 9.4 5.4a5.6 5.6 0 0 0 9.2 9.2z", "vi-moon-body")
_CHECK = _p(_RING, "vi-ring") + _p("M8.6 12.4l2.3 2.3 4.6-4.9", "vi-tick vi-accent", ' pathLength="1"')
_DASH = _p(_RING) + _p("M8.6 12h6.8")

# ---- new for the redesign ----------------------------------------------------------------
_UPLOAD = _p("M12 15V5.4M8.2 9.2 12 5.4l3.8 3.8", "vi-lift") + _p("M5.4 18.6h13.2")
_DOWNLOAD = _p("M12 5.4V15M8.2 11.2 12 15l3.8-3.8", "vi-drop") + _p("M5.4 18.6h13.2")
_FOLDER_OPEN = _p("M4.4 18.2V7.4a1.5 1.5 0 0 1 1.5-1.5h3.2l1.8 2h6.2a1.5 1.5 0 0 1 1.5 1.5v1.5") + _p(
    "M4.4 18.2l2.1-6.4a1.3 1.3 0 0 1 1.2-.9h11.4a.9.9 0 0 1 .86 1.2l-1.9 5.2a1.3 1.3 0 0 1-1.2.9z",
    "vi-flap",
)
_SWAP = (
    '<g class="vi-swap-g">'
    + _p("M5.4 8.6h13.2M14.8 4.8l3.8 3.8-3.8 3.8", "vi-swap-a")
    + _p("M18.6 15.4H5.4M9.2 11.6l-3.8 3.8 3.8 3.8", "vi-swap-b")
    + "</g>"
)
_RETRY = _p("M19.2 12a7.2 7.2 0 1 1-2.1-5.1M19.2 4.4v3h-3", "vi-retry-arrow")
_STOP = _p(
    "M7.1 5.6h9.8a1.5 1.5 0 0 1 1.5 1.5v9.8a1.5 1.5 0 0 1-1.5 1.5H7.1a1.5 1.5 0 0 1-1.5-1.5V7.1a1.5 1.5 0 0 1 1.5-1.5z"
)
_ALERT = _p(_RING, "vi-ring") + _p("M12 8v4.6M12 15.8h.01", "vi-mark")
# sliders: each rail is one line whose gap is a dash pattern, so the gap travels with its knob
_SETTINGS = (
    _p("M5.4 8h13.2", "vi-rail-a", ' stroke-dasharray="5.6 5.6 30"')
    + _p("M5.4 16h13.2", "vi-rail-b", ' stroke-dasharray="2 5.6 30"')
    + _p("M13.8 5.8v4.4", "vi-knob-a")
    + _p("M10.2 13.8v4.4", "vi-knob-b")
)
_PLUS = _p("M12 5.4v13.2M5.4 12h13.2", "vi-press")
_CLOSE = _p("M6.8 6.8l10.4 10.4M17.2 6.8 6.8 17.2", "vi-press")
_SPINNER = _p(_RING, "vi-track") + _p("M12 4.4a7.6 7.6 0 0 1 7.6 7.6", "vi-arc vi-accent")
_DOT = (
    '<circle class="vi-pulse-ring vi-accent" cx="12" cy="12" r="7.6"/>'
    '<circle class="vi-dot-core vi-accent-fill" cx="12" cy="12" r="4.6" fill="currentColor" stroke="none"/>'
)

# ---- exports -----------------------------------------------------------------------------
SUN = _svg(_SUN, "sun")
MOON = _svg(_MOON, "moon")
GLOBE = _svg(_GLOBE, "globe")
FILE = _svg(_FILE, "file")
CHECK = _svg(_CHECK, "check")
DASH = _svg(_DASH, "dash")
MONITOR = _svg(_MONITOR, "monitor")
KEY = _svg(_KEY, "key")
BOOK = _svg(_BOOK, "book")
FOLDER = _svg(_FOLDER, "folder")
ZIP = _svg(_ZIP, "zip")
BUBBLE = _svg(_BUBBLE, "bubble")
GAME = _svg(_GAME, "game")
UPLOAD = _svg(_UPLOAD, "upload")
DOWNLOAD = _svg(_DOWNLOAD, "download")
FOLDER_OPEN = _svg(_FOLDER_OPEN, "folder-open")
SWAP = _svg(_SWAP, "swap")
RETRY = _svg(_RETRY, "retry")
STOP = _svg(_STOP, "stop")
ALERT = _svg(_ALERT, "alert")
SETTINGS = _svg(_SETTINGS, "settings")
PLUS = _svg(_PLUS, "plus")
CLOSE = _svg(_CLOSE, "close")
SPINNER = _svg(_SPINNER, "spinner")
DOT = _svg(_DOT, "dot")

ICONS = {
    "sun": SUN, "moon": MOON, "globe": GLOBE, "file": FILE, "check": CHECK, "dash": DASH,
    "monitor": MONITOR, "key": KEY, "book": BOOK, "folder": FOLDER, "zip": ZIP, "bubble": BUBBLE,
    "game": GAME, "upload": UPLOAD, "download": DOWNLOAD, "folder_open": FOLDER_OPEN, "swap": SWAP,
    "retry": RETRY, "stop": STOP, "alert": ALERT, "settings": SETTINGS, "plus": PLUS,
    "close": CLOSE, "spinner": SPINNER, "dot": DOT,
}

MASKS = {
    "monitor": _mask_uri(_MONITOR),
    "globe": _mask_uri(_GLOBE),
    "key": _mask_uri(_KEY),
    "book": _mask_uri(_BOOK),
    "file": _mask_uri(_FILE),
    "folder": _mask_uri(_FOLDER),
    "zip": _mask_uri(_ZIP),
    "bubble": _mask_uri(_BUBBLE),
    "game": _mask_uri(_GAME),
    "folder_open": _mask_uri(_FOLDER_OPEN),
    "upload": _mask_uri(_UPLOAD),
    "download": _mask_uri(_DOWNLOAD),
    "swap": _mask_uri(_SWAP),
    "retry": _mask_uri(_RETRY),
    "settings": _mask_uri(_SETTINGS),
    "plus": _mask_uri(_PLUS),
    "close": _mask_uri(_CLOSE),
}

_HOST = ':is(.vi-host, button, a, label, [role="button"]):is(:hover, :focus-visible, .is-hover)'

ICON_CSS = f"""
.vi {{ --vi-ease: cubic-bezier(.22, 1, .36, 1); overflow: visible; transform-origin: 50% 50%; }}
.vi * {{ transform-box: view-box; transform-origin: 12px 12px; }}
.vi .vi-accent {{ stroke: var(--vi-accent, currentColor); }}
.vi .vi-accent-fill {{ fill: var(--vi-accent, currentColor); }}
.vi .vi-track {{ opacity: .22; }}

.vi-rays, .vi-moon-body, .vi-lift, .vi-drop, .vi-swap-a, .vi-swap-b, .vi-flap, .vi-knob-a, .vi-knob-b {{
  transition: transform 300ms var(--vi-ease);
}}
.vi {{ transition: transform 140ms var(--vi-ease); }}
.vi-rail-a, .vi-rail-b {{ transition: stroke-dashoffset 300ms var(--vi-ease); }}
.vi-knob-b, .vi-rail-b {{ transition-delay: 50ms; }}
{_HOST} .vi-rays {{ transform: rotate(30deg); }}
{_HOST} .vi-moon-body {{ transform: rotate(-18deg); }}
{_HOST} .vi-drop {{ transform: translateY(2.4px); }}
{_HOST} .vi-lift, .vi-drag .vi-lift {{ transform: translateY(-2.4px); }}
{_HOST} .vi-swap-a {{ transform: translateX(2.4px); }}
{_HOST} .vi-swap-b {{ transform: translateX(-2.4px); }}
{_HOST} .vi-flap {{ transform: translateY(-1.6px) skewX(-10deg); }}
{_HOST} .vi-knob-a {{ transform: translateX(1.4px); }}
{_HOST} .vi-rail-a {{ stroke-dashoffset: -1.4; }}
{_HOST} .vi-knob-b {{ transform: translateX(-1.4px); }}
{_HOST} .vi-rail-b {{ stroke-dashoffset: 1.4; }}
{_HOST} .vi-retry-arrow {{ animation: vi-turn 380ms var(--vi-ease) 1; }}
:is(.vi-host, button, a, label, [role="button"]):active:not(.is-press) > .vi,
:is(.vi-host, button, a, label, [role="button"]):active:not(.is-press) .sfts-ico > .vi {{ transform: scale(.86); }}
/* Litora's press set, verbatim: each object moves its own way */
.is-press .vi, .vi.is-press {{ animation: vi-pop 440ms cubic-bezier(.3, .7, .3, 1) 1; }}
:is(.is-press .vi, .vi.is-press):is(.vi-download, .vi-upload, .vi-folder) {{ animation-name: vi-drop-press; }}
:is(.is-press .vi, .vi.is-press):is(.vi-settings, .vi-book, .vi-key) {{ animation-name: vi-lean; transform-origin: 50% 85%; }}
:is(.is-press .vi, .vi.is-press):is(.vi-retry, .vi-swap, .vi-globe) {{ animation-name: vi-spin-press; animation-duration: 560ms; }}
:is(.is-press .vi, .vi.is-press):is(.vi-folder-open, .vi-file, .vi-zip) {{ animation-name: vi-open; }}

.vi-anim-check .vi-ring {{ animation: vi-settle 240ms var(--vi-ease) both; }}
.vi-anim-check .vi-tick {{ animation: vi-draw 300ms var(--vi-ease) 60ms both; }}
.vi-anim-alert .vi-ring {{ animation: vi-settle 240ms var(--vi-ease) both; }}
.vi-anim-alert .vi-mark {{ animation: vi-alert 320ms var(--vi-ease) 1; transform-origin: 12px 16px; }}
.vi-anim-in.vi, .vi-anim-in .vi {{ animation: vi-in 280ms var(--vi-ease) both; }}
.vi-play .vi-swap-g {{ animation: vi-flip 380ms var(--vi-ease) 1; }}
.vi-play .vi-rays {{ animation: vi-rays-in 380ms var(--vi-ease) 1; }}
.vi-play .vi-moon-body {{ animation: vi-moon-in 380ms var(--vi-ease) 1; }}
.vi-spinner .vi-arc {{ animation: vi-turn 900ms linear infinite; }}
.vi-dot .vi-pulse-ring {{ animation: vi-ripple 1600ms var(--vi-ease) infinite; }}

@keyframes vi-turn {{ from {{ transform: rotate(0); }} to {{ transform: rotate(360deg); }} }}
@keyframes vi-draw {{
  from {{ stroke-dasharray: 1 1; stroke-dashoffset: 1; }}
  to {{ stroke-dasharray: 1 1; stroke-dashoffset: 0; }}
}}
@keyframes vi-settle {{ from {{ opacity: 0; transform: scale(.82); }} to {{ opacity: 1; transform: none; }} }}
@keyframes vi-alert {{ from {{ transform: scaleY(.6); opacity: 0; }} to {{ transform: none; opacity: 1; }} }}
/* Litora's nav-pop, verbatim (fired once per click via .is-press, as Litora's popNavIcon does) */
@keyframes vi-pop {{ 0% {{ transform: scale(1); }} 30% {{ transform: scale(.78); }} 62% {{ transform: scale(1.14); }} 100% {{ transform: scale(1); }} }}
@keyframes vi-drop-press {{ 0% {{ transform: translateY(0); }} 30% {{ transform: translateY(-4px) scale(.94); }} 60% {{ transform: translateY(2px) scale(1.06); }} 100% {{ transform: none; }} }}
@keyframes vi-lean {{ 0% {{ transform: rotate(0); }} 30% {{ transform: rotate(-14deg) scale(.92); }} 64% {{ transform: rotate(6deg) scale(1.06); }} 100% {{ transform: none; }} }}
@keyframes vi-spin-press {{ 0% {{ transform: rotate(0) scale(1); }} 40% {{ transform: rotate(200deg) scale(.9); }} 100% {{ transform: rotate(360deg) scale(1); }} }}
@keyframes vi-open {{ 0% {{ transform: scaleX(1); }} 35% {{ transform: scaleX(.72); }} 70% {{ transform: scaleX(1.12); }} 100% {{ transform: none; }} }}
@keyframes vi-in {{ from {{ opacity: 0; transform: scale(.78); }} to {{ opacity: 1; transform: none; }} }}
@keyframes vi-flip {{ from {{ transform: rotate(0); }} to {{ transform: rotate(180deg); }} }}
@keyframes vi-rays-in {{ from {{ opacity: 0; transform: rotate(-60deg) scale(.7); }} to {{ opacity: 1; transform: none; }} }}
@keyframes vi-moon-in {{ from {{ opacity: 0; transform: rotate(40deg) scale(.85); }} to {{ opacity: 1; transform: none; }} }}
@keyframes vi-ripple {{
  0% {{ opacity: .5; transform: scale(.55); }} 70%, 100% {{ opacity: 0; transform: scale(1); }}
}}

@media (prefers-reduced-motion: reduce) {{
  .vi, .vi * {{ animation: none !important; transition: none !important; transform: none !important; }}
  .vi .vi-rail-a, .vi .vi-rail-b {{ stroke-dashoffset: 0 !important; }}
  .vi .vi-pulse-ring {{ opacity: 0; }}
  /* busy icons keep a still cue: the spinner's arc and the dot's core stay drawn, nothing loops */
  .vi.vi-spinner .vi-arc, .vi.vi-dot .vi-dot-core {{ opacity: 1 !important; }}
}}
"""


def wrap(svg: str) -> str:
    return f'<span class="sfts-ico" aria-hidden="true">{svg}</span>'
