"""Versora chrome CSS — Litora family: warm paper, ink, one terracotta accent,
hairlines not frames, quiet motion. Local fonts only. No blur, no coloured shadows.

Grid: 4px. Control heights are 42 (fields, main buttons) or 34 (small: segments,
rail, cancel, remove). scripts/check_alignment.py measures both on the live page.

Streamlit does not nest widgets inside markdown wrappers, so chrome rules target
widget keys (``.st-key-*`` / ``[class*="st-key-"]``) and ``data-testid``s of
Streamlit 1.65 (react-aria widgets, not baseweb). Renaming a widget key in
app.py breaks its rule here.
"""

from __future__ import annotations

from urllib.parse import quote

from src import icons as _icons
from src.icons import ICON_CSS, MASKS as _ICON_MASKS

# The one place the brand colour (Versora ink blue) lives; everything else reads --sfts-accent*.
# accent = fills (white text on it 4.6:1). accent-strong = text, links, hover fills (6.4:1 on paper).
# The fill colour as text on paper is only 4.3:1, so text never uses it. Dark: lighter steps, navy ink on fills (7.3:1).
ACCENTS = {
    "light": {"accent": "#4677B5", "accent-strong": "#325D8E", "accent-soft": "rgba(70,119,181,.09)", "accent-ink": "#FFFFFF"},
    "dark": {"accent": "#7FA6D8", "accent-strong": "#A3C0E6", "accent-soft": "rgba(127,166,216,.13)", "accent-ink": "#0E1320"},
}
ACCENT = ACCENTS["light"]["accent"]


# Button glyphs are CSS masks (Streamlit buttons cannot hold inline SVG). The icon set
# exports masks for most names; stop/sun/moon are built from its own paths here.
MASKS = {
    **_ICON_MASKS,
    "stop": _icons._mask_uri(_icons._STOP),
    "sun": _icons._mask_uri(_icons._SUN),
    "moon": _icons._mask_uri(_icons._MOON),
}

_TOKENS = {
    "light": {
        "bg": "#FAF8F5", "card": "#FFFFFF", "sunken": "#F5F0E6", "raised": "#FEFDFB",
        "text": "#1A1A1A", "muted": "#6B6B6B", "faint": "#8C8880",
        "line": "rgba(15,15,15,.08)", "line-strong": "rgba(15,15,15,.15)",
        **ACCENTS["light"],
        "ok": "#2B7A52", "err": "#A81830", "wait": "#C9C4BA",
        "ok-soft": "rgba(43,122,82,.08)", "err-soft": "rgba(168,24,48,.06)",
        "wm-ink": "rgba(26,26,26,.06)",
        "shadow": "0 8px 22px rgba(15,15,15,.08), 0 2px 6px rgba(15,15,15,.05)",
        "shadow-sm": "0 2px 6px rgba(15,15,15,.05), 0 1px 2px rgba(15,15,15,.04)",
        "toast-bg": "#1A1A1A", "toast-ink": "#FAF8F5",
        "art-fill": "rgba(26,26,26,.04)", "art-line": "rgba(26,26,26,.40)",
    },
    "dark": {
        "bg": "#0E1320", "card": "#161D2E", "sunken": "#1F2940", "raised": "#1A2234",
        "text": "#F2F5FB", "muted": "#8F9DB8", "faint": "#6F7C96",
        "line": "rgba(242,245,251,.08)", "line-strong": "rgba(242,245,251,.16)",
        **ACCENTS["dark"],
        "ok": "#5CB287", "err": "#E07A8C", "wait": "#4A5470",
        "ok-soft": "rgba(92,178,135,.12)", "err-soft": "rgba(224,122,140,.10)",
        "wm-ink": "rgba(242,245,251,.03)",
        "shadow": "0 8px 22px rgba(0,0,0,.30), 0 2px 6px rgba(0,0,0,.22)",
        "shadow-sm": "0 2px 6px rgba(0,0,0,.22), 0 1px 2px rgba(0,0,0,.16)",
        "toast-bg": "#F2F5FB", "toast-ink": "#0E1320",
        "art-fill": "rgba(242,245,251,.04)", "art-line": "rgba(242,245,251,.45)",
    },
}

_FONTS = """
  --f-ui: "Inter", "Segoe UI Variable Text", "Segoe UI", "Noto Sans TC", "Microsoft JhengHei UI", "PingFang TC", system-ui, sans-serif;
  --f-disp: "Poppins", "Space Grotesk", "Segoe UI Variable Display", "Segoe UI", "Noto Sans TC", "Microsoft JhengHei UI", sans-serif;
  --f-mono: "IBM Plex Mono", "Cascadia Mono", Consolas, ui-monospace, "Microsoft JhengHei UI", monospace;
  --e-out: cubic-bezier(.22,1,.36,1);
  --d-press: 80ms; --d-fast: 140ms; --d-norm: 220ms;
  --h-ctrl: 42px; --h-small: 34px;
  --vi-accent: var(--sfts-accent);
"""


def _art_uri(fill: str, line: str, accent: str) -> str:
    """Two pages, one turning into the other: the drop zone drawing."""
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 168 96" fill="none" '
        f'stroke="{line}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">'
        '<path d="M14 86h140" stroke-opacity=".5"/>'
        f'<rect x="40" y="14" width="50" height="66" rx="4" fill="{fill}"/>'
        '<path d="M50 30h30M50 40h30M50 50h22M50 60h26" stroke-opacity=".6"/>'
        f'<rect x="80" y="22" width="50" height="62" rx="4" fill="{fill}" stroke="{accent}" stroke-dasharray="4 4"/>'
        f'<path d="M90 38h28M90 48h24M90 58h28" stroke="{accent}" stroke-opacity=".8"/>'
        f'<path d="M64 6c10-4 22-2 30 6" stroke="{accent}"/><path d="M90 6l4 6-7 1" stroke="{accent}"/>'
        "</svg>"
    )
    return "data:image/svg+xml," + quote(svg, safe="")


