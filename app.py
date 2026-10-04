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
import subprocess
import sys
import tempfile
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
from src.models import default_model, models_for, resolve_model
from src.providers.codex_cli import INSTALL_HINT as CODEX_HINT
from src.providers.codex_cli import codex_cli_path_setting, probe_codex_cli
from src.providers.demo import demo_enabled
from src.providers.grok_cli import INSTALL_HINT as GROK_HINT
from src.providers.grok_cli import grok_cli_path_setting, probe_grok_cli
from src.security.secrets import load_secret, redact_secrets, save_secret_to_env
from src.icons import CHECK, DASH, FILE, GLOBE, wrap
from src.theme import css_for
from src.ui_prefs import load_prefs, save_prefs

ROOT = Path(__file__).resolve().parent
ICON_PATH = ROOT / "icon.png"
UPLOAD_TYPES = sorted({s.lstrip(".") for s in SUPPORTED_SUFFIXES} | {"zip", "markdown", "htm"})
FORMATS_LINE = "txt · md · docx · pdf · json · csv · yaml · po · xliff · xlsx · html · srt · vtt"
TARGET_CODES = [
    "zh-Hant", "zh-Hans", "en", "ja", "ko", "es", "fr", "de", "pt", "vi", "th", "id", "other",
]
PROVIDER_OPTIONS = ["auto", "openai", "anthropic", "gemini", "xai", "grok_cli", "codex_cli"] + (
    ["demo"] if demo_enabled() else []
)
PROVIDER_SHORT = {
    "openai": "OpenAI", "anthropic": "Claude", "gemini": "Gemini", "xai": "xAI",
    "grok_cli": "Grok CLI", "codex_cli": "Codex CLI", "demo": "Demo",
}
SETTINGS_PANES = ("appearance", "translation", "keys", "glossary")
KEY_ROWS = (
    ("OpenAI", "OPENAI_API_KEY", "keys.openai"),
    ("Anthropic", "ANTHROPIC_API_KEY", "keys.anthropic"),
    ("Gemini API", "GEMINI_API_KEY", "keys.gemini"),
    ("Grok / xAI API key", "XAI_API_KEY", "keys.xai"),
)
# Translate choices live in plain keys. Their widgets use other keys and copy back on
# change: Streamlit deletes a widget's key on any run that does not draw it, which is
# what reset these every time Settings was opened.
CHOICE_DEFAULTS = {
    "target_lang": "en",
    "source_choice": "auto",
    "source_type": "file",
    "content_mode": "document",
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
        "settings_pane": "appearance",
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
        "toasts": [],
        "folder_path": "",
        **{k: prefs.get(k, v) for k, v in CHOICE_DEFAULTS.items()},
    }
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


# ── state helpers ────────────────────────────────────────────────────────────


def _toast(msg: str, tone: str = "ok") -> None:
    """Queue a toast. Shown at the top of the next run, so it survives st.rerun()."""
    st.session_state.toasts.append((msg, tone))


def _drain_toasts() -> None:
    queued, st.session_state.toasts = list(st.session_state.toasts), []
    for msg, tone in queued:
        st.toast(msg, icon=TOAST_ICON.get(tone, TOAST_ICON["ok"]))


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
    raw = ICON_PATH.read_bytes() if ICON_PATH.is_file() else b""
    return "data:image/png;base64," + base64.b64encode(raw).decode("ascii") if raw else ""


@st.cache_data(ttl=60, show_spinner=False)
def _probe(which: str):
    return probe_grok_cli() if which == "grok" else probe_codex_cli()


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


def _check_update() -> None:
    with st.spinner(L("update.checking")):
        out = _run_overlay()
    if "STATUS=UPDATED" in out:
        _toast(L("update.reopen"), "warn")
    elif "STATUS=UP_TO_DATE" in out:
        _toast(L("update.up_to_date"))
    else:
        _toast(L("update.failed"), "error")
    st.rerun()


def _open_folder(path: str) -> None:
    try:
        if sys.platform.startswith("win"):
            os.startfile(path)  # type: ignore[attr-defined]
        else:
            subprocess.Popen(["open" if sys.platform == "darwin" else "xdg-open", path])
    except Exception as e:
        _toast(L("main.status_error", msg=redact_secrets(str(e))), "error")


