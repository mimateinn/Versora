#!/usr/bin/env python3
"""
Versora — local Streamlit file translator.
Run: streamlit run app.py
"""

from __future__ import annotations

import base64
import html
import io
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
from pathlib import Path

import streamlit as st

from src.batch import (
    CANCELLED,
    MAX_CONCURRENCY,
    MIN_CONCURRENCY,
    DEFAULT_CONCURRENCY,
    BatchReport,
    clamp_concurrency,
    translate_single_file,
    translate_tree,
    translate_zip,
)
from src.config import get_default_provider, list_available_providers, outputs_dir
from src.extractors import SUPPORTED_SUFFIXES, is_supported
from src.game_text import SCRIPT_SUFFIXES
from src.glossary import ensure_project, list_projects, load_glossary, save_glossary
from src.i18n import FALLBACK_LANG, available_languages, detect_ui_language, language_display_name, t
from src.models import default_model, models_for
from src.prompts import PRESETS as PURPOSE_PRESETS, PURPOSES, custom_path, load as load_purpose, save_custom, system_prompt
from src.providers import cli as cli_engine
from src.providers.demo import demo_enabled
from src.security.secrets import load_secret, redact_secrets, remove_secret_from_env, save_secret_to_env
from src import runtime
from src.icons import ALERT, CHECK, DASH, FILE, GLOBE, SPINNER, ZIP, wrap
from src.theme import FX_JS, css_for
from src import __version__
from src.ui_prefs import load_prefs, save_prefs

ROOT = Path(__file__).resolve().parent
ICON_PATH = ROOT / "icon.png"
UPLOAD_TYPES = sorted({s.lstrip(".") for s in SUPPORTED_SUFFIXES} | {"zip", "markdown", "htm"})
# no-break space before each dot: a wrapped line can end with "·" but never start with one
FORMATS_LINE = "\u00a0· ".join("txt md docx pdf json csv yaml po xliff xlsx html srt vtt".split())
TARGET_CODES = [
    "zh-Hant", "zh-Hans", "en", "ja", "ko", "es", "fr", "de", "pt", "vi", "th", "id", "other",
]
PROVIDER_OPTIONS = ["auto", "claude_cli", "codex_cli", "grok_cli", "openai", "anthropic", "gemini", "xai"] + (
    ["demo"] if demo_enabled() else []
)
SETTINGS_PANES = ("purposes", "keys", "order", "glossary", "appearance")
# Translate choices live in plain keys. Their widgets use other keys and copy back on
# change: Streamlit deletes a widget's key on any run that does not draw it, which is
# what reset these every time Settings was opened.
CHOICE_DEFAULTS = {
    "target_lang": "en",
    "source_choice": "auto",
    "source_type": "file",
    "purpose": "general",
    "target_other": "",
}
TOAST_ICON = {"ok": ":material/check_circle:", "warn": ":material/info:", "error": ":material/error:"}
WM_WORDS = ["VERSORA", "TRANSLATE", "翻譯", "TRADUIRE", "ÜBERSETZEN", "翻訳", "TRADUCIR", "번역"]


def _browser_accept_language() -> str:
    try:
        headers = getattr(getattr(st, "context", None), "headers", None) or {}
        return str(headers.get("Accept-Language") or headers.get("accept-language") or "")
    except Exception:
        return ""


def _init_state() -> None:
    prefs = load_prefs()
    langs = available_languages()
    follow = prefs.get("ui_lang_follow")
    if follow is None:
        follow = prefs.get("ui_lang") not in langs
    saved_lang = prefs.get("ui_lang")
    if follow or saved_lang not in langs:
        saved_lang = detect_ui_language(_browser_accept_language())
        if saved_lang not in langs:
            saved_lang = FALLBACK_LANG if FALLBACK_LANG in langs else langs[0]
        follow = True if prefs.get("ui_lang_follow") is None else bool(follow)
    saved_provider = prefs.get("provider") if prefs.get("provider") in PROVIDER_OPTIONS else get_default_provider()
    saved_models = prefs.get("model_by_provider") if isinstance(prefs.get("model_by_provider"), dict) else {}
    defaults = {
        "ui_lang": saved_lang,
        "ui_lang_follow": bool(follow),
        "theme": prefs.get("theme") if prefs.get("theme") in {"light", "dark"} else "light",
        "uploader_nonce": 0,
        "picked_name": None,
        "picked_size": 0,
        "picked_bytes": None,
        "page": "translate",
        "settings_pane": "purposes",
        "provider": saved_provider,
        "model_by_provider": dict(saved_models),
        "model": saved_models.get(saved_provider) or default_model(saved_provider),
        "concurrency": clamp_concurrency(prefs.get("concurrency", DEFAULT_CONCURRENCY)),
        "project": "default",
        "glossary_pairs": None,
        "glossary_nonce": 0,
        "result": None,  # single file: {"src", "out", "path"}
        "batch_report": None,
        "batch_job": None,  # how to rerun the job for "retry failed"
        "batch_zip": None,
        "single_error": None,  # single file failed: {"name", "size", "msg"}
        "batch_error": None,  # a whole batch could not run: {"name", "msg", "missing"?}
        "open_error": None,
        "toasts": [],
        "folder_path": "",
        **{k: prefs.get(k, v) for k, v in CHOICE_DEFAULTS.items()},
    }
    if "purpose" not in prefs and prefs.get("content_mode") == "game":  # v0.2.0 "Game text" switch
        defaults["purpose"] = "game"
    for key, val in defaults.items():
        if key not in st.session_state:
            st.session_state[key] = val
    if st.session_state.glossary_pairs is None:
        st.session_state.glossary_pairs = load_glossary(st.session_state.project)


_init_state()


def L(key: str, **kwargs) -> str:
    return t(key, st.session_state.ui_lang, **kwargs)


st.set_page_config(
    page_title="Versora",
    page_icon=str(ICON_PATH),
    layout="wide",
    initial_sidebar_state="collapsed",
)
_qp = st.query_params
if _qp.get("page") in {"translate", "settings"}:
    st.session_state.page = str(_qp.get("page"))
if _qp.get("theme") in {"light", "dark"}:
    st.session_state.theme = str(_qp.get("theme"))
if _qp.get("pane") in SETTINGS_PANES:
    st.session_state.settings_pane = str(_qp.get("pane"))
st.markdown(
    css_for(
        st.session_state.theme,
        st.session_state.page,
        st.session_state.settings_pane,
        {
            "drop-title": L("drop.zip_title") if st.session_state.source_type == "zip" else L("drop.title"),
            "drop-browse": L("drop.browse"),
            "drop-formats": "zip" if st.session_state.source_type == "zip" else FORMATS_LINE,
            "drop-hint": L("drop.zip_hint") if st.session_state.source_type == "zip" else L("drop.hint"),
        },
    ),
    unsafe_allow_html=True,
)


def _watermark() -> str:
    """Even rows in one layer, odd rows in the other; each row starts on a different word.
    Each row holds its sequence twice so a -50% roll loops without a seam."""
    sep = "  ·  "

    def row(k: int) -> str:
        words = WM_WORDS[k % len(WM_WORDS):] + WM_WORDS[: k % len(WM_WORDS)]
        unit = sep.join(words) + sep
        return f"<div>{unit * 12}</div>"

    rows_a = "".join(row(k * 2) for k in range(48))
    rows_b = "".join(row(k * 2 + 3) for k in range(48))
    return (
        '<div class="sfts-wm" aria-hidden="true"><div class="sfts-wm-field">'
        f'<div class="sfts-wm-layer sfts-wm-a">{rows_a}</div>'
        f'<div class="sfts-wm-layer sfts-wm-b">{rows_b}</div></div></div>'
    )


st.markdown(_watermark(), unsafe_allow_html=True)
st.html(FX_JS, unsafe_allow_javascript=True)  # click motion for icons; binds once per browser tab


# ── state helpers ────────────────────────────────────────────────────────────


def _toast(msg: str, tone: str = "ok") -> None:
    """Queue a toast. Shown at the top of the next run, so it survives st.rerun()."""
    st.session_state.toasts.append((msg, tone))


def _drain_toasts() -> None:
    queued, st.session_state.toasts = list(st.session_state.toasts), []
    for msg, tone in queued:  # 4 s, matched by the exit animation in theme.py (TOAST_MS)
        st.toast(msg, icon=TOAST_ICON.get(tone, TOAST_ICON["ok"]), duration=4)


def _sync_query() -> None:
    st.query_params["page"] = st.session_state.page
    st.query_params["theme"] = st.session_state.theme
    if st.session_state.page == "settings":
        st.query_params["pane"] = st.session_state.settings_pane
    elif "pane" in st.query_params:
        del st.query_params["pane"]