def _vars(theme: str) -> str:
    tok = _TOKENS[theme]
    lines = "".join(f"  --sfts-{k}: {v};\n" for k, v in tok.items())
    art = _art_uri(tok["art-fill"], tok["art-line"], tok["accent"])
    return (
        f":root {{\n{lines}{_FONTS}  --sfts-art: url(\"{art}\");\n"
        f"  --sfts-upload: url(\"{MASKS['upload']}\");\n  color-scheme: {theme};\n}}\n"
    )


def _mask_icon(selector: str, name: str, margin: str = "0 8px 0 0", size: int = 16) -> str:
    uri = MASKS[name]
    return f"""
{selector}::before {{
  content: ""; display: inline-block; width: {size}px; height: {size}px; flex: 0 0 {size}px;
  margin: {margin}; background-color: currentColor;
  -webkit-mask: url("{uri}") center / contain no-repeat; mask: url("{uri}") center / contain no-repeat;
}}"""


_SHARED = """
html, body, .stApp, .stMarkdown, button, input, textarea, label, [data-testid="stWidgetLabel"] {
  font-family: var(--f-ui) !important;
}
.stApp { font-size: 14px; }
.stApp [data-testid="stMarkdownContainer"] p, .stApp button p, .stApp input, .stApp textarea { font-size: 14px; }
/* Streamlit chrome off */
[data-testid="stSidebar"], [data-testid="stSidebarCollapsedControl"], [data-testid="stHeader"],
[data-testid="stToolbar"], [data-testid="stDecoration"], [data-testid="stStatusWidget"],
#MainMenu, footer, .stDeployButton, .stAppDeployButton, [data-testid="stAppDeployButton"],
div[data-testid="stToolbarActions"], [data-testid="stHeaderActionElements"] {
  display: none !important;
}
html, body, .stApp { background: var(--sfts-bg) !important; color: var(--sfts-text); }
.stApp { isolation: isolate; }
[data-testid="stAppViewContainer"], [data-testid="stMain"], .main, section.main,
[data-testid="stBottom"] > div { background: transparent !important; }
.block-container, [data-testid="stMainBlockContainer"] {
  padding: 16px 24px 40px !important; max-width: 912px !important;
}
[data-testid="stMarkdownContainer"] p { color: inherit; margin-bottom: 0; }
/* Streamlit pulls markdown up by -16px; our blocks size themselves, so their box must be their content */
[data-testid="stMarkdownContainer"] { margin-bottom: 0 !important; }
[data-testid="stMainBlockContainer"] > [data-testid="stVerticalBlock"],
[data-testid="stMainBlockContainer"] > div > [data-testid="stVerticalBlock"] { gap: 24px !important; }  /* one section gap */
a { color: var(--sfts-accent-strong); }
:focus-visible { outline: 2px solid var(--sfts-accent) !important; outline-offset: 3px !important; }
/* Zero-height helper blocks (style, watermark) must not add gaps */
[data-testid="stElementContainer"]:has(> div > div > style),
[data-testid="stElementContainer"]:has(.sfts-wm) {
  position: absolute !important; height: 0 !important; margin: 0 !important; overflow: visible !important;
}

/* ── Watermark: two composited layers roll opposite ways; only transform animates ── */
.sfts-wm { position: fixed; inset: 0; z-index: -1; pointer-events: none; overflow: hidden; }
.sfts-wm-field {
  position: absolute; left: 50%; top: 50%; width: 160vmax; height: 160vmax;
  transform: translate(-50%, -50%) rotate(-26.6deg);
}
.sfts-wm-layer {
  position: absolute; left: -40vmax; top: 0; width: max-content; will-change: transform;
  font: 600 8.5px/30px var(--f-ui); letter-spacing: .3em; text-transform: uppercase;
  color: var(--sfts-wm-ink); white-space: nowrap;
}
.sfts-wm-layer div { height: 60px; }
.sfts-wm-a { animation: sfts-roll-l 480s linear infinite; }
.sfts-wm-b { top: 30px; animation: sfts-roll-r 480s linear infinite; }
@keyframes sfts-roll-l { from { transform: translateX(0); } to { transform: translateX(-50%); } }
@keyframes sfts-roll-r { from { transform: translateX(-50%); } to { transform: translateX(0); } }

/* ── Top bar: one quiet wordmark ── */
.sfts-brand { display: flex; align-items: center; gap: 8px; height: 42px; }
.sfts-brand img { width: 24px; height: 24px; border-radius: 6px; display: block; }
.sfts-brand b { font-size: 15px; font-weight: 600; letter-spacing: -.005em; color: var(--sfts-text); }

/* ── Cards (st.container(border=True, key="card_*")) ── */
div[class*="st-key-card_"] {
  background: var(--sfts-card) !important; border: 1px solid var(--sfts-line) !important;
  border-radius: 12px !important; box-shadow: var(--sfts-shadow-sm) !important;
  padding: 20px !important; gap: 16px !important;
  animation: sfts-enter 320ms var(--e-out) both;
}
div[class*="st-key-card_result"] { animation-delay: 24ms; }
div[class*="st-key-card_run"] { animation-delay: 24ms; }
@keyframes sfts-enter { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: none; } }
.sfts-rule { height: 1px; background: var(--sfts-line); border: 0; margin: 0 -20px !important; }
.sfts-divider { height: 1px; background: var(--sfts-line); border: 0; margin: 4px 0; }

/* ── Labels: mono eyebrows, baseline-aligned with the field below ── */
[data-testid="stWidgetLabel"] { min-height: 20px !important; margin: 0 0 4px !important; padding: 0 !important; }
[data-testid="stWidgetLabel"] p, .sfts-flabel {
  font-family: var(--f-mono) !important; font-size: 11px !important; line-height: 20px !important;
  letter-spacing: .14em !important; text-transform: uppercase !important;
  color: var(--sfts-muted) !important; font-weight: 500 !important;
}
.sfts-flabel { height: 20px; margin: 0; }
/* quick bar columns: label + 4 + control, same as a widget's own label */
[class*="st-key-quickbar"] [data-testid="stColumn"] > [data-testid="stVerticalBlock"] { gap: 4px !important; }
/* settings rail: items 4 apart */
[data-testid="stColumn"]:has([class*="st-key-pane_"]) > [data-testid="stVerticalBlock"] { gap: 4px !important; }
.sfts-eyebrow, .sfts-panel-title {
  font-family: var(--f-mono); font-size: 11px; line-height: 20px; font-weight: 500; letter-spacing: .14em;
  text-transform: uppercase; color: var(--sfts-muted); margin: 0;
}

/* ── Buttons: 42 tall, 10px corners, no pills ── */
[data-testid^="stBaseButton"] {
  min-height: var(--h-ctrl) !important; height: var(--h-ctrl) !important; padding: 0 16px !important;
  border-radius: 10px !important; font-weight: 500 !important; box-shadow: none !important;
  display: inline-flex !important; align-items: center !important; justify-content: center !important;
  transition: background-color var(--d-fast) var(--e-out), color var(--d-fast) var(--e-out),
              border-color var(--d-fast) var(--e-out), box-shadow var(--d-fast) var(--e-out),
              transform var(--d-fast) var(--e-out) !important;
}
[data-testid^="stBaseButton"] > div { flex: 0 1 auto !important; }  /* icon + label centre together */
[data-testid^="stBaseButton"] p { font-weight: inherit !important; line-height: 20px !important; }
[data-testid^="stBaseButton"]:active:not(:disabled) {
  transform: translateY(1px) scale(.985) !important; transition-duration: var(--d-press) !important;
}
[data-testid="stBaseButton-secondary"] {
  background: var(--sfts-card) !important; color: var(--sfts-text) !important;
  border: 1px solid var(--sfts-line-strong) !important;
}
[data-testid="stBaseButton-secondary"]:hover:not(:disabled) {
  border-color: var(--sfts-accent) !important; color: var(--sfts-accent-strong) !important;
}
[data-testid="stBaseButton-primary"] {
  background: var(--sfts-accent) !important; color: var(--sfts-accent-ink) !important;
  border: 1px solid var(--sfts-accent) !important;
}
[data-testid="stBaseButton-primary"]:hover:not(:disabled) {
  background: var(--sfts-accent-strong) !important; border-color: var(--sfts-accent-strong) !important;
}
[data-testid^="stBaseButton"]:disabled { opacity: .4 !important; cursor: not-allowed !important; }

/* ── Fields: select, text, textarea — 42 tall ── */
[data-testid="stSelectbox"] [role="group"], [data-testid="stTextInputRootElement"] {
  height: var(--h-ctrl) !important; min-height: var(--h-ctrl) !important; box-sizing: border-box !important;
  background: var(--sfts-raised) !important; border: 1px solid var(--sfts-line-strong) !important;
  border-radius: 10px !important; color: var(--sfts-text) !important;
  transition: border-color var(--d-fast) var(--e-out), box-shadow var(--d-fast) var(--e-out);
}
[data-testid="stTextArea"] textarea {
  background: var(--sfts-sunken) !important; border: 1px solid var(--sfts-line) !important; border-radius: 10px !important;
  font-family: var(--f-mono) !important; font-size: 12px !important; line-height: 20px !important;
  color: var(--sfts-text) !important; -webkit-text-fill-color: var(--sfts-text) !important; opacity: 1 !important;
  cursor: text !important; padding: 12px !important;
}
[data-testid="stTextArea"] > div, [data-testid="stTextArea"] [data-baseweb="textarea"] { border: 0 !important; background: transparent !important; }
[data-testid="stSelectbox"] [role="group"]:hover, [data-testid="stTextInputRootElement"]:hover { border-color: var(--sfts-faint) !important; }
[data-testid="stSelectbox"] [role="group"]:focus-within, [data-testid="stTextInputRootElement"]:focus-within {
  border-color: var(--sfts-accent) !important; box-shadow: 0 0 0 3px var(--sfts-accent-soft) !important;
}
[data-testid="stTextInputRootElement"] > div { border: 0 !important; background: transparent !important; }
[data-testid="stSelectbox"] input, [data-testid="stTextInput"] input {
  color: var(--sfts-text) !important; background: transparent !important; caret-color: var(--sfts-accent);
}
[data-testid="stSelectbox"] button, [data-testid="stSelectbox"] svg { color: var(--sfts-muted) !important; }
.stApp input::placeholder, .stApp textarea::placeholder { color: var(--sfts-faint) !important; opacity: 1 !important; -webkit-text-fill-color: var(--sfts-faint) !important; }
[data-testid="stTextInput"] button, [data-testid="stTextInput"] svg { color: var(--sfts-muted) !important; }
[role="listbox"] { background: var(--sfts-card) !important; border: 1px solid var(--sfts-line) !important; border-radius: 10px !important; box-shadow: var(--sfts-shadow) !important; }
[role="listbox"] [role="option"] { color: var(--sfts-text) !important; border-radius: 6px; font-size: 14px; }
[role="listbox"] [role="option"]:hover, [role="listbox"] [role="option"][data-focused="true"] { background: var(--sfts-sunken) !important; }
[role="listbox"] [role="option"][aria-selected="true"] { color: var(--sfts-accent-strong) !important; font-weight: 600; }
[data-testid="stSlider"] [role="slider"] { background: var(--sfts-accent) !important; box-shadow: none !important; }
[data-testid="stCaptionContainer"], [data-testid="stCaptionContainer"] p { color: var(--sfts-muted) !important; font-size: 12px !important; }

/* ── Segmented controls as quiet tabs: 42 track, 34 segments ── */
[data-testid="stButtonGroup"] [role="radiogroup"] {
  background: var(--sfts-sunken) !important; border: 1px solid var(--sfts-line) !important;
  border-radius: 10px !important; padding: 3px !important; gap: 2px !important; height: var(--h-ctrl) !important;
  box-sizing: border-box !important; display: inline-flex !important; flex-wrap: nowrap !important;
}
[data-testid="stButtonGroup"] button {
  background: transparent !important; border: none !important; box-shadow: none !important;
  border-radius: 7px !important; color: var(--sfts-muted) !important; font-weight: 500 !important;
  height: var(--h-small) !important; min-height: var(--h-small) !important; padding: 0 12px !important;
  display: inline-flex !important; align-items: center !important; justify-content: center !important;
  white-space: nowrap !important; min-width: max-content !important;
  transition: background-color var(--d-fast) var(--e-out), color var(--d-fast) var(--e-out), box-shadow var(--d-fast) var(--e-out) !important;
}
[data-testid="stButtonGroup"] button:hover { color: var(--sfts-text) !important; }
[data-testid="stButtonGroup"] button[aria-checked="true"] {
  background: var(--sfts-card) !important; color: var(--sfts-text) !important;
  box-shadow: 0 1px 2px rgba(15,15,15,.06), inset 0 0 0 1px var(--sfts-line) !important;
}
[data-testid="stButtonGroup"] button[aria-checked="true"]::before { color: var(--sfts-accent); }
[data-testid="stButtonGroup"] button p { overflow: visible !important; text-overflow: clip !important; white-space: nowrap !important; }
[class*="st-key-content_mode"] { align-self: flex-end !important; }  /* mode control on the card's right edge */
[class*="st-key-content_mode"] [data-testid="stButtonGroup"] { display: flex; justify-content: flex-end; align-items: center; gap: 4px; }
[class*="st-key-source_type"] [data-testid="stWidgetLabel"],
[class*="st-key-content_mode"] [data-testid="stWidgetLabel"] { display: none !important; }

/* ── Drop zone: the hero of the Translate page ── */
[data-testid="stFileUploader"] > [data-testid="stWidgetLabel"] { display: none !important; }
[data-testid="stFileUploaderDropzone"] {
  display: flex !important; flex-direction: column !important; align-items: center !important; justify-content: center !important;
  gap: 12px !important; min-height: 264px !important; padding: 24px !important;
  background: transparent !important; border: 1px dashed var(--sfts-line-strong) !important;
  border-radius: 10px !important; cursor: pointer;
  transition: border-color var(--d-fast) var(--e-out), background-color var(--d-fast) var(--e-out);
}
[data-testid="stFileUploaderDropzone"]:hover { border-color: var(--sfts-accent) !important; background: var(--sfts-accent-soft) !important; }
[data-testid="stFileUploaderDropzone"]::before {
  content: var(--sfts-drop-title); display: block; order: -1; padding-top: 108px; min-width: 168px; text-align: center;
  background: var(--sfts-art) top center / 168px 96px no-repeat;
  font-size: 15px; line-height: 20px; font-weight: 600; color: var(--sfts-text);
}
[data-testid="stFileUploaderDropzone"] > span { order: 1; }
[data-testid="stFileUploaderDropzone"] button { min-width: 144px; }
[data-testid="stFileUploaderDropzone"] button [data-testid="stIconMaterial"] { font-size: 0 !important; width: 16px; height: 16px; display: inline-block;
  background-color: currentColor; -webkit-mask: var(--sfts-upload) center / contain no-repeat; mask: var(--sfts-upload) center / contain no-repeat;
  transition: transform 300ms var(--e-out); }
[data-testid="stFileUploaderDropzone"]:hover button [data-testid="stIconMaterial"] { transform: translateY(-2.4px); }
[data-testid="stFileUploaderDropzone"] button [data-testid="stMarkdownContainer"] p { font-size: 0 !important; }
[data-testid="stFileUploaderDropzone"] button [data-testid="stMarkdownContainer"] p::after { content: var(--sfts-drop-browse); font-size: 14px; }
[data-testid="stFileUploaderDropzoneInstructions"] { order: 2; }
[data-testid="stFileUploaderDropzoneInstructions"] span, [data-testid="stFileUploaderDropzoneInstructions"] div { font-size: 0 !important; }
[data-testid="stFileUploaderDropzoneInstructions"]::after {
  content: var(--sfts-drop-formats); display: block; text-align: center; max-width: 480px;
  font-family: var(--f-mono); font-size: 11px; line-height: 16px; letter-spacing: .12em; text-transform: uppercase; color: var(--sfts-faint);
}
[data-testid="stFileUploaderFile"], [data-testid="stFileUploaderFileData"], [data-testid="stFileUploader"] small { display: none !important; }

/* ── Picked file row: the keyed container is the chip; its 34 remove button sits 3px inside the right edge ── */
[class*="st-key-filechip"] {
  height: var(--h-ctrl) !important; min-height: var(--h-ctrl) !important; box-sizing: border-box !important;
  flex-wrap: nowrap !important; align-items: center !important; gap: 8px !important;
  background: var(--sfts-sunken) !important; border: 1px solid var(--sfts-line) !important; border-radius: 10px !important;
  padding: 0 3px 0 12px !important;
  animation: sfts-enter 240ms var(--e-out) both;
}
[class*="st-key-filechip"] > [data-testid="stElementContainer"]:first-child { flex: 1 1 auto !important; min-width: 0; width: auto !important; }
.sfts-filechip { display: flex; align-items: center; gap: 8px; min-width: 0; }
.sfts-filechip .sfts-ico { color: var(--sfts-accent); margin: 0; }
.sfts-filechip-name { font-weight: 600; color: var(--sfts-text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sfts-filechip-size { color: var(--sfts-muted); font-family: var(--f-mono); font-size: 11px; margin-left: auto; }

/* ── Running ── */
.sfts-run { display: flex; align-items: center; gap: 8px; height: 20px; font-weight: 600; color: var(--sfts-text); }
.sfts-run img { width: 20px; height: 20px; display: block; }
.sfts-run span { color: var(--sfts-muted); font-weight: 400; font-family: var(--f-mono); font-size: 12px; margin-left: auto; }
@keyframes sfts-pulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: .35; transform: scale(.8); } }
/* 1.65: an (empty) label row, then track > fill; the fill moves by transform */
[data-testid="stProgress"] > div:not([role="progressbar"]) { display: none !important; }
[data-testid="stProgressBarTrack"] { background: var(--sfts-sunken) !important; height: 3px !important; border-radius: 3px !important; overflow: hidden; }
[data-testid="stProgress"] [role="progressbar"] > div > div {
  height: 3px !important; border-radius: 3px !important; position: relative; overflow: hidden;
  background: var(--sfts-accent) !important; transition: transform var(--d-norm) var(--e-out) !important;
}
/* shimmer: a lighter band of the same hue slides along the filled part */
[data-testid="stProgress"] [role="progressbar"] > div > div::after {
  content: ""; position: absolute; inset: 0; width: 40%;
  background: linear-gradient(90deg, transparent, rgba(255,255,255,.45), transparent);
  animation: sfts-shimmer 1.4s var(--e-out) infinite;
}
/* one file has no fraction to show: a 30% band slides along the track */
.sfts-bar { height: 3px; border-radius: 3px; background: var(--sfts-sunken); position: relative; overflow: hidden; }
.sfts-bar::after {
  content: ""; position: absolute; top: 0; bottom: 0; left: 0; width: 30%; border-radius: 3px;
  background: var(--sfts-accent); animation: sfts-ind 1.6s ease-in-out infinite alternate;
}
@keyframes sfts-ind { from { transform: translateX(-20%); } to { transform: translateX(253%); } }  /* 30% band, 6% past each end */
[data-testid="stProgress"] p { font-family: var(--f-mono) !important; font-size: 11px !important; color: var(--sfts-muted) !important; }
@keyframes sfts-shimmer { from { transform: translateX(-100%); } to { transform: translateX(250%); } }

/* ── Per-file rows: 3px left bar = state ── */
.sfts-files { display: flex; flex-direction: column; gap: 4px; max-height: 324px; overflow: auto; }
.sfts-file {
  display: flex; align-items: center; gap: 12px; height: 34px; box-sizing: border-box; padding: 0 12px; border-radius: 6px;
  background: var(--sfts-raised); box-shadow: inset 3px 0 0 var(--sfts-wait); font-size: 13px;
  transition: box-shadow var(--d-norm) var(--e-out), background-color var(--d-norm) var(--e-out);
}
.sfts-file b { font-weight: 500; font-family: var(--f-mono); font-size: 12px; color: var(--sfts-text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.sfts-file span { color: var(--sfts-muted); margin-left: auto; white-space: nowrap; font-size: 12px; overflow: hidden; text-overflow: ellipsis; }
.sfts-file[data-s="run"] { box-shadow: inset 3px 0 0 var(--sfts-accent); }
.sfts-file[data-s="run"] span { color: var(--sfts-accent); }
.sfts-file[data-s="done"] { box-shadow: inset 3px 0 0 var(--sfts-ok); }
.sfts-file[data-s="fail"] { box-shadow: inset 3px 0 0 var(--sfts-err); background: var(--sfts-err-soft); }
.sfts-file[data-s="fail"] span { color: var(--sfts-err); }
.sfts-file[data-s="skip"] span { color: var(--sfts-faint); }

/* ── Done: a drawn tick, no badge ── */
.sfts-done { display: flex; align-items: flex-start; gap: 12px; }
.sfts-done > .sfts-ico { flex: 0 0 20px; width: 20px; height: 20px; margin: 0; color: var(--sfts-ok); --vi-accent: var(--sfts-ok); }
.sfts-done > .sfts-ico svg { width: 20px; height: 20px; }
.sfts-done[data-tone="warn"] > .sfts-ico { color: var(--sfts-accent); --vi-accent: var(--sfts-accent); }
.sfts-done[data-tone="err"] > .sfts-ico { color: var(--sfts-err); --vi-accent: var(--sfts-err); }
.sfts-detail {
  display: block; font-family: var(--f-mono); font-size: 11px; line-height: 16px; color: var(--sfts-muted);
  background: none; padding: 0; white-space: pre-wrap; word-break: break-word;
}
.sfts-done-title { font-size: 15px; line-height: 20px; font-weight: 600; color: var(--sfts-text); }
.sfts-done-sub { color: var(--sfts-muted); font-size: 13px; line-height: 20px; margin-top: 4px; }
.sfts-path { display: block; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
.sfts-done > div { min-width: 0; flex: 1 1 auto; }
.sfts-done-sub code { font-family: var(--f-mono); font-size: 11px; background: none; color: var(--sfts-faint); padding: 0; word-break: break-all; }
@keyframes sfts-draw { to { stroke-dashoffset: 0; } }

/* ── Toast: bottom centre ink pill (Litora) ── */
[data-testid="stToastContainer"] {
  left: 50% !important; right: auto !important; bottom: 24px !important; top: auto !important;
  transform: translateX(-50%) !important; align-items: center !important;
}
[data-testid="stToast"] {
  background: var(--sfts-toast-bg) !important; color: var(--sfts-toast-ink) !important;
  border-radius: 999px !important; padding: 8px 20px !important; box-shadow: var(--sfts-shadow) !important;
  border: none !important; width: auto !important; max-width: min(70vw, 640px) !important; min-width: 0 !important;
  font-size: 13px !important; animation: sfts-toast-in 180ms var(--e-out);
}
[data-testid="stToast"] * { color: var(--sfts-toast-ink) !important; font-size: 13px !important; }
[data-testid="stToast"] [data-testid="stIconMaterial"] { color: var(--sfts-accent) !important; font-size: 18px !important; }
[data-testid="stToast"] button { display: none !important; }
[data-testid="stToast"] > div, [data-testid="stToast"] [data-testid="stToastIcon"] { align-items: center !important; }
[data-testid="stToast"] p { margin: 0 !important; line-height: 20px !important; }
@keyframes sfts-toast-in { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: none; } }

/* ── Text bits ── */
.sfts-muted { color: var(--sfts-muted); font-size: 12px; line-height: 20px; }
.sfts-note { color: var(--sfts-muted); font-size: 13px; line-height: 20px; }
.sfts-note .sfts-ico { color: var(--sfts-accent-strong); }
.sfts-warn { color: var(--sfts-err); font-size: 13px; line-height: 20px; }
.sfts-row-label { color: var(--sfts-text); font-size: 14px; font-weight: 500; line-height: 42px; }
.sfts-pane-title { font-family: var(--f-disp); font-size: 22px; line-height: 28px; height: 28px; font-weight: 600; letter-spacing: -.01em; margin: 0; color: var(--sfts-text); }
.sfts-key-name { font-size: 14px; font-weight: 600; line-height: 20px; display: flex; align-items: center; gap: 8px; }
.sfts-ico { display: inline-flex; align-items: center; justify-content: center; width: 16px; height: 16px; vertical-align: -3px; margin-right: 6px; }
.sfts-ico svg { width: 16px; height: 16px; display: block; }
.sfts-lang-count { color: var(--sfts-accent-strong); font-size: 12px; line-height: 20px; }
.sfts-pill-on, .sfts-pill-off {
  display: inline-flex; align-items: center; gap: 4px; height: 20px; padding: 0 8px; border-radius: 4px;
  font-family: var(--f-mono); font-size: 11px; font-weight: 500; letter-spacing: .04em;
}
.sfts-pill-on { background: var(--sfts-ok-soft); color: var(--sfts-ok); }
.sfts-pill-off { background: var(--sfts-sunken); color: var(--sfts-muted); }
.sfts-pill-on .sfts-ico, .sfts-pill-off .sfts-ico { margin-right: 0; width: 12px; height: 12px; }
.sfts-pill-on .sfts-ico svg, .sfts-pill-off .sfts-ico svg { width: 12px; height: 12px; }
.sfts-footer { color: var(--sfts-faint); font-size: 11px; line-height: 20px; margin: 0; font-family: var(--f-mono); letter-spacing: .06em; }
[data-testid="stExpander"] details { border: 1px solid var(--sfts-line) !important; border-radius: 10px !important; background: transparent !important; }
[data-testid="stExpander"] summary { color: var(--sfts-muted) !important; background: transparent !important; min-height: 34px; padding: 0 12px !important; }
[data-testid="stExpander"] summary p { font-family: var(--f-mono); font-size: 11px !important; letter-spacing: .14em; text-transform: uppercase; }
hr { border-color: var(--sfts-line) !important; margin: 0 !important; }

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { animation-duration: 1ms !important; animation-iteration-count: 1 !important; transition-duration: 1ms !important; }
  .sfts-wm-a, .sfts-wm-b { animation: none !important; }
  .sfts-bar::after { width: 100%; animation: vi-fade 1.2s ease-in-out infinite alternate !important; }  /* busy, but still */
}
"""