# Error text from providers and parsers → one plain sentence.
_ERROR_HINTS = (
    (CANCELLED, "err.cancelled"),
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


def _human(msg: str, hints=_ERROR_HINTS) -> str:
    low = (msg or "").lower()
    for needle, key in hints:
        if needle in low:
            return L(key)
    text = redact_secrets(msg or "").strip()
    return text if len(text) <= 160 else text[:157] + "..."


# ── chrome ──────────────────────────────────────────────────────────────────


def render_chrome() -> None:
    brand, tab_t, tab_s, theme = st.columns([5.2, 1.1, 1.1, 0.5], vertical_alignment="center", gap="small")
    with brand:
        uri = _icon_data_uri()
        img = f'<img src="{uri}" alt="">' if uri else ""
        st.markdown(f'<div class="sfts-brand">{img}<b>{L("app.title")}</b></div>', unsafe_allow_html=True)
    with tab_t:
        if st.button(L("nav.translate"), key="nav_translate"):
            _go("translate")
    with tab_s:
        if st.button(L("nav.settings"), key="nav_settings"):
            _go("settings")
    dark = st.session_state.theme == "dark"
    with theme:
        if st.button(
            L("theme.light") if dark else L("theme.dark"),
            key="theme_toggle",
            help=L("theme.light") if dark else L("theme.dark"),
        ):
            _set_theme("light" if dark else "dark")


# ── settings ────────────────────────────────────────────────────────────────


def render_appearance_pane() -> None:
    langs = available_languages()
    detected = detect_ui_language(_browser_accept_language())
    if detected not in langs:
        detected = FALLBACK_LANG if FALLBACK_LANG in langs else langs[0]
    now_name = language_display_name(detected, st.session_state.ui_lang)
    st.markdown(f'<div class="sfts-pane-title">{L("card.appearance")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_appearance"):
        lab, ctl = st.columns([1.1, 2.2], vertical_alignment="center")
        with lab:
            st.markdown(f'<div class="sfts-row-label">{L("card.theme")}</div>', unsafe_allow_html=True)
        with ctl:
            theme = st.segmented_control(
                L("card.theme"),
                options=["light", "dark"],
                default=st.session_state.theme,
                format_func={"light": L("theme.light"), "dark": L("theme.dark")}.get,
                key="theme_seg",
                label_visibility="collapsed",
            )
        if theme in {"light", "dark"} and theme != st.session_state.theme:
            _set_theme(theme)
        st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)
        lang_labels = {code: language_display_name(code, st.session_state.ui_lang) for code in langs}
        options = ["__system__"] + langs
        current = "__system__" if st.session_state.get("ui_lang_follow") else (
            st.session_state.ui_lang if st.session_state.ui_lang in langs else "__system__"
        )
        chosen = st.selectbox(
            L("sidebar.language"),
            options=options,
            index=options.index(current) if current in options else 0,
            format_func={**lang_labels, "__system__": L("lang.follow_system", name=now_name)}.get,
            key="ui_lang_select",
        )
        if chosen == "__system__":
            if not st.session_state.get("ui_lang_follow") or st.session_state.ui_lang != detected:
                _set_lang(detected, follow=True)
        elif chosen != st.session_state.ui_lang or st.session_state.get("ui_lang_follow"):
            _set_lang(chosen, follow=False)
        st.markdown(f'<div class="sfts-muted">{L("card.lang_fallback")}</div>', unsafe_allow_html=True)
        st.markdown(
            f'<div class="sfts-lang-count">{wrap(GLOBE)}{L("card.lang_count")}</div>',
            unsafe_allow_html=True,
        )


def _provider_labels() -> dict[str, str]:
    labels = {p: L(f"sidebar.provider_{p}") for p in PROVIDER_OPTIONS}
    labels["auto"] = L("sidebar.provider_auto")
    if "demo" in labels:
        labels["demo"] = L("sidebar.provider_demo")
    return labels


def render_translation_pane() -> None:
    st.markdown(f'<div class="sfts-pane-title">{L("card.translation")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_translation"):
        provider_labels = _provider_labels()
        chosen_provider = st.selectbox(
            L("sidebar.provider"),
            options=PROVIDER_OPTIONS,
            index=PROVIDER_OPTIONS.index(st.session_state.provider) if st.session_state.provider in PROVIDER_OPTIONS else 0,
            format_func=lambda x: provider_labels.get(x, x),
            key="provider_select",
        )
        if chosen_provider != st.session_state.provider:
            st.session_state.provider = chosen_provider
            stored = (st.session_state.model_by_provider or {}).get(chosen_provider)
            st.session_state.model = resolve_model(chosen_provider, stored) or default_model(chosen_provider)
            _persist_prefs()
            st.rerun()
        model_options = models_for(st.session_state.provider)
        if not model_options:
            st.caption(L("sidebar.model_auto"))
        else:
            current_model = resolve_model(st.session_state.provider, st.session_state.get("model"))
            if current_model not in model_options:
                current_model = model_options[0]
            picked = st.selectbox(
                L("sidebar.model"),
                options=model_options,
                index=model_options.index(current_model),
                key="model_select",
            )
            if picked != st.session_state.model:
                st.session_state.model = picked
                models = dict(st.session_state.model_by_provider or {})
                models[st.session_state.provider] = picked
                st.session_state.model_by_provider = models
                _persist_prefs()
                st.rerun()
        conc = st.slider(
            L("sidebar.concurrency"),
            min_value=MIN_CONCURRENCY,
            max_value=MAX_CONCURRENCY,
            value=clamp_concurrency(st.session_state.concurrency),
            key="concurrency_slider",
        )
        if conc != st.session_state.concurrency:
            st.session_state.concurrency = conc
            _persist_prefs()
            st.rerun()
        st.markdown(f'<div class="sfts-muted">{L("sidebar.concurrency_hint")}</div>', unsafe_allow_html=True)
        st.markdown(f'<div class="sfts-muted">{L("quick.where")}</div>', unsafe_allow_html=True)


def _key_label(fallback: str, locale_key: str) -> str:
    text = L(locale_key)
    return fallback if text == locale_key else text


def _saved_key(env_name: str, value: str) -> None:
    save_secret_to_env(env_name, value)
    list_available_providers(fresh=True)
    _probe.clear()
    _toast(L("sidebar.save_key_ok"))
    st.rerun()


def _key_head(label: str, on: bool, status: str) -> None:
    pill = (
        f"<span class='sfts-pill-on'>{wrap(CHECK)}{html.escape(status)}</span>"
        if on
        else f"<span class='sfts-pill-off'>{wrap(DASH)}{html.escape(status)}</span>"
    )
    st.markdown(f'<div class="sfts-key-name">{html.escape(label)}{pill}</div>', unsafe_allow_html=True)


def _field_and_save(label: str, key: str, save_key: str, **field) -> str | None:
    """A field and its Save button on one row; returns the stripped value when Save is clicked."""
    c1, c2 = st.columns([3, 1.25], vertical_alignment="bottom")
    with c1:
        val = st.text_input(label, key=key, label_visibility="collapsed", **field)
    with c2:
        clicked = st.button(L("keys.save_local"), key=save_key, use_container_width=True)
    return val.strip() if clicked and val.strip() else None


def _cli_block(which: str, label: str, hint_url: str, path_setting: str, prefix: str) -> None:
    status = _probe(which)
    _key_head(label, status.usable, L("keys.connected_cli") if status.usable else L("keys.unset"))
    if not status.usable:
        st.markdown(
            f'<div class="sfts-muted">{L(f"keys.{prefix}_login") if status.hint == "login" else L(f"keys.{prefix}_missing", url=hint_url)}</div>',
            unsafe_allow_html=True,
        )
    saved = _field_and_save(
        L(f"keys.{prefix}_path"), f"{prefix}_path_input", f"save_{prefix}_path",
        value=path_setting, placeholder=L(f"keys.{prefix}_path"),
    )
    if saved:
        _saved_key("GROK_CLI_PATH" if which == "grok" else "CODEX_CLI_PATH", saved)
    st.markdown(f'<div class="sfts-muted">{L(f"keys.{prefix}_hint")}</div>', unsafe_allow_html=True)
    st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)


def render_keys_pane() -> None:
    st.markdown(f'<div class="sfts-pane-title">{L("card.keys")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_keys"):
        for fallback, env_name, locale_key in KEY_ROWS:
            label = _key_label(fallback, locale_key)
            val = load_secret(env_name)
            if val:
                tail = val[-4:] if len(val) >= 4 else ""
                _key_head(label, True, L("keys.connected", tail=tail) if tail else L("keys.set"))
            else:
                _key_head(label, False, L("keys.unset"))
                saved = _field_and_save(
                    L("keys.paste_api"), f"paste_{env_name}", f"save_{env_name}", type="password", placeholder=env_name,
                )
                if saved:
                    _saved_key(env_name, saved)
        st.markdown(f'<div class="sfts-muted">{L("keys.xai_hint")}</div>', unsafe_allow_html=True)
        st.markdown('<hr class="sfts-divider">', unsafe_allow_html=True)
        _cli_block("grok", _key_label("Official Grok CLI", "keys.grok_cli"), GROK_HINT, grok_cli_path_setting(), "grok_cli")
        _cli_block("codex", _key_label("Official Codex CLI", "keys.codex_cli"), CODEX_HINT, codex_cli_path_setting(), "codex_cli")
        st.markdown(
            f'<div class="sfts-muted">{L("keys.local_only")} {L("sidebar.connect_official_only")} '
            f'{L("sidebar.connect_no_websites")}</div>',
            unsafe_allow_html=True,
        )


def render_glossary_pane() -> None:
    st.markdown(f'<div class="sfts-pane-title">{L("card.glossary")}</div>', unsafe_allow_html=True)
    with st.container(border=True, key="card_glossary"):
        projects = list_projects()
        if "default" not in projects:
            ensure_project("default")
            projects = list_projects()
        p_idx = projects.index(st.session_state.project) if st.session_state.project in projects else 0
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
            st.caption(L("glossary.empty"))
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
                if st.button(L("glossary.delete"), key=f"gdel_{nonce}_{i}", help=L("glossary.delete")):  # ICON: CLOSE
                    drop = i
            current.append((nt, ntr))
        if drop is not None:
            st.session_state.glossary_pairs = [p for j, p in enumerate(current) if j != drop]
            st.session_state.glossary_nonce += 1
            st.rerun()
        b1, b2 = st.columns(2)
        with b1:
            if st.button(L("sidebar.add_term"), key="glossary_add", use_container_width=True):
                st.session_state.glossary_pairs = current + [("", "")]
                st.session_state.glossary_nonce += 1
                st.rerun()
        with b2:
            if st.button(L("sidebar.save_glossary"), type="primary", key="glossary_save", use_container_width=True):
                edited = [(a.strip(), b.strip()) for a, b in current if a.strip()]
                save_glossary(st.session_state.project, edited)
                st.session_state.glossary_pairs = edited
                st.session_state.glossary_nonce += 1
                _toast(L("glossary.saved", name=st.session_state.project))
                st.rerun()


def render_settings() -> None:
    rail, pane = st.columns([1, 3.2], gap="large")
    labels = {
        "appearance": L("card.appearance"),
        "translation": L("card.translation"),
        "keys": L("card.keys"),
        "glossary": L("card.glossary"),
    }
    with rail:
        st.markdown('<div style="height:40px"></div>', unsafe_allow_html=True)  # title 28 + gap 16 - rail gap 4: rail meets card top
        for pane_id in SETTINGS_PANES:
            if st.button(labels[pane_id], use_container_width=True, key=f"pane_{pane_id}"):
                _go("settings", pane_id)
        if st.button(L("update.button"), key="update_settings"):
            _check_update()
    with pane:
        current = st.session_state.settings_pane
        if current == "appearance":
            render_appearance_pane()
        elif current == "translation":
            render_translation_pane()
        elif current == "keys":
            render_keys_pane()
        else:
            render_glossary_pane()


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


def _swap_langs() -> None:
    src, dst = st.session_state.source_choice, st.session_state.target_lang
    if src == "auto" or dst == "other":
        return
    st.session_state.source_choice, st.session_state.target_lang = dst, src
    st.session_state["qb_source"], st.session_state["qb_target"] = dst, src
    _persist_prefs()


def _chip_text(available: list[str]) -> tuple[str, bool]:
    provider = st.session_state.provider
    if not available:
        return L("quick.no_translator"), True
    if provider == "auto":
        return f'{L("quick.auto")} · {PROVIDER_SHORT.get(available[0], available[0])}', False
    if provider not in available:
        return f'{PROVIDER_SHORT.get(provider, provider)} · {L("keys.unset")}', True
    model = resolve_model(provider, st.session_state.get("model"))
    name = PROVIDER_SHORT.get(provider, provider)
    return (f"{name} · {model}" if model and model != "demo" else name), False


def render_quick_bar() -> None:
    """From / swap / To / translator, as the header row of the translate card."""
    available = list_available_providers()
    with st.container(key="quickbar"):
        c_from, c_swap, c_to, c_chip = st.columns([2.2, 0.42, 2.2, 2.3], vertical_alignment="bottom")
        src_opts = ["auto"] + [c for c in TARGET_CODES if c != "other"]
        with c_from:
            st.selectbox(
                L("quick.from"),
                options=src_opts,
                format_func={c: _lang_label(c) for c in src_opts}.get,
                **_bound("qb_source", "source_choice"),
            )
        with c_swap:
            can_swap = st.session_state.source_choice != "auto" and st.session_state.target_lang != "other"
            st.button(
                L("quick.swap"),
                key="swap_langs",
                on_click=_swap_langs,
                disabled=not can_swap,
                help=L("quick.swap") if can_swap else L("quick.swap_auto"),
            )
        with c_to:
            st.selectbox(
                L("quick.to"),
                options=TARGET_CODES,
                format_func={c: L(f"target.{c}") for c in TARGET_CODES}.get,
                **_bound("qb_target", "target_lang"),
            )
        with c_chip:
            st.markdown(f'<div class="sfts-flabel">{L("quick.translator")}</div>', unsafe_allow_html=True)
            text, warn = _chip_text(available)
            if st.button(text, key="provider_chip_warn" if warn else "provider_chip", help=L("quick.change"), use_container_width=True):
                _go("settings", "keys" if warn else "translation")
        if st.session_state.target_lang == "other":
            st.text_input(L("sidebar.target_other"), placeholder="e.g. it, nl, pl", **_bound("qb_other", "target_other"))


def _job_kwargs() -> dict:
    return dict(
        target_lang=_target_lang(),
        source_lang=_source_lang(),
        project=st.session_state.project,
        provider_choice=st.session_state.provider,
        game_mode=st.session_state.content_mode == "game",
        model=resolve_model(st.session_state.provider, st.session_state.get("model")),
        concurrency=clamp_concurrency(st.session_state.concurrency),
    )


def _ready() -> bool:
    if list_available_providers():
        return True
    _toast(L("main.status_no_key"), "error")
    return False


def _cancel_clicked() -> None:
    _toast(L("run.cancelled"), "warn")


def _unique_out(name: str, suffix: str) -> Path:
    out = outputs_dir() / (Path(name).stem + f".{_target_lang()}" + suffix)
    if out.exists():
        out = out.with_name(out.stem + time.strftime(".%Y%m%d-%H%M%S") + out.suffix)
    return out


def _preview_text(raw: bytes) -> str:
    if b"\x00" in raw[:2048]:
        return L("main.binary_preview")
    return raw[:16000].decode("utf-8", errors="replace")[:4000]


def _run_header():
    """Status line with Cancel on the right, then a thin bar. Returns (head, bar)."""
    line, stop = st.columns([6, 1], vertical_alignment="center")
    with line:
        head = st.empty()
    with stop:
        st.button(L("run.cancel"), key="cancel_run", on_click=_cancel_clicked, use_container_width=True)
    return head, st.progress(0.0)


def _run_line(head, text: str, right: str = "") -> None:
    head.markdown(
        f'<div class="sfts-run"><i></i>{html.escape(text)}<span>{html.escape(right)}</span></div>',
        unsafe_allow_html=True,
    )


def _run_single() -> None:
    name = st.session_state.picked_name
    raw = st.session_state.picked_bytes or b""
    suffix = Path(name).suffix.lower()
    out_path = _unique_out(name, suffix)
    with st.container(border=True, key="card_run"):
        head, bar = _run_header()
        _run_line(head, L("run.file", name=name), L("run.preparing"))

        def progress(done: int, total: int, _item) -> None:
            if total:
                _run_line(head, L("run.file", name=name), L("run.chunks", done=done, total=total))
                bar.progress(min(1.0, done / total))

        try:
            with tempfile.TemporaryDirectory(prefix="versora_") as tmp:
                src = Path(tmp) / f"source{suffix}"
                src.write_bytes(raw)
                translate_single_file(src, out_path, on_progress=progress, **_job_kwargs())
        except Exception as e:
            _toast(L("main.status_error", msg=_human(str(e))), "error")
            st.rerun()
    binary = out_path.suffix.lower() in {".docx", ".pdf", ".xlsx"}
    st.session_state.result = {
        "src": _preview_text(raw),
        "out": L("main.saved_binary") if binary else out_path.read_text(encoding="utf-8", errors="replace")[:4000],
        "path": str(out_path),
    }
    _toast(L("main.status_done"))
    st.rerun()


def _files_html(report: BatchReport, workers: int) -> str:
    done = {i.rel for i in report.written}
    failed = {i.rel: i for i in report.failed}
    pending = [r for r in report.planned if r not in done and r not in failed]
    running = set(pending[:workers])
    rows = []
    for rel in report.planned:
        if rel in done:
            s, note = "done", L("state.done")
        elif rel in failed:
            s, note = "fail", _human(failed[rel].error)
        elif rel in running:
            s, note = "run", L("state.running")
        else:
            s, note = "wait", L("state.waiting")
        rows.append(f'<div class="sfts-file" data-s="{s}"><b>{html.escape(rel)}</b><span>{html.escape(note)}</span></div>')
    return '<div class="sfts-files">' + "".join(rows) + "</div>"


def _run_batch(job: dict, only: set[str] | None = None) -> None:
    """Run a folder or zip job with live per-file rows. ``only`` = retry those files."""
    report = BatchReport()
    previous = st.session_state.batch_report if only else None
    if not only:
        st.session_state.batch_report = report
        st.session_state.batch_job = job
    st.session_state.batch_zip = None
    workers = clamp_concurrency(st.session_state.concurrency)
    with st.container(border=True, key="card_run"):
        head, bar = _run_header()
        _run_line(head, L("run.preparing"))
        rows = st.empty()

        def progress(done: int, total: int, _item) -> None:
            _run_line(head, L("run.batch", total=total), L("run.files", done=done, total=total))
            bar.progress(min(1.0, done / total) if total else 0.0)
            rows.markdown(_files_html(report, workers), unsafe_allow_html=True)

        try:
            kw = dict(_job_kwargs(), report=report, on_progress=progress, only=only)
            if job["kind"] == "folder":
                root = Path(job["path"])
                translate_tree(root, job_name=root.name, **kw)
            else:
                with tempfile.TemporaryDirectory(prefix="versora_zip_") as tmp:
                    zpath = Path(tmp) / "upload.zip"
                    zpath.write_bytes(job["bytes"])
                    translate_zip(zpath, Path(tmp) / "tree", job_name=job["name"], **kw)
        except Exception as e:
            _toast(L("main.status_error", msg=_human(str(e))), "error")
            if not only:
                st.session_state.batch_report = None
            st.rerun()
    if previous is not None:  # merge a retry into the shown report
        previous.written += report.written
        previous.failed = [f for f in previous.failed if f.rel not in only] + report.failed
        previous.cancelled = report.cancelled
        report = previous
        st.session_state.batch_report = report
    n_ok, n_fail = len(report.written), len(report.failed)
    _toast(
        L("main.batch_done", n=n_ok, k=len(report.skipped)) if not n_fail else L("done.batch_failed", n=n_ok, f=n_fail),
        "ok" if not n_fail else "warn",
    )
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


def _done_header(title: str, sub: str, warn: bool = False) -> None:
    st.markdown(
        f'<div class="sfts-done" data-tone="{"warn" if warn else "ok"}">'
        '<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3.5 10.5l4 4 9-9.5"/></svg>'
        f'<div><div class="sfts-done-title">{html.escape(title)}</div>'
        f'<div class="sfts-done-sub">{sub}</div></div></div>',
        unsafe_allow_html=True,
    )


def render_single_result() -> None:
    res = st.session_state.result
    path = Path(res["path"])
    with st.container(border=True, key="card_result"):
        top, act = st.columns([3, 1.4], vertical_alignment="center")
        with top:
            _done_header(L("done.file"), f'<code class="sfts-path">{html.escape(str(path))}</code>')
        with act:
            if path.is_file():
                st.download_button(
                    L("main.download"),
                    data=path.read_bytes(),
                    file_name=path.name,
                    mime="application/octet-stream",
                    key="dl_single",
                    type="primary",
                    use_container_width=True,
                )
        c1, c2 = st.columns(2)
        with c1:
            st.markdown(f'<div class="sfts-panel-title">{L("main.preview_src")}</div>', unsafe_allow_html=True)
            st.text_area("src", value=res["src"], height=240, label_visibility="collapsed", disabled=True)
        with c2:
            st.markdown(f'<div class="sfts-panel-title">{L("main.preview_out")}</div>', unsafe_allow_html=True)
            st.text_area("out", value=res["out"], height=240, label_visibility="collapsed", disabled=True)


def render_batch_result() -> None:
    report: BatchReport = st.session_state.batch_report
    job = st.session_state.batch_job
    n_ok, n_fail, n_skip = len(report.written), len(report.failed), len(report.skipped)
    with st.container(border=True, key="card_result"):
        parts = [L("done.n_saved", n=n_ok)]
        if n_fail:
            parts.append(L("done.n_failed", n=n_fail))
        if n_skip:
            parts.append(L("done.n_skipped", n=n_skip))
        title = L("run.cancelled") if report.cancelled else (L("done.batch") if not n_fail else L("done.batch_some"))
        _done_header(title, " · ".join(parts) + f'<br><code class="sfts-path">{html.escape(report.output_root)}</code>', warn=bool(n_fail))
        b1, b2, b3 = st.columns(3)
        with b1:
            if n_ok:
                if st.session_state.batch_zip is None:
                    st.session_state.batch_zip = _zip_outputs(report)
                st.download_button(
                    L("batch.download_all"),
                    data=st.session_state.batch_zip,
                    file_name=Path(report.output_root).name + ".zip",
                    mime="application/zip",
                    key="dl_all",
                    type="primary",
                    use_container_width=True,
                )
        with b2:
            if report.output_root and Path(report.output_root).is_dir():
                if st.button(L("batch.open_folder"), key="open_out", use_container_width=True):
                    _open_folder(report.output_root)
        with b3:
            if n_fail and job:
                if st.button(L("batch.retry", n=n_fail), key="retry_failed", use_container_width=True):
                    _run_batch(job, only={f.rel for f in report.failed})

        def rows(items, state: str, note) -> str:
            return '<div class="sfts-files">' + "".join(
                f'<div class="sfts-file" data-s="{state}"><b>{html.escape(i.rel)}</b><span>{html.escape(note(i))}</span></div>'
                for i in items
            ) + "</div>"

        if n_fail:
            st.markdown(f'<div class="sfts-panel-title sfts-section">{L("batch.failed")} · {n_fail}</div>', unsafe_allow_html=True)
            st.markdown(rows(report.failed, "fail", lambda i: _human(i.error)), unsafe_allow_html=True)
        if n_ok:
            st.markdown(f'<div class="sfts-panel-title sfts-section">{L("batch.saved")} · {n_ok}</div>', unsafe_allow_html=True)
            st.markdown(rows(sorted(report.written, key=lambda i: i.rel), "done", lambda i: L("state.done")), unsafe_allow_html=True)
        if n_skip:
            with st.expander(f'{L("batch.skipped")} · {n_skip}'):
                st.markdown(rows(report.skipped, "skip", lambda i: _human(i.skipped or i.error, _SKIP_HINTS)), unsafe_allow_html=True)


def _show_file_chip() -> bool:
    """Picked file row: name, size, remove, and the start button. Returns True on start."""
    name = st.session_state.picked_name
    chip, clear, start = st.columns([7, 0.5, 2], vertical_alignment="center")
    with chip:
        st.markdown(
            f'<div class="sfts-filechip">{wrap(FILE)}'
            f'<span class="sfts-filechip-name">{html.escape(name)}</span>'
            f'<span class="sfts-filechip-size">{_fmt_size(int(st.session_state.picked_size or 0))}</span></div>',
            unsafe_allow_html=True,
        )
    with clear:
        if st.button(L("main.clear"), key="clear_picked", help=L("main.clear")):  # ICON: CLOSE (drawn by theme.py)
            _forget_pick()
            _clear_results()
            st.rerun()
    suffix = Path(name).suffix.lower()
    problem = None
    if st.session_state.source_type == "file" and suffix == ".zip":
        problem = L("main.zip_use_zip_mode")
    elif st.session_state.source_type == "file" and not is_supported(name) and suffix not in SCRIPT_SUFFIXES:
        problem = L("error.unsupported_format")
    with start:
        go = st.button(L("main.translate_btn"), type="primary", use_container_width=True, key="start_translate", disabled=bool(problem))
    if problem:
        st.markdown(f'<div class="sfts-warn">{problem}</div>', unsafe_allow_html=True)
    return go


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
    """One card: language row, then source tabs, then the drop zone as the page's hero."""
    with st.container(border=True, key="card_main"):
        render_quick_bar()
        st.markdown('<hr class="sfts-rule">', unsafe_allow_html=True)
        c1, c2 = st.columns([1.3, 1], vertical_alignment="center")
        with c1:
            st.segmented_control(
                L("main.source_type"),
                options=["file", "folder", "zip"],
                format_func={"file": L("main.seg_file"), "folder": L("main.seg_folder"), "zip": L("main.seg_zip")}.get,
                required=True,
                label_visibility="collapsed",
                **_bound("source_type_seg", "source_type"),
            )
        with c2:
            st.segmented_control(
                L("main.content_mode"),
                options=["document", "game"],
                format_func={"document": L("main.seg_doc"), "game": L("main.seg_game")}.get,
                required=True,
                label_visibility="collapsed",
                help=L("main.mode_help"),
                **_bound("content_mode_seg", "content_mode"),
            )
        kind = st.session_state.source_type
        go = False
        with st.container(key="sourcebody"):
            if kind in {"file", "zip"}:
                name = st.session_state.picked_name
                if name and kind == "zip" and not name.lower().endswith(".zip"):
                    _forget_pick()
                    name = None
                if not name:
                    _pick_upload(kind)
                else:
                    go = _show_file_chip()
            else:
                st.markdown(f'<div class="sfts-note">{L("main.folder_hint")}</div>', unsafe_allow_html=True)
                path_col, start = st.columns([7.5, 2], vertical_alignment="center")
                with path_col:
                    folder = st.text_input(
                        L("main.folder_path"),
                        label_visibility="collapsed",
                        placeholder=L("main.folder_placeholder"),
                        **_bound("folder_path_in", "folder_path"),
                    )
                with start:
                    go = st.button(
                        L("main.translate_btn"), type="primary", use_container_width=True,
                        key="start_translate", disabled=not folder.strip(),
                    )

    if go and _ready():
        _clear_results()
        if kind == "file":
            _run_single()
        elif kind == "zip":
            _run_batch({"kind": "zip", "bytes": st.session_state.picked_bytes, "name": Path(st.session_state.picked_name).stem})
        else:
            root = Path(st.session_state.folder_path.strip()).expanduser()
            if not root.is_dir():
                _toast(L("main.folder_missing"), "error")
                st.rerun()
            _run_batch({"kind": "folder", "path": str(root)})
    elif go:
        st.rerun()

    if kind == "file" and st.session_state.result:
        render_single_result()
    if kind in {"folder", "zip"} and st.session_state.batch_report is not None:
        render_batch_result()


render_chrome()
_sync_query()
_drain_toasts()
if st.session_state.page == "settings":
    render_settings()
else:
    render_translate()

st.markdown(f'<div class="sfts-footer">{L("about.footer")}</div>', unsafe_allow_html=True)