def _go(page: str, pane: str | None = None) -> None:
    st.session_state.page = page
    if pane in SETTINGS_PANES:
        st.session_state.settings_pane = pane
    _sync_query()
    st.rerun()


def _persist_prefs() -> None:
    save_prefs(
        theme=st.session_state.theme,
        ui_lang=st.session_state.ui_lang,
        ui_lang_follow=bool(st.session_state.get("ui_lang_follow", True)),
        provider=st.session_state.provider,
        model_by_provider=dict(st.session_state.model_by_provider or {}),
        concurrency=st.session_state.concurrency,
        **{k: st.session_state.get(k) for k in CHOICE_DEFAULTS},
    )


def _bound(widget_key: str, canon: str) -> dict:
    """Widget kwargs that mirror plain key ``canon``. Seeds the widget each time it reappears."""
    if widget_key not in st.session_state:
        st.session_state[widget_key] = st.session_state[canon]
    return {"key": widget_key, "on_change": _copy_back, "args": (widget_key, canon)}


def _copy_back(widget_key: str, canon: str) -> None:
    val = st.session_state.get(widget_key)
    if val is None:  # a segmented control clicked off
        st.session_state[widget_key] = st.session_state[canon]
        return
    st.session_state[canon] = val
    if canon == "source_type":
        _forget_pick()
    _persist_prefs()


def _set_theme(theme: str) -> None:
    if theme not in {"light", "dark"}:
        return
    st.session_state.theme = theme
    _persist_prefs()
    _sync_query()
    st.rerun()


def _set_lang(lang: str, *, follow: bool = False) -> None:
    st.session_state.ui_lang = lang
    st.session_state.ui_lang_follow = follow
    _persist_prefs()
    st.rerun()


@st.cache_data(show_spinner=False)
def _icon_data_uri() -> str:
    """The vector mark: a 24px PNG downscale of the 512 icon reads soft."""
    path = ROOT / "assets" / "icon.svg"
    raw = path.read_bytes() if path.is_file() else b""
    return "data:image/svg+xml;base64," + base64.b64encode(raw).decode("ascii") if raw else ""


@st.cache_data(show_spinner=False)
def _busy_icon_uri() -> str:
    """The animated app mark (badge lifts and stamps) shown while a job runs."""
    path = ROOT / "assets" / "icon-animated.svg"
    raw = path.read_bytes() if path.is_file() else b""
    return "data:image/svg+xml;base64," + base64.b64encode(raw).decode("ascii")


def _fmt_size(n: int) -> str:
    if n < 1024:
        return f"{n} B"
    if n < 1024 * 1024:
        return f"{n / 1024:.0f} KB"
    return f"{n / (1024 * 1024):.1f} MB"


def _target_lang() -> str:
    choice = st.session_state.target_lang
    return choice if choice != "other" else (st.session_state.get("target_other") or "en").strip()


def _source_lang() -> str | None:
    choice = st.session_state.source_choice
    return None if choice == "auto" else choice


def _lang_label(code: str) -> str:
    return L("sidebar.source_auto") if code == "auto" else L(f"target.{code}")


def _run_overlay() -> str:
    script = ROOT / "scripts" / "sfts_overlay.py"
    try:
        proc = subprocess.run(
            [sys.executable, str(script), "--apply"],
            capture_output=True,
            text=True,
            cwd=str(ROOT),
            check=False,
            timeout=180,
        )
        return (proc.stdout or "") + (proc.stderr or "")
    except Exception:
        return ""


def _open_folder(path: str) -> str | None:
    """Open ``path`` in the file manager; returns an error sentence (shown inline) or None."""
    try:
        if sys.platform.startswith("win"):
            os.startfile(path)  # type: ignore[attr-defined]
        else:
            subprocess.Popen(["open" if sys.platform == "darwin" else "xdg-open", path])
    except Exception as e:
        return L("main.status_error", msg=redact_secrets(str(e)))
    return None


# Error text from providers and parsers → one plain sentence.
_ERROR_HINTS = (
    (CANCELLED, "err.cancelled"),
    ("[timeout]", "err.timeout"), ("[limit]", "err.rate"), ("[refused]", "err.refused"),
    ("[empty]", "err.blank"), ("[truncated]", "err.truncated"),
    ("logged in", "err.signin"), ("login", "err.signin"), ("[auth]", "err.key"), ("[spawn]", "err.spawn"),
    ("401", "err.key"), ("403", "err.key"), ("api key", "err.key"), ("unauthorized", "err.key"),
    ("authentication", "err.key"),
    ("429", "err.rate"), ("rate limit", "err.rate"), ("quota", "err.rate"), ("overloaded", "err.rate"),
    ("timed out", "err.timeout"), ("timeout", "err.timeout"),
    ("connect", "err.network"), ("network", "err.network"), ("name resolution", "err.network"),
    ("empty text", "err.empty"), ("nothing to translate", "err.empty"),
)
_SKIP_HINTS = (
    ("unsupported", "skip.type"), ("binary", "skip.binary"), ("nested zip", "skip.zip"),
    ("overwrite", "skip.overwrite"), ("unsafe", "skip.unsafe"), ("zip-slip", "skip.unsafe"),
    ("absolute path", "skip.unsafe"), ("outside", "skip.unsafe"),
)


def _hint(msg: str, hints=_ERROR_HINTS) -> str | None:
    low = (msg or "").lower()
    for needle, key in hints:
        if needle in low:
            return L(key)
    return None


def _human(msg: str, hints=_ERROR_HINTS) -> str:
    text = redact_secrets(msg or "").strip()
    return _hint(msg, hints) or (text if len(text) <= 160 else text[:157] + "...") or L("err.generic")


# ── chrome ──────────────────────────────────────────────────────────────────


def render_chrome() -> None:
    brand, nav = st.columns([1, 1], vertical_alignment="center", gap="small")
    with brand:
        uri = _icon_data_uri()
        img = f'<img src="{uri}" alt="">' if uri else ""
        st.markdown(f'<div class="sfts-brand"><span class="sfts-tile">{img}</span><b>{L("app.title")}</b></div>', unsafe_allow_html=True)
    dark = st.session_state.theme == "dark"
    with nav, st.container(key="topnav", horizontal=True, horizontal_alignment="right", vertical_alignment="center"):
        busy = _busy()
        if st.button(L("nav.translate"), key="nav_translate", disabled=busy):
            _go("translate")
        if st.button(L("nav.settings"), key="nav_settings", disabled=busy):
            _go("settings")
        if st.button(
            L("theme.light") if dark else L("theme.dark"),
            key="theme_toggle",
            help=L("theme.light") if dark else L("theme.dark"),
            disabled=busy,
        ):
            _set_theme("light" if dark else "dark")


# ── settings ────────────────────────────────────────────────────────────────


def _check_update() -> None:
    """Runs as a pending action (the button shows "Checking…" first); the outcome stays on the row."""
    out = _run_overlay()
    if "STATUS=UPDATED" in out:
        st.session_state.update_status = ("warn", L("update.reopen"))
    elif "STATUS=UP_TO_DATE" in out:
        st.session_state.update_status = ("ok", L("update.up_to_date"))
    else:
        st.session_state.update_status = ("err", L("update.failed"))


def _pend(action: str, pid: str = "") -> None:
    """Mark a slow action; the pane draws its button as pending, then runs it at the end of the run."""
    st.session_state.svc_pending = (action, pid)


def _pending(action: str, pid: str = "") -> bool:
    return st.session_state.get("svc_pending") == (action, pid)


def _pending_button(label: str, busy_label: str, key: str, action: str, pid: str = "", **kw) -> None:
    if _pending(action, pid):
        with st.container(key=f"svc_pending_{key}"):
            st.button(busy_label, key=key, disabled=True)
    else:
        st.button(label, key=key, on_click=_pend, args=(action, pid), disabled=bool(st.session_state.get("svc_pending")), **kw)


def _run_pending() -> None:
    """After the pane is drawn (pending buttons visible): run the one slow action, then redraw."""
    pend = st.session_state.get("svc_pending")
    if not pend:
        return
    action, pid = pend
    try:
        if action == "recheck":
            _recheck(pid)
        elif action == "test":
            _test_service(pid)
        elif action == "save_test":
            value = (st.session_state.get(f"paste_{pid}") or "").strip()
            if value:
                save_secret_to_env(API_KEY_NAMES[pid], value)
                list_available_providers(fresh=True)
                st.session_state.pop(f"svc_edit_{pid}", None)
                st.session_state.pop("svc_add", None)
                _test_service(pid)
        elif action == "update":
            _check_update()
    finally:
        st.session_state.svc_pending = None
    st.rerun()