def _chrome_keys(theme: str, page: str, pane: str) -> str:
    active_tab = "nav_translate" if page == "translate" else "nav_settings"
    active_pane = f"pane_{pane}" if pane else "pane_appearance"
    seg = '[class*="st-key-{key}"] [data-testid="stButtonGroup"] button:nth-of-type({n})'
    icons = "".join(
        [
            _mask_icon('[class*="st-key-pane_appearance"] button', "monitor"),
            _mask_icon('[class*="st-key-pane_translation"] button', "globe"),
            _mask_icon('[class*="st-key-pane_keys"] button', "key"),
            _mask_icon('[class*="st-key-pane_glossary"] button', "book"),
            _mask_icon(seg.format(key="source_type", n=1), "file", "0 6px 0 0"),
            _mask_icon(seg.format(key="source_type", n=2), "folder", "0 6px 0 0"),
            _mask_icon(seg.format(key="source_type", n=3), "zip", "0 6px 0 0"),
            _mask_icon(seg.format(key="content_mode", n=1), "bubble", "0 6px 0 0"),
            _mask_icon(seg.format(key="content_mode", n=2), "game", "0 6px 0 0"),
            _mask_icon('[class*="st-key-swap_langs"] button', "swap", "0"),
            _mask_icon('[class*="st-key-theme_toggle"] button', "moon" if theme == "light" else "sun", "0"),
            _mask_icon('[class*="st-key-dl_"] button', "download"),
            _mask_icon('[class*="st-key-open_out"] button', "folder_open"),
            _mask_icon('[class*="st-key-retry_"] button', "retry"),
            _mask_icon('[class*="st-key-cancel_run"] button', "stop", "0 6px 0 0", 12),
            _mask_icon('[class*="st-key-clear_picked"] button', "close", "0", 14),
            _mask_icon('[class*="st-key-gdel_"] button', "close", "0", 14),
        ]
    )
    seg_file, seg_folder, seg_zip = (seg.format(key="source_type", n=n) for n in (1, 2, 3))
    theme_in = "vi-moon-in" if theme == "light" else "vi-rays-in"  # the glyph now showing swings in
    return f"""
/* Top tabs: plain text + accent underline, 42 tall */
[class*="st-key-nav_translate"] button, [class*="st-key-nav_settings"] button {{
  background: transparent !important; border: none !important; box-shadow: none !important;
  border-radius: 0 !important; padding: 0 4px !important; color: var(--sfts-muted) !important;
  border-bottom: 2px solid transparent !important;
}}
[class*="st-key-nav_translate"] button:hover, [class*="st-key-nav_settings"] button:hover {{ color: var(--sfts-text) !important; }}
[class*="st-key-{active_tab}"] button {{
  color: var(--sfts-text) !important; font-weight: 600 !important; border-bottom-color: var(--sfts-accent) !important;
}}
[class*="st-key-nav_"] {{ display: flex; justify-content: flex-end; }}
/* Square icon buttons */
[class*="st-key-theme_toggle"] button, [class*="st-key-swap_langs"] button,
[class*="st-key-clear_picked"] button, [class*="st-key-gdel_"] button {{
  width: var(--h-ctrl) !important; min-width: var(--h-ctrl) !important; padding: 0 !important;
  background: transparent !important; border: 1px solid var(--sfts-line) !important; color: var(--sfts-muted) !important;
}}
[class*="st-key-clear_picked"] button, [class*="st-key-gdel_"] button {{ border-color: transparent !important; }}
[class*="st-key-clear_picked"] button {{
  width: var(--h-small) !important; min-width: var(--h-small) !important; height: var(--h-small) !important; min-height: var(--h-small) !important;
  border-radius: 7px !important;
}}
[class*="st-key-theme_toggle"] button:hover, [class*="st-key-swap_langs"] button:hover:not(:disabled),
[class*="st-key-clear_picked"] button:hover {{
  color: var(--sfts-accent-strong) !important; border-color: var(--sfts-accent) !important; background: var(--sfts-accent-soft) !important;
}}
[class*="st-key-gdel_"] button:hover {{ color: var(--sfts-err) !important; background: var(--sfts-err-soft) !important; }}
[class*="st-key-theme_toggle"] button > div, [class*="st-key-swap_langs"] button > div,
[class*="st-key-clear_picked"] button > div, [class*="st-key-gdel_"] button > div {{ display: none !important; }}
[class*="st-key-theme_toggle"] {{ display: flex; justify-content: flex-end; }}
/* Click motion (FX_JS): .is-press once per click, Litora's press set; .vi-play on swap and on a theme change */
.stApp button.is-press::before {{ animation: vi-pop 440ms cubic-bezier(.3, .7, .3, 1) 1; }}
[class*="st-key-dl_"] button.is-press::before, {seg_folder}.is-press::before,
[data-testid="stFileUploaderDropzone"] button.is-press [data-testid="stIconMaterial"] {{ animation: vi-drop-press 440ms cubic-bezier(.3, .7, .3, 1) 1; }}
[class*="st-key-pane_keys"] button.is-press::before, [class*="st-key-pane_glossary"] button.is-press::before {{
  animation: vi-lean 440ms cubic-bezier(.3, .7, .3, 1) 1; transform-origin: 50% 85%;
}}
[class*="st-key-retry_"] button.is-press::before, [class*="st-key-pane_translation"] button.is-press::before {{
  animation: vi-spin-press 560ms cubic-bezier(.3, .7, .3, 1) 1;
}}
[class*="st-key-open_out"] button.is-press::before, {seg_file}.is-press::before, {seg_zip}.is-press::before {{
  animation: vi-open 440ms cubic-bezier(.3, .7, .3, 1) 1;
}}
[class*="st-key-swap_langs"] button.vi-play::before {{ animation: vi-flip 380ms var(--e-out) 1; }}
[class*="st-key-theme_toggle"] button.vi-play::before {{ animation: {theme_in} 380ms var(--e-out) 1; }}

/* Translator field: a button dressed as a select */
[class*="st-key-provider_chip"] button {{
  width: 100% !important; justify-content: space-between !important; padding: 0 12px !important;
  background: var(--sfts-raised) !important; border: 1px solid var(--sfts-line-strong) !important; color: var(--sfts-text) !important;
}}
[class*="st-key-provider_chip"] button > div {{ flex: 1 1 auto !important; justify-content: flex-start !important; text-align: left !important; min-width: 0; }}
[class*="st-key-provider_chip"] button p {{ overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
[class*="st-key-provider_chip"] button::after {{ content: "›"; color: var(--sfts-muted); font-size: 18px; line-height: 1; margin-left: 8px; }}
[class*="st-key-provider_chip"] button:hover {{ border-color: var(--sfts-accent) !important; }}
[class*="st-key-provider_chip_warn"] button {{ color: var(--sfts-accent-strong) !important; border-color: var(--sfts-accent) !important; background: var(--sfts-accent-soft) !important; }}

/* Small buttons: 34 */
[class*="st-key-cancel_run"] button, [class*="st-key-pane_"] button, [class*="st-key-update_settings"] button {{
  min-height: var(--h-small) !important; height: var(--h-small) !important;
}}
[class*="st-key-cancel_run"] button {{ padding: 0 12px !important; }}
[class*="st-key-cancel_run"] button p {{ font-size: 13px !important; }}

/* Settings rail */
[class*="st-key-pane_"] button, [class*="st-key-update_settings"] button {{
  background: transparent !important; border: none !important; box-shadow: none !important;
  justify-content: flex-start !important; color: var(--sfts-muted) !important; width: 100% !important; padding: 0 12px !important;
}}
[class*="st-key-pane_"] button > div, [class*="st-key-update_settings"] button > div {{ flex: 1 1 auto !important; justify-content: flex-start !important; text-align: left !important; }}
[class*="st-key-pane_"] button:hover {{ background: var(--sfts-sunken) !important; color: var(--sfts-text) !important; }}
[class*="st-key-pane_"] button p {{ overflow: hidden !important; text-overflow: ellipsis !important; white-space: nowrap !important; }}
[class*="st-key-{active_pane}"] button {{
  background: var(--sfts-accent-soft) !important; box-shadow: inset 3px 0 0 var(--sfts-accent) !important;
  color: var(--sfts-accent-strong) !important; font-weight: 600 !important;
}}
[class*="st-key-update_settings"] button p {{ font-size: 12px !important; text-decoration: underline dotted; text-underline-offset: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
[class*="st-key-update_settings"] button:hover {{ color: var(--sfts-accent-strong) !important; }}
{icons}
"""


# Icon click motion (ICON_CSS contract). st.html runs this in the page itself; the flag keeps one
# set of listeners per tab across reruns. Reduced motion binds nothing (the CSS stops it too).
FX_JS = """<script>
(() => {
  if (window.__versoraFx || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  window.__versoraFx = true;
  const replay = (el, cls) => {
    clearTimeout(el.__fxT);
    el.classList.remove(cls); void el.offsetWidth; el.classList.add(cls);
    el.__fxT = setTimeout(() => el.classList.remove(cls), 900);  // hosts whose icon has no motion
  };
  document.addEventListener('animationend', (e) => {
    const host = e.animationName.startsWith('vi-') && e.target.closest('.is-press, .vi-play');
    if (host) { clearTimeout(host.__fxT); host.classList.remove('is-press', 'vi-play'); }
  }, true);
  const THEME = '.st-key-theme_toggle button';
  const glyph = (el) => { const cs = getComputedStyle(el, '::before'); return cs.maskImage || cs.webkitMaskImage; };
  document.addEventListener('click', (e) => {
    const host = e.target.closest && e.target.closest('button, a, label, [role="button"]');
    if (!host || host.disabled) return;
    if (host.matches('.st-key-swap_langs button')) return replay(host, 'vi-play');
    if (host.matches(THEME)) {  // play once the rerun has swapped sun and moon
      const was = glyph(host), t0 = performance.now();
      const wait = () => {
        const btn = document.querySelector(THEME);
        if (btn && glyph(btn) !== was) return replay(btn, 'vi-play');
        if (performance.now() - t0 < 3000) requestAnimationFrame(wait);
      };
      return requestAnimationFrame(wait);
    }
    replay(host, 'is-press');
  }, true);
})();
</script>"""


def css_for(
    theme: str,
    page: str = "translate",
    pane: str = "appearance",
    strings: dict[str, str] | None = None,
) -> str:
    """``strings`` fills CSS-drawn copy (drop zone title, browse, formats) in the UI language."""
    theme = theme if theme in _TOKENS else "light"
    copy = ""
    for name, text in (strings or {}).items():
        safe = str(text).replace("\\", "\\\\").replace('"', '\\"').replace("\n", " ").replace("<", "")
        copy += f'  --sfts-{name}: "{safe}";\n'
    return (
        "<style>\n" + _vars(theme) + (":root {\n" + copy + "}\n" if copy else "")
        + _SHARED + ICON_CSS + _chrome_keys(theme, page, pane) + "</style>"
    )