def render_appearance_pane() -> None:
    langs = available_languages()
    detected = detect_ui_language(_browser_accept_language())
    if detected not in langs:
        detected = FALLBACK_LANG if FALLBACK_LANG in langs else langs[0]
    now_name = language_display_name(detected, st.session_state.ui_lang)
    st.markdown(f'<div class="sfts-pane-title">{L("card.appearance")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_appearance"):
        lang_labels = {code: language_display_name(code, st.session_state.ui_lang) for code in langs}
        options = ["__system__"] + langs
        current = "__system__" if st.session_state.get("ui_lang_follow") else (
            st.session_state.ui_lang if st.session_state.ui_lang in langs else "__system__"
        )
        lab, ctl = st.columns([1.1, 2.2], vertical_alignment="center")
        with lab:
            st.markdown(f'<div class="sfts-row-label">{L("sidebar.language")}</div>', unsafe_allow_html=True)
        with ctl:
            chosen = st.selectbox(
                L("sidebar.language"),
                options=options,
                index=options.index(current) if current in options else 0,
                format_func={**lang_labels, "__system__": L("lang.follow_system", name=now_name)}.get,
                key="ui_lang_select",
                label_visibility="collapsed",
            )
        if chosen == "__system__":
            if not st.session_state.get("ui_lang_follow") or st.session_state.ui_lang != detected:
                _set_lang(detected, follow=True)
        elif chosen != st.session_state.ui_lang or st.session_state.get("ui_lang_follow"):
            _set_lang(chosen, follow=False)
        st.markdown(f'<div class="sfts-muted">{L("card.lang_fallback")} {L("card.lang_count")}</div>', unsafe_allow_html=True)
        st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)
        with st.container(key="about_row", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
            st.markdown(f'<div class="sfts-row-label">{L("about.version", v=__version__)}</div>', unsafe_allow_html=True)
            _pending_button(L("update.button"), L("svc.checking"), "update_settings", "update")
        status = st.session_state.get("update_status")
        if status:
            st.markdown(f'<div class="sfts-test" data-tone="{status[0]}">{html.escape(status[1])}</div>', unsafe_allow_html=True)
    _run_pending()


def _copy_purpose() -> None:
    st.session_state["purpose_body"] = load_purpose(st.session_state.get("purpose_from") or "general")[1]


def _save_purpose() -> None:
    st.session_state.purpose_saved = save_custom(st.session_state.get("purpose_body") or "")


def render_purposes_pane() -> None:
    """Custom purpose: start from any preset, edit, save (each save is a new version)."""
    st.markdown(f'<div class="sfts-pane-title">{L("card.purposes")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_purpose"):
        version, body = load_purpose("custom") if custom_path().is_file() else (0, load_purpose("general")[1])
        if "purpose_body" not in st.session_state:
            st.session_state["purpose_body"] = body
        st.markdown(f'<div class="sfts-note">{L("purpose.hint")}</div>', unsafe_allow_html=True)
        with st.container(key="purpose_from_row", horizontal=True, vertical_alignment="bottom", gap="small", wrap=False):
            st.selectbox(
                L("purpose.start_from"), options=list(PURPOSE_PRESETS),
                format_func={p: L(f"purpose.{p}") for p in PURPOSE_PRESETS}.get, key="purpose_from",
            )
            st.button(L("purpose.copy"), key="purpose_copy", on_click=_copy_purpose)
        st.text_area(L("purpose.instructions"), key="purpose_body", height=220)
        with st.container(key="purpose_save_row", horizontal=True, horizontal_alignment="right", vertical_alignment="center"):
            saved = st.session_state.pop("purpose_saved", None)
            if saved:
                st.markdown(f'<div class="sfts-test" data-tone="ok">{wrap(CHECK)}{L("purpose.saved", v=saved)}</div>', unsafe_allow_html=True)
            elif version:
                st.markdown(f'<div class="sfts-muted">{L("purpose.version", v=version)}</div>', unsafe_allow_html=True)
            st.button(L("purpose.save"), key="purpose_save", type="primary", on_click=_save_purpose,
                      disabled=not (st.session_state.get("purpose_body") or "").strip())


API_KEY_NAMES = {"openai": "OPENAI_API_KEY", "anthropic": "ANTHROPIC_API_KEY", "gemini": "GEMINI_API_KEY", "xai": "XAI_API_KEY"}
API_HOSTS = {"openai": "api.openai.com", "anthropic": "api.anthropic.com", "gemini": "generativelanguage.googleapis.com", "xai": "api.x.ai"}
# Where each vendor hands out developer keys (the user opens these; Versora never calls them).
API_KEY_PAGES = {"openai": "https://platform.openai.com/api-keys", "anthropic": "https://console.anthropic.com/settings/keys",
                 "gemini": "https://aistudio.google.com/apikey", "xai": "https://console.x.ai"}
SERVICE_NAMES = {"claude_cli": "Claude Code", "codex_cli": "Codex", "grok_cli": "Grok", "openai": "OpenAI",
                 "anthropic": "Anthropic", "gemini": "Gemini", "xai": "xAI", "demo": "Demo"}
EFFORTS = ("", "low", "medium", "high")


def _services() -> list[str]:
    return [*cli_engine.PRESETS, *API_KEY_NAMES] + (["demo"] if demo_enabled() else [])


def _saved_key(env_name: str, value: str) -> None:
    save_secret_to_env(env_name, value)
    cli_engine.forget_status()
    list_available_providers(fresh=True)
    _toast(L("sidebar.save_key_ok"))
    st.rerun()


def _removed_key(pid: str) -> None:
    remove_secret_from_env(API_KEY_NAMES[pid])
    st.session_state.pop(f"svc_remove_{pid}", None)
    st.session_state.get("svc_test", {}).pop(pid, None)
    list_available_providers(fresh=True)
    _toast(L("svc.removed"))


def _recheck(pid: str) -> None:
    cli_engine.probe(pid, fresh=True)
    list_available_providers(fresh=True)


def _chain_rows() -> list[dict]:
    """The saved order, plus every known translator not yet in it (appended, on)."""
    rows = [r for r in load_prefs().get("chain", []) if r["id"] in _services()]
    have = {r["id"] for r in rows}
    return rows + [{"id": pid, "model": "", "effort": "", "enabled": True} for pid in _services() if pid not in have]


def _save_chain(rows: list[dict]) -> None:
    save_prefs(chain=rows)


def _chain_set(pid: str, field: str, widget_key: str) -> None:
    rows = _chain_rows()
    for row in rows:
        if row["id"] == pid:
            value = st.session_state.get(widget_key)
            row[field] = bool(value) if field == "enabled" else (value or "")
    _save_chain(rows)


def _chain_move(pid: str, step: int) -> None:
    rows = _chain_rows()
    i = next(n for n, r in enumerate(rows) if r["id"] == pid)
    j = max(0, min(len(rows) - 1, i + step))
    rows[i], rows[j] = rows[j], rows[i]
    _save_chain(rows)


def _service_state(pid: str) -> tuple[str, str]:
    """(state key, tone) for the chip: ready / signed_out / limit / setup / retrying / unchecked."""
    h = runtime.health(pid)
    now = time.time()
    if h.get("retry_until", 0) > now:
        return "retrying", "warn"
    if h.get("kind") == "limit" and now - h.get("at", 0) < 900:
        return "limit", "warn"
    test = st.session_state.get("svc_test", {}).get(pid)
    if pid == "demo":
        return "ready", "ok"
    if test and test[0] == "err":
        return ("signed_out", "err") if test[2] in (L("err.signin"), L("err.key")) else ("setup", "err")
    if pid in cli_engine.PRESETS:
        status = cli_engine.probe(pid)
        if not status.present:
            return "setup", "off"
        if status.logged_in is False or h.get("kind") == "auth":
            return "signed_out", "err"
        if status.logged_in or (test and test[0] == "ok") or (h and h.get("kind") is None):
            return "ready", "ok"
        return "unchecked", "off"
    if not load_secret(API_KEY_NAMES[pid]) or pid not in list_available_providers() or h.get("kind") == "auth":
        return "setup", "off" if not h.get("kind") else "err"
    if (test and test[0] == "ok") or (h and h.get("kind") is None):
        return "ready", "ok"
    return "unchecked", "off"


def _test_service(pid: str) -> None:
    """Translate one short sentence with this translator alone; keep how long it took and when."""
    row = next((r for r in _chain_rows() if r["id"] == pid), {})
    started = time.perf_counter()
    try:
        engine = runtime.make_engine(runtime.Link(pid, row.get("model", ""), row.get("effort", "")))
        target = _target_lang()
        reply = engine.complete(
            system_prompt("general", target, "en"),
            f"Target language: {target}\nTranslate each numbered line.\n\n1. Hello! The file is ready.",
        )
        text = runtime.parse_numbered(reply, 1)[0] if reply.strip().startswith("1") else reply.strip()
        runtime.note(pid, None)
        result = ("ok", time.perf_counter() - started, text[:80])
    except Exception as e:  # shown as a line; nothing here may break the page
        runtime.note(pid, getattr(e, "kind", None) or "spawn")
        result = ("err", time.perf_counter() - started, _human(str(e)))
    st.session_state.setdefault("svc_test", {})[pid] = (*result, time.strftime("%H:%M"))


def _test_line(pid: str) -> None:
    test = st.session_state.get("svc_test", {}).get(pid)
    if not test:
        return
    ok, secs, text, at = test
    msg = L("svc.test_ok", secs=f"{secs:.1f}", time=at) if ok == "ok" else L("svc.test_fail", msg=text)
    st.markdown(f'<div class="sfts-test" data-tone="{ok}">{html.escape(msg)}</div>', unsafe_allow_html=True)


def _service_head(pid: str, kind: str, line: str) -> None:
    state, tone = _service_state(pid)
    st.markdown(
        f'<div class="sfts-svc"><i class="sfts-dot" data-tone="{tone}"></i><b>{html.escape(SERVICE_NAMES[pid])}</b>'
        f'<span class="sfts-chip">{html.escape(kind)}</span>'
        f'<span class="sfts-state" data-tone="{tone}">{html.escape(L(f"st.{state}"))}</span></div>'
        f'<code class="sfts-svc-line">{html.escape(line)}</code>',
        unsafe_allow_html=True,
    )


def _key_editor(pid: str) -> None:
    """One field for one vendor: paste, Save and test; the vendor's key page beside it."""
    st.markdown(
        f'<div class="sfts-muted">{html.escape(L("keys.local_plain"))} '
        f'<a href="{API_KEY_PAGES[pid]}" target="_blank" rel="noopener">{L("svc.get_key")}</a></div>',
        unsafe_allow_html=True,
    )
    with st.container(key=f"row_{pid}", horizontal=True, vertical_alignment="bottom", gap="small", wrap=False):
        st.text_input(L("svc.paste", name=SERVICE_NAMES[pid]), key=f"paste_{pid}", type="password",
                      label_visibility="collapsed", placeholder=L("svc.paste", name=SERVICE_NAMES[pid]))
        if load_secret(API_KEY_NAMES[pid]) or st.session_state.get("svc_add") == pid:
            st.button(L("run.cancel"), key=f"svc_cancel_{pid}",
                      on_click=lambda: (st.session_state.pop(f"svc_edit_{pid}", None), st.session_state.pop("svc_add", None)))
        _pending_button(L("svc.save_test"), L("svc.testing"), f"svc_save_{pid}", "save_test", pid, type="primary")


def _service_card(pid: str) -> None:
    is_cli = pid in cli_engine.PRESETS
    kind = L("svc.local") if is_cli else (L("svc.builtin") if pid == "demo" else L("svc.online"))
    if is_cli:
        status, preset = cli_engine.probe(pid), cli_engine.PRESETS[pid]
        line = f"{preset.bin} · v{status.version}" if status.present and status.version else (
            preset.bin if status.present else f"{preset.bin} · {L('svc.not_found')}")
    elif pid == "demo":
        line = "demo · offline"
    else:
        key = load_secret(API_KEY_NAMES[pid])
        line = f"{API_HOSTS[pid]} · ••••{key[-4:]}" if key and len(key) >= 4 else API_HOSTS[pid]
    with st.container(key=f"svc_{pid}"):
        with st.container(key=f"svc_head_{pid}", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
            _service_head(pid, kind, line)
            if is_cli:
                _pending_button(L("svc.recheck"), L("svc.checking"), f"svc_recheck_{pid}", "recheck", pid)
                if status.present:
                    _pending_button(L("svc.test"), L("svc.testing"), f"svc_test_{pid}", "test", pid)
            elif pid != "demo":
                _pending_button(L("svc.test"), L("svc.testing"), f"svc_test_{pid}", "test", pid)
                st.button(L("svc.replace"), key=f"svc_replace_{pid}",
                          on_click=lambda: st.session_state.update({f"svc_edit_{pid}": True}))
                st.button(L("svc.remove"), key=f"svc_remove_btn_{pid}",
                          on_click=lambda: st.session_state.update({f"svc_remove_{pid}": True}))
        _test_line(pid)
        if is_cli and not status.present:
            st.markdown(f'<div class="sfts-muted">{L("svc.install", url=preset.install_url)}</div>', unsafe_allow_html=True)
        elif is_cli and status.logged_in is False:
            st.markdown(f'<div class="sfts-muted">{L("svc.signin", cmd=f"<code>{preset.login_cmd}</code>")}</div>', unsafe_allow_html=True)
        elif is_cli and status.logged_in is None and _service_state(pid)[0] == "unchecked":
            st.markdown(f'<div class="sfts-muted">{L("svc.unknown")}</div>', unsafe_allow_html=True)
        if not is_cli and pid != "demo":
            if pid not in list_available_providers():
                st.markdown(f'<div class="sfts-warn">{L("svc.blocked")}</div>', unsafe_allow_html=True)
            if st.session_state.get(f"svc_remove_{pid}"):
                with st.container(key=f"svc_confirm_{pid}", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
                    st.markdown(f'<div class="sfts-warn">{L("svc.remove_ask")}</div>', unsafe_allow_html=True)
                    st.button(L("svc.keep"), key=f"svc_keep_{pid}", on_click=lambda: st.session_state.pop(f"svc_remove_{pid}", None))
                    st.button(L("svc.confirm_remove"), key=f"svc_rm_{pid}", type="primary", on_click=_removed_key, args=(pid,))
            if st.session_state.get(f"svc_edit_{pid}"):
                _key_editor(pid)


def _add_online(pid: str) -> None:
    st.session_state.svc_add = pid


def render_keys_pane() -> None:
    """Translators: local programs and configured APIs as rows; the rest as an "add" list."""
    st.markdown(f'<div class="sfts-pane-title">{L("card.translators")}</div>', unsafe_allow_html=True)
    configured = [p for p in API_KEY_NAMES if load_secret(API_KEY_NAMES[p])]
    unset = [p for p in API_KEY_NAMES if p not in configured]
    with st.container(border=True, key="card_services"):
        shown = [*cli_engine.PRESETS, *configured] + (["demo"] if demo_enabled() else [])
        for i, pid in enumerate(shown):
            if i:
                st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)
            _service_card(pid)
        if unset:
            st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)
            adding = st.session_state.get("svc_add")
            with st.container(key="svc_addrow", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
                st.markdown(f'<div class="sfts-row-label">{L("svc.add_online")}</div>', unsafe_allow_html=True)
                for pid in unset:
                    st.button(SERVICE_NAMES[pid], key=f"svc_add_{pid}", on_click=_add_online, args=(pid,),
                              type="primary" if adding == pid else "secondary")
            if adding in unset:
                _test_line(adding)
                _key_editor(adding)
    _run_pending()


def _order_row(n: int, row: dict, last: bool) -> None:
    """One chain row. A translator that is not set up keeps its saved row (model, effort, on/off
    stay in prefs) but its controls are drawn disabled, with "Not set up" and a Set up link."""
    pid = row["id"]
    is_cli = pid in cli_engine.PRESETS
    state, tone = _service_state(pid)
    off = pid not in list_available_providers()
    with st.container(key=f"ord_{pid}", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
        st.markdown(
            f'<div class="sfts-ord"{" data-off" if off else ""}><span class="sfts-rank">{n}</span>'
            f'<i class="sfts-dot" data-tone="{"off" if off else tone}"></i>'
            f'<b>{html.escape(SERVICE_NAMES[pid])}</b></div>',
            unsafe_allow_html=True,
        )
        extra = cli_engine.probe(pid).models if is_cli else ()
        options = models_for(pid, tuple(extra))
        if row.get("model") and row["model"] not in options:
            options = [row["model"], *options]
        st.selectbox(
            L("order.model"), options=options,
            index=None if off else (options.index(row["model"]) if row.get("model") in options else None),
            placeholder=L("order.not_set_up") if off else L("order.model_default"), accept_new_options=True,
            label_visibility="collapsed", disabled=off,
            key=f"ord_model_off_{pid}" if off else f"ord_model_{pid}", on_change=_chain_set, args=(pid, "model", f"ord_model_{pid}"),
        )
        if off:
            st.markdown(f'<div class="sfts-ord-none"><a href="?page=settings&pane=keys">{L("order.set_up")}</a></div>',
                        unsafe_allow_html=True)
        elif is_cli:
            labels = {"": L("order.effort_default"), "low": L("order.low"), "medium": L("order.medium"), "high": L("order.high")}
            st.selectbox(
                L("order.effort"), options=list(EFFORTS), index=EFFORTS.index(row.get("effort") or ""),
                format_func=labels.get, label_visibility="collapsed",
                key=f"ord_eff_{pid}", on_change=_chain_set, args=(pid, "effort", f"ord_eff_{pid}"),
            )
        else:
            st.markdown('<div class="sfts-ord-none">—</div>', unsafe_allow_html=True)
        st.toggle(L("order.col_on"), value=row.get("enabled", True), key=f"ord_on_{pid}", label_visibility="collapsed",
                  disabled=off, on_change=_chain_set, args=(pid, "enabled", f"ord_on_{pid}"))
        st.button(L("order.up"), key=f"ord_up_{pid}", on_click=_chain_move, args=(pid, -1), disabled=n == 1, help=L("order.up"))
        st.button(L("order.down"), key=f"ord_dn_{pid}", on_click=_chain_move, args=(pid, 1), disabled=last, help=L("order.down"))


def _set_limits() -> None:
    save_prefs(concurrency=st.session_state.get("ord_global"), per_provider=st.session_state.get("ord_per"))
    st.session_state.concurrency = clamp_concurrency(st.session_state.get("ord_global"))


def _limit_select(label: str, key: str, bounds: tuple[int, int, int], value) -> None:
    lo, hi, _default = bounds
    opts = list(range(lo, hi + 1))
    with st.container(key=f"{key}_row", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
        st.markdown(f'<div class="sfts-row-label">{label}</div>', unsafe_allow_html=True)
        st.selectbox(label, options=opts, index=opts.index(runtime.clamp(value, bounds)),
                     label_visibility="collapsed", key=key, on_change=_set_limits)


def render_order_pane() -> None:
    """Order: the chain Automatic follows (the Translate page's Translator field picks Automatic or one)."""
    st.markdown(f'<div class="sfts-pane-title">{L("card.order")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_order"):
        st.markdown(f'<div class="sfts-muted">{L("order.hint")}</div>', unsafe_allow_html=True)
        st.markdown(
            f'<div class="sfts-ord-head"><span>{L("order.col_name")}</span><span>{L("order.model")}</span>'
            f'<span>{L("order.effort")}</span><span>{L("order.col_on")}</span><span></span></div>',
            unsafe_allow_html=True,
        )
        rows = _chain_rows()
        for n, row in enumerate(rows, 1):
            _order_row(n, row, n == len(rows))
        st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)
        prefs = load_prefs()
        _limit_select(L("order.files"), "ord_global", runtime.GLOBAL_LIMIT, prefs.get("concurrency", runtime.GLOBAL_LIMIT[2]))
        with st.expander(L("order.advanced")):
            _limit_select(L("order.per"), "ord_per", runtime.PER_LIMIT, prefs.get("per_provider", runtime.PER_LIMIT[2]))
            st.markdown(f'<div class="sfts-muted">{L("order.limits_hint")}</div>', unsafe_allow_html=True)
            for pid, preset in cli_engine.PRESETS.items():
                st.markdown(f'<div class="sfts-flabel">{html.escape(L("order.path", name=preset.name))}</div>', unsafe_allow_html=True)
                saved = _field_and_save(L("order.path", name=preset.name), f"path_{pid}", f"save_path_{pid}",
                                        value=cli_engine.path_setting(pid), placeholder=preset.bin)
                if saved:
                    _saved_key(preset.path_env, saved)


def _field_and_save(label: str, key: str, save_key: str, **field) -> str | None:
    """A field and its Save button on one row; returns the stripped value when Save is clicked."""
    with st.container(key=f"row_{key}", horizontal=True, vertical_alignment="bottom", gap="small", wrap=False):
        val = st.text_input(label, key=key, label_visibility="collapsed", **field)
        clicked = st.button(L("keys.save"), key=save_key)
    return val.strip() if clicked and val.strip() else None


def render_glossary_pane() -> None:
    st.markdown(f'<div class="sfts-pane-title">{L("card.glossary")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_glossary"):
        projects = list_projects()
        if "default" not in projects:
            ensure_project("default")
            projects = list_projects()
        p_idx = projects.index(st.session_state.project) if st.session_state.project in projects else 0
        saved_pairs = load_glossary(st.session_state.project)  # the file is the durable source, not the widgets
        c1, c2, c3 = st.columns([2, 2, 1], vertical_alignment="bottom")
        with c1:
            selected = st.selectbox(L("sidebar.project"), options=projects, index=p_idx, key="project_select")
        with c2:
            new_name = st.text_input(L("sidebar.new_project"), placeholder="name", key="new_project_name")
        with c3:
            create = st.button(L("sidebar.create_project"), key="create_project", use_container_width=True)
        if selected != st.session_state.project:
            st.session_state.project = selected
            st.session_state.glossary_pairs = load_glossary(selected)
            st.session_state.glossary_nonce += 1
            st.rerun()
        if create and new_name.strip():
            ensure_project(new_name.strip())
            st.session_state.project = new_name.strip()
            st.session_state.glossary_pairs = []
            st.session_state.glossary_nonce += 1
            st.rerun()
        st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)

        pairs = list(st.session_state.glossary_pairs or [])
        if not pairs:
            st.markdown(f'<div class="sfts-note">{L("glossary.empty")}</div>', unsafe_allow_html=True)
        else:
            h1, h2, _h3 = st.columns([2, 2, 0.5])
            h1.markdown(f'<div class="sfts-panel-title">{L("glossary.col_src")}</div>', unsafe_allow_html=True)
            h2.markdown(f'<div class="sfts-panel-title">{L("glossary.col_dst")}</div>', unsafe_allow_html=True)
        # Row keys carry a nonce: after a delete every row is rebuilt from `current`,
        # so a value never slides into the row below.
        nonce = st.session_state.glossary_nonce
        current: list[tuple[str, str]] = []
        drop = None
        for i, (term, trans) in enumerate(pairs):
            c1, c2, c3 = st.columns([2, 2, 0.5], vertical_alignment="center")
            with c1:
                nt = st.text_input(L("glossary.term"), value=term, key=f"gterm_{nonce}_{i}", label_visibility="collapsed")
            with c2:
                ntr = st.text_input(L("glossary.translation"), value=trans, key=f"gtr_{nonce}_{i}", label_visibility="collapsed")
            with c3:
                if st.button(L("glossary.delete"), key=f"gdel_{nonce}_{i}", help=L("glossary.delete")):  # CLOSE glyph: theme.py mask
                    drop = i
            current.append((nt, ntr))
        if drop is not None:
            st.session_state.glossary_pairs = [p for j, p in enumerate(current) if j != drop]
            st.session_state.glossary_nonce += 1
            st.rerun()
        # Save only when the normalized rows differ from the saved file (a removed saved term counts).
        edited = [(a.strip(), b.strip()) for a, b in current if a.strip()]
        with st.container(key="glossary_actions", horizontal=True, horizontal_alignment="distribute",
                          vertical_alignment="center", wrap=False):
            if st.button(L("sidebar.add_term"), key="glossary_add"):
                st.session_state.glossary_pairs = current + [("", "")]
                st.session_state.glossary_nonce += 1
                st.rerun()
            if st.button(L("sidebar.save_glossary"), type="primary", key="glossary_save", disabled=edited == saved_pairs):
                save_glossary(st.session_state.project, edited)
                st.session_state.glossary_pairs = edited
                st.session_state.glossary_nonce += 1
                _toast(L("glossary.saved", name=st.session_state.project))
                st.rerun()


def render_settings() -> None:
    rail, pane = st.columns([1, 3.2], gap="large")
    panes = {  # nav label == pane heading
        "purposes": (L("card.purposes"), render_purposes_pane),
        "keys": (L("card.translators"), render_keys_pane),
        "order": (L("card.order"), render_order_pane),
        "glossary": (L("card.glossary"), render_glossary_pane),
        "appearance": (L("card.appearance"), render_appearance_pane),
    }
    with rail:
        for pane_id in SETTINGS_PANES:
            if st.button(panes[pane_id][0], use_container_width=True, key=f"pane_{pane_id}"):
                _go("settings", pane_id)
    with pane:
        panes.get(st.session_state.settings_pane, panes["purposes"])[1]()


# ── translate ───────────────────────────────────────────────────────────────


def _forget_pick() -> None:
    st.session_state.picked_name = None
    st.session_state.picked_size = 0
    st.session_state.picked_bytes = None
    st.session_state.uploader_nonce = int(st.session_state.get("uploader_nonce") or 0) + 1


def _clear_results() -> None:
    st.session_state.result = None
    st.session_state.batch_report = None
    st.session_state.batch_job = None
    st.session_state.batch_zip = None
    st.session_state.single_error = None
    st.session_state.batch_error = None
    st.session_state.open_error = None


def _busy() -> bool:
    """A job starts after this run's widgets are drawn: everything that would rerun (and so kill
    it) is drawn disabled; only Cancel stays live."""
    return bool(st.session_state.get("job_pending") or st.session_state.get("retry_pending"))


def _swap_langs() -> None:
    src, dst = st.session_state.source_choice, st.session_state.target_lang
    if src == "auto" or dst == "other":
        return
    st.session_state.source_choice, st.session_state.target_lang = dst, src
    st.session_state["qb_source"], st.session_state["qb_target"] = dst, src
    _persist_prefs()


def _translator_label(pid: str) -> str:
    if pid == "auto":
        return L("quick.auto_try")
    if pid == "demo":
        return L("quick.demo_chip")
    return SERVICE_NAMES.get(pid, pid)


def render_quick_bar() -> None:
    """From / swap / To / Purpose / Translator on one row of equal fields; the Translator field is
    the one place to pick Automatic (the Order chain) or a single translator."""
    available = list_available_providers()
    busy = _busy()
    custom = st.session_state.purpose == "custom"
    with st.container(key="quickbar", horizontal=True, vertical_alignment="bottom", gap="small", wrap=False):
        src_opts = ["auto"] + [c for c in TARGET_CODES if c != "other"]
        st.selectbox(L("quick.from"), options=src_opts, format_func={c: _lang_label(c) for c in src_opts}.get,
                     disabled=busy, **_bound("qb_source", "source_choice"))
        can_swap = st.session_state.source_choice != "auto" and st.session_state.target_lang != "other"
        st.button(L("quick.swap"), key="swap_langs", on_click=_swap_langs, disabled=busy or not can_swap,
                  help=L("quick.swap") if can_swap else L("quick.swap_auto"))
        st.selectbox(L("quick.to"), options=TARGET_CODES, format_func={c: L(f"target.{c}") for c in TARGET_CODES}.get,
                     disabled=busy, **_bound("qb_target", "target_lang"))
        # Settings links in labels are dropped while a job runs: following one would rerun and kill it.
        label = L("quick.purpose") + (f' · [{L("purpose.edit")}](?page=settings&pane=purposes)' if custom and not busy else "")
        st.selectbox(label, options=list(PURPOSES), format_func={p: L(f"purpose.{p}") for p in PURPOSES}.get,
                     disabled=busy, **_bound("qb_purpose", "purpose"))
        if available:
            options = ["auto", *available]
            if st.session_state.provider not in options:
                st.session_state.provider = "auto"
                st.session_state.pop("qb_provider", None)
            label = L("quick.translator") + ("" if busy else f' · [{L("quick.manage")}](?page=settings&pane=keys)')
            st.selectbox(label, options=options, format_func={p: _translator_label(p) for p in options}.get,
                         disabled=busy, **_bound("qb_provider", "provider"))
        else:
            with st.container(key="qb_setup"):
                st.markdown(f'<div class="sfts-flabel">{L("quick.translator")}</div>', unsafe_allow_html=True)
                if st.button(L("quick.no_translator"), key="provider_chip_warn", disabled=busy):
                    _go("settings", "keys")
    if st.session_state.target_lang == "other":
        st.text_input(L("sidebar.target_other"), placeholder="e.g. it, nl, pl", disabled=busy, **_bound("qb_other", "target_other"))
    if not available:
        st.markdown(f'<div class="sfts-note">{wrap(ALERT)}{L("main.no_translator")}</div>', unsafe_allow_html=True)


def _job_kwargs() -> dict:
    return dict(
        target_lang=_target_lang(),
        source_lang=_source_lang(),
        project=st.session_state.project,
        provider_choice=st.session_state.provider,
        game_mode=st.session_state.purpose == "game",  # game text: only player-facing strings are sent
        purpose=st.session_state.purpose if st.session_state.purpose in PURPOSES else "general",
        model=None,  # each translator's model lives in its Order row
        concurrency=clamp_concurrency(st.session_state.concurrency),
    )


def _ready() -> bool:
    if list_available_providers():
        return True
    _toast(L("main.no_translator"), "error")  # the quick bar shows the same notice inline
    return False


def _cancel_clicked() -> None:
    # This callback runs on the next pass, after the interrupted run has unwound and its worker
    # (or CLI child) has stopped, so the toast is true when it shows. FX_JS shows "Cancelling…" meanwhile.
    _toast(L("run.cancelled"), "warn")


def _start_job() -> None:
    st.session_state.job_pending = True


def _remove_pick() -> None:
    _forget_pick()
    _clear_results()


def _clear_folder() -> None:
    st.session_state.folder_path = ""
    st.session_state["folder_path_in"] = ""
    _clear_results()


def _out_name(name: str) -> str:
    return Path(name).stem + f".{_target_lang()}" + Path(name).suffix.lower()


def _unique_out(name: str, suffix: str) -> Path:
    out = outputs_dir() / _out_name(name)
    if out.exists():
        out = out.with_name(out.stem + time.strftime(".%Y%m%d-%H%M%S") + out.suffix)
    return out


def _preview_text(raw: bytes) -> str:
    if b"\x00" in raw[:2048]:
        return L("main.binary_preview")
    return raw[:16000].decode("utf-8", errors="replace")[:4000]


def _run_header(indeterminate: bool = False):
    """Status line with Cancel on the right, then a thin bar. Returns (head, bar); bar is None
    when indeterminate (one file has no meaningful fraction)."""
    line, stop = st.columns([6, 1], vertical_alignment="center")
    with line:
        head = st.empty()
    with stop:
        st.button(L("run.cancel"), key="cancel_run", on_click=_cancel_clicked)
    if indeterminate:
        st.markdown('<div class="sfts-bar" role="progressbar" aria-busy="true"></div>', unsafe_allow_html=True)
        return head, None
    return head, st.progress(0.0)


def _run_line(head, text: str, right: str = "") -> None:
    head.markdown(
        f'<div class="sfts-run" data-cancelling="{html.escape(L("run.cancelling"))}">{wrap(SPINNER)}'
        f'{html.escape(text)}<span>{html.escape(right)}</span></div>',
        unsafe_allow_html=True,
    )


# Parsers' words for a file that is not what its name says (docx/xlsx are zips, pdf has a trailer).
_DAMAGED = ("package not found", "not a zip file", "badzipfile", "eof marker", "pdfreaderror", "invalid pdf", "no /root")
_TEMP_SRC = re.compile(r"(?:[A-Za-z]:)?[\\/][^'\"\n]*?versora_[^'\"\n\\/]*[\\/]source\.\w+")
_ABS_PATH = re.compile(r"(?:[A-Za-z]:\\|/(?:tmp|var|home|Users|private)/)[^'\"\s]*")


def _file_reason(msg: str, name: str, size: int) -> str:
    low = (msg or "").lower()
    if any(n in low for n in _DAMAGED):
        return L("err.damaged", ext=(Path(name).suffix.lstrip(".").upper() or "?"), size=_fmt_size(size))
    return _hint(msg) or L("err.generic")


def _detail_text(msg: str, name: str) -> str:
    """Raw error for Details: secrets redacted, our temp copy shown as the user's file name, other paths cut."""
    text = _TEMP_SRC.sub(name, redact_secrets(msg or ""))
    return _ABS_PATH.sub("…", text).strip()


def _run_single() -> None:
    """The file is translated on a worker thread while this thread keeps drawing the status line.
    A Cancel click interrupts that drawing; the finally then sets ``cancel`` (a CLI child is killed
    by run_cli) and waits for the worker, so nothing keeps running behind the next page."""
    name = st.session_state.picked_name
    raw = st.session_state.picked_bytes or b""
    suffix = Path(name).suffix.lower()
    out_path = _unique_out(name, suffix)
    cancel = threading.Event()
    state: dict = {"done": 0, "total": 0, "error": None}
    with st.container(border=True, key="card_run"):
        head, _bar = _run_header(indeterminate=True)
        _run_line(head, L("run.file", name=name), L("run.preparing"))
        with tempfile.TemporaryDirectory(prefix="versora_", ignore_cleanup_errors=True) as tmp:
            src = Path(tmp) / f"source{suffix}"
            src.write_bytes(raw)

            kw = _job_kwargs()  # read session state here: the worker thread has no script context

            def work() -> None:
                try:
                    translate_single_file(src, out_path, cancel=cancel,
                                          on_progress=lambda d, t, _i: state.update(done=d, total=t), **kw)
                except Exception as e:  # handed to the drawing thread
                    state["error"] = e

            worker = threading.Thread(target=work, daemon=True)
            worker.start()
            try:
                while worker.is_alive():
                    total = state["total"]
                    _run_line(head, L("run.file", name=name), L("run.chunks", done=state["done"], total=total) if total > 1 else "")
                    worker.join(0.3)
            finally:
                cancel.set()
                worker.join()
    if state["error"] is not None:
        st.session_state.single_error = {"name": name, "size": len(raw), "msg": str(state["error"])}
        st.rerun()
    binary = out_path.suffix.lower() in {".docx", ".pdf", ".xlsx"}
    st.session_state.result = {
        "src": _preview_text(raw),
        "out": L("main.saved_binary") if binary else out_path.read_text(encoding="utf-8", errors="replace")[:4000],
        "path": str(out_path),
    }
    _toast(L("main.status_done"))
    st.rerun()


def _file_row(rel: str, state: str, note: str) -> str:
    return f'<div class="sfts-file" data-s="{state}"><b>{html.escape(rel)}</b><span>{html.escape(note)}</span></div>'


def _run_cap() -> int:
    """How many files can really be translating at once: files in flight, the global call limit and
    the per-translator limit times the translators in play, whichever is smallest."""
    prefs = load_prefs()
    g = runtime.clamp(prefs.get("concurrency"), runtime.GLOBAL_LIMIT)
    per = runtime.clamp(prefs.get("per_provider"), runtime.PER_LIMIT)
    try:
        links = max(1, len(runtime.resolve_chain(st.session_state.provider)))
    except Exception:
        links = 1
    return max(1, min(clamp_concurrency(st.session_state.concurrency), g, per * links))


def _files_html(report: BatchReport, cap: int) -> str:
    done = {i.rel for i in report.written}
    failed = {i.rel: i for i in report.failed}
    running = [r for r in report.started if r not in done and r not in failed][:cap]  # real starts only
    rows = []
    for rel in report.planned:
        if rel in done:
            rows.append(_file_row(rel, "done", L("state.done")))
        elif rel in failed:
            rows.append(_file_row(rel, "fail", _human(failed[rel].error)))
        elif rel in running:
            rows.append(_file_row(rel, "run", L("state.running")))
        else:
            rows.append(_file_row(rel, "wait", L("state.waiting")))
    return '<div class="sfts-files">' + "".join(rows) + "</div>"


def _run_batch(job: dict, only: set[str] | None = None) -> None:
    """Run a folder or zip job with live per-file rows. ``only`` = retry those files."""
    report = BatchReport()
    previous = st.session_state.batch_report if only else None
    if not only:
        st.session_state.batch_report = report
        st.session_state.batch_job = job
    st.session_state.batch_zip = None
    cap = _run_cap()
    with st.container(border=True, key="card_run"):
        head, bar = _run_header()
        _run_line(head, L("run.preparing"))
        rows = st.empty()
        shown = {"rows": ""}

        def progress(done: int, total: int, _item) -> None:
            _run_line(head, L("run.batch", total=total), L("run.files", done=done, total=total))
            bar.progress(min(1.0, done / total) if total else 0.0)
            now = _files_html(report, cap)
            if now != shown["rows"]:  # same states: leave the rows' DOM (and its scroll) alone
                shown["rows"] = now
                rows.markdown(now, unsafe_allow_html=True)

        try:
            kw = dict(_job_kwargs(), report=report, on_progress=progress, only=only)
            if job["kind"] == "folder":
                root = Path(job["path"])
                translate_tree(root, job_name=root.name, **kw)
            else:
                with tempfile.TemporaryDirectory(prefix="versora_zip_", ignore_cleanup_errors=True) as tmp:
                    zpath = Path(tmp) / "upload.zip"
                    zpath.write_bytes(job["bytes"])
                    translate_zip(zpath, Path(tmp) / "tree", job_name=job["name"], **kw)
        except Exception as e:  # the whole job could not run: a card that stays, not a toast
            if not only:
                st.session_state.batch_report = None
            st.session_state.batch_error = {"name": job.get("name") or Path(job.get("path", "")).name, "msg": str(e)}
            st.rerun()
    if previous is not None:  # merge a retry into the shown report
        previous.written += report.written
        previous.failed = [f for f in previous.failed if f.rel not in only] + report.failed
        previous.cancelled = report.cancelled
        report = previous
        st.session_state.batch_report = report
    if not report.cancelled:
        _toast(L("toast.batch_failed") if report.failed else L("toast.batch_done"), "warn" if report.failed else "ok")
    st.rerun()


def _zip_outputs(report: BatchReport) -> bytes:
    buf = io.BytesIO()
    root = Path(report.output_root)
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        for item in report.written:
            p = Path(item.out)
            if p.is_file():
                zf.write(p, p.relative_to(root).as_posix() if p.is_relative_to(root) else p.name)
    return buf.getvalue()


def _done_header(title: str, sub: str, warn: bool = False, tone: str | None = None) -> None:
    st.markdown(
        f'<div class="sfts-done {"vi-anim-alert" if warn else "vi-anim-check"}" data-tone="{tone or ("warn" if warn else "ok")}">'
        f'{wrap(ALERT if warn else CHECK)}'
        f'<div><div class="sfts-done-title">{html.escape(title)}</div>'
        + (f'<div class="sfts-done-sub">{sub}</div>' if sub else "") + "</div></div>",
        unsafe_allow_html=True,
    )


def _result_head(title: str, sub: str, actions, warn: bool = False, tone: str | None = None) -> None:
    """Card header (fixed height): what happened on the left, one action bar on the right."""
    with st.container(key="result_head", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
        _done_header(title, sub, warn, tone)
        actions()
    if st.session_state.get("open_error"):
        st.markdown(f'<div class="sfts-warn">{html.escape(st.session_state.open_error)}</div>', unsafe_allow_html=True)


def _path_sub(path: str, extra: str = "") -> str:
    """The output's name, with the full path as its tooltip and a small Copy path (FX_JS copies)."""
    name = Path(path).name or path
    return (f'{extra}<span class="sfts-path" title="{html.escape(path)}">{html.escape(name)}</span>'
            f'<button type="button" class="sfts-copy" data-copy="{html.escape(path)}" data-done="{html.escape(L("result.copied"))}">'
            f'{html.escape(L("result.copy_path"))}</button>')


def _open(path: str) -> None:
    st.session_state.open_error = None
    err = _open_folder(path)
    if err:
        st.session_state.open_error = err


def render_single_result() -> None:
    res = st.session_state.result
    path = Path(res["path"])

    def actions() -> None:
        if path.parent.is_dir():
            st.button(L("batch.open_folder"), key="open_out_single", on_click=_open, args=(str(path.parent),))
        if path.is_file():
            st.download_button(
                L("main.download"), data=path.read_bytes(), file_name=path.name,
                mime="application/octet-stream", key="dl_single", type="primary",
            )

    with st.container(border=True, key="card_result"):
        _result_head(L("done.file"), _path_sub(str(path)), actions)
        with st.expander(L("result.show_preview")):
            c1, c2 = st.columns(2, gap="medium")
            with c1:
                st.markdown(f'<div class="sfts-panel-title">{L("main.preview_src")}</div>'
                            f'<pre class="sfts-pre">{html.escape(res["src"])}</pre>', unsafe_allow_html=True)
            with c2:
                st.markdown(f'<div class="sfts-pre-col"><div class="sfts-panel-title">{L("main.preview_out")}</div>'
                            f'<pre class="sfts-pre">{html.escape(res["out"])}</pre></div>', unsafe_allow_html=True)


def _error_card(reason: str, name: str, detail: str, choose) -> None:
    """One error component for a file or a whole batch: the reason, then exactly one retry
    (primary) and one other way out (choose another). ``name`` may be "" (the file row above
    already names a single file); Details stays collapsed."""

    def actions() -> None:
        st.button(L("main.choose_another"), key="choose_another", on_click=choose, disabled=_busy())
        st.button(L("main.try_again"), key="retry_single", type="primary", on_click=_start_job, disabled=_busy())

    with st.container(border=True, key="card_error"):
        _result_head(reason, html.escape(name), actions, warn=True, tone="err")
        if detail and detail != reason:
            with st.expander(L("main.details")):
                st.markdown(f'<code class="sfts-detail">{html.escape(detail)}</code>', unsafe_allow_html=True)


def render_single_error() -> None:
    err = st.session_state.single_error
    _error_card(_file_reason(err["msg"], err["name"], int(err.get("size") or 0)), "",
                _detail_text(err["msg"], err["name"]), _remove_pick)


def render_batch_error() -> None:
    err = st.session_state.batch_error
    missing = err.get("missing")
    reason = L("main.folder_missing") if missing else _human(err["msg"])
    _error_card(reason, err["name"], "" if missing else _detail_text(err["msg"], err["name"]),
                _clear_folder if st.session_state.source_type == "folder" else _remove_pick)


def render_batch_result() -> None:
    report: BatchReport = st.session_state.batch_report
    job = st.session_state.batch_job
    n_ok, n_fail, n_skip = len(report.written), len(report.failed), len(report.skipped)
    parts = [L("done.n_saved", n=n_ok)]
    if n_fail:
        parts.append(L("done.n_failed", n=n_fail))
    if n_skip:
        parts.append(L("done.n_skipped", n=n_skip))
    title = L("run.cancelled") if report.cancelled else (L("done.batch") if not n_fail else L("done.batch_some"))

    def actions() -> None:
        if n_fail and job:  # runs at page level on the next pass
            st.button(L("batch.retry", n=n_fail), key="retry_failed", on_click=lambda: st.session_state.update(retry_pending=True))
        if report.output_root and Path(report.output_root).is_dir():
            st.button(L("batch.open_folder"), key="open_out", on_click=_open, args=(report.output_root,))
        if n_ok:
            if st.session_state.batch_zip is None:
                st.session_state.batch_zip = _zip_outputs(report)
            st.download_button(
                L("batch.download_all"), data=st.session_state.batch_zip,
                file_name=Path(report.output_root).name + ".zip", mime="application/zip", key="dl_all", type="primary",
            )

    def group(label: str, items, state: str, note) -> None:
        st.markdown(f'<div class="sfts-panel-title">{label} · {len(items)}</div>', unsafe_allow_html=True)
        st.markdown('<div class="sfts-files">' + "".join(_file_row(i.rel, state, note(i)) for i in items) + "</div>", unsafe_allow_html=True)

    with st.container(border=True, key="card_result"):
        _result_head(title, _path_sub(report.output_root, " · ".join(parts) + " · "), actions, warn=bool(n_fail))
        if n_fail:  # problems first and open; the saved list is a folded summary
            group(L("batch.failed"), report.failed, "fail", lambda i: _human(i.error))
        if n_skip:
            group(L("batch.skipped"), report.skipped, "skip", lambda i: _human(i.skipped or i.error, _SKIP_HINTS))
        if n_ok:
            saved = sorted(report.written, key=lambda i: i.rel)
            with st.expander(f'{L("batch.saved")} · {n_ok}'):
                st.markdown('<div class="sfts-files">' + "".join(_file_row(i.rel, "done", L("state.done")) for i in saved) + "</div>", unsafe_allow_html=True)


def _has_result() -> bool:
    """A result card is on screen for the current source type (then Download leads, not Translate)."""
    if st.session_state.source_type == "file":
        return bool(st.session_state.result or st.session_state.single_error)
    return st.session_state.batch_report is not None or bool(st.session_state.get("batch_error"))


def _has_error() -> bool:
    if st.session_state.source_type == "file":
        return bool(st.session_state.single_error)
    return bool(st.session_state.get("batch_error"))


def _translate_button(disabled: bool = False) -> None:
    """The page's one start button: filled until a result exists, then outlined "Translate again";
    hidden while an error card offers Try again; disabled with a pulse while its job runs."""
    busy = _busy()
    if _has_error() and not busy:
        return
    again = _has_result() and not busy
    label = L("state.running") if busy else (L("main.translate_again") if again else L("main.translate_btn"))
    with st.container(key="start_busy" if busy else "start_idle"):
        st.button(
            label, key="start_translate", type="secondary" if again else "primary",
            disabled=disabled or busy, on_click=_start_job,
        )


_TYPE_NAMES = {".md": "Markdown", ".markdown": "Markdown", ".docx": "Word", ".xlsx": "Excel", ".htm": "HTML", ".yml": "YAML"}


def _show_file_row() -> None:
    """The picked file, compact, in the drop zone's place: icon, name, size · type, the output name,
    then Translate and a round remove button at the right edge."""
    name = st.session_state.picked_name
    size = int(st.session_state.picked_size or 0)
    suffix = Path(name).suffix.lower()
    kind = st.session_state.source_type
    problem = None
    if kind == "file" and suffix == ".zip":
        problem = L("main.zip_use_zip_mode")
    elif kind == "file" and not is_supported(name) and suffix not in SCRIPT_SUFFIXES:
        problem = L("error.unsupported_format")
    type_name = _TYPE_NAMES.get(suffix, suffix.lstrip(".").upper())
    out = f"archive_{Path(name).stem}/" if kind == "zip" else _out_name(name)
    if kind == "file" and st.session_state.result:  # the name really written, collision suffix included
        out = Path(st.session_state.result["path"]).name
    before, _, after = (html.escape(s) for s in L("main.saves_as", name="\x00").partition("\x00"))
    with st.container(key="filerow", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
        st.markdown(
            f'<div class="sfts-frow">{wrap(ZIP if suffix == ".zip" else FILE)}'
            f'<b title="{html.escape(name)}">{html.escape(name)}</b>'
            f'<span class="sfts-frow-meta">{_fmt_size(size)} · {html.escape(type_name)}</span>'
            f'<span class="sfts-frow-out">{before}<code>{html.escape(out)}</code>{after}</span></div>',
            unsafe_allow_html=True,
        )
        _translate_button(bool(problem))
        st.button(L("main.clear"), key="clear_picked", help=L("main.clear"), on_click=_remove_pick, disabled=_busy())
    if problem:
        st.markdown(f'<div class="sfts-warn">{problem}</div>', unsafe_allow_html=True)


def _pick_upload(kind: str) -> None:
    nonce = int(st.session_state.get("uploader_nonce") or 0)
    up = st.file_uploader(
        L("main.zip_upload") if kind == "zip" else L("main.upload"),
        type=["zip"] if kind == "zip" else UPLOAD_TYPES,
        key=f"{kind}_up_{nonce}",
        label_visibility="collapsed",
    )
    if up is not None:
        st.session_state.picked_name = up.name
        st.session_state.picked_bytes = up.getvalue()
        st.session_state.picked_size = len(st.session_state.picked_bytes)
        _clear_results()
        st.rerun()


def render_translate() -> None:
    """One card: language row, then source tabs, then the drop zone (or the picked file's row)."""
    busy = _busy()
    with st.container(border=True, key="card_main"):
        render_quick_bar()
        st.markdown('<hr class="sfts-rule">', unsafe_allow_html=True)
        with st.container(key="source_row", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
            st.segmented_control(
                L("main.source_type"),
                options=["file", "folder", "zip"],
                format_func={"file": L("main.seg_file"), "folder": L("main.seg_folder"), "zip": L("main.seg_zip")}.get,
                required=True,
                label_visibility="collapsed",
                disabled=busy,
                **_bound("source_type_seg", "source_type"),
            )
            if st.session_state.picked_name and not busy and not _has_result():  # a selected, idle file only
                st.markdown(f'<div class="sfts-muted">{L("main.switch_clears")}</div>', unsafe_allow_html=True)
        kind = st.session_state.source_type
        with st.container(key="sourcebody"):
            if kind in {"file", "zip"}:
                name = st.session_state.picked_name
                if name and kind == "zip" and not name.lower().endswith(".zip"):
                    _forget_pick()
                    name = None
                if not name:
                    _pick_upload(kind)
                else:
                    _show_file_row()
            else:
                st.markdown(f'<div class="sfts-note">{L("main.folder_hint")}</div>', unsafe_allow_html=True)
                with st.container(key="folder_row", horizontal=True, vertical_alignment="center", gap="small", wrap=False):
                    folder = st.text_input(
                        L("main.folder_path"),
                        label_visibility="collapsed",
                        placeholder=L("main.folder_placeholder"),
                        disabled=busy,
                        **_bound("folder_path_in", "folder_path"),
                    )
                    _translate_button(not folder.strip())

    go = st.session_state.pop("job_pending", False)
    if go and kind in {"file", "zip"} and not st.session_state.picked_name:
        go = False
    if go and _ready():
        _clear_results()
        if kind == "file":
            _run_single()
        elif kind == "zip":
            _run_batch({"kind": "zip", "bytes": st.session_state.picked_bytes, "name": Path(st.session_state.picked_name).stem})
        else:
            root = Path(st.session_state.folder_path.strip()).expanduser()
            if not root.is_dir():
                st.session_state.batch_error = {"name": str(root), "msg": "", "missing": True}
                st.rerun()
            _run_batch({"kind": "folder", "path": str(root), "name": root.name})
    elif go:
        st.rerun()

    if st.session_state.pop("retry_pending", False) and st.session_state.batch_report and st.session_state.batch_job:
        _run_batch(st.session_state.batch_job, only={f.rel for f in st.session_state.batch_report.failed})

    if kind == "file" and st.session_state.result:
        render_single_result()
    if kind == "file" and st.session_state.single_error:
        render_single_error()
    if kind in {"folder", "zip"} and st.session_state.get("batch_error"):
        render_batch_error()
    if kind in {"folder", "zip"} and st.session_state.batch_report is not None:
        render_batch_result()


render_chrome()
_sync_query()
_drain_toasts()
if st.session_state.page == "settings":
    render_settings()
else:
    render_translate()

st.markdown(f'<div class="sfts-footer"><span>{L("about.footer")} · v{__version__}</span></div>', unsafe_allow_html=True)
