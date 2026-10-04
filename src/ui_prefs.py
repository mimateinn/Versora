"""Local UI prefs. Never stores secrets."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

from .batch import DEFAULT_CONCURRENCY, clamp_concurrency
from .config import projects_dir

_PREFS_NAME = ".sfts-ui.json"
_THEMES = {"light", "dark"}
_PROVIDERS = {
    "auto",
    "openai",
    "anthropic",
    "gemini",
    "xai",
    "claude_cli",
    "grok_cli",
    "codex_cli",
    "demo",
}
_PURPOSES = {"general", "technical", "ui", "subtitles", "game", "legal", "academic", "business", "custom"}
_SAFE_ID = re.compile(r"^[A-Za-z0-9._:/-]{0,64}$")  # model / effort ids (they may reach a CLI's argv)


def _clean_chain(rows: Any) -> list[dict[str, Any]]:
    """The ordered translator chain: known ids once each, safe model / effort ids, an on/off flag."""
    out, seen = [], set()
    for row in rows if isinstance(rows, list) else []:
        if not isinstance(row, dict):
            continue
        pid = str(row.get("id") or "")
        if pid not in _PROVIDERS or pid in seen:
            continue
        model, effort = str(row.get("model") or "").strip(), str(row.get("effort") or "").strip()
        seen.add(pid)
        out.append({
            "id": pid,
            "model": model if _SAFE_ID.match(model) else "",
            "effort": effort if effort in {"", "low", "medium", "high"} else "",
            "enabled": bool(row.get("enabled", True)),
        })
    return out
# Last-used translate choices. Codes are short tags ("en", "zh-Hant", "auto", "other");
# the free-text "other" target is a language code or name typed by the user.
_CHOICES = {
    "source_type": {"file", "folder", "zip"},
    "content_mode": {"document", "game"},
}
_CODE_RE = re.compile(r"^[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*$")


def _clamp_per(value: Any) -> int:
    try:
        return max(1, min(8, int(value)))
    except (TypeError, ValueError):
        return 1


def prefs_path() -> Path:
    return projects_dir() / _PREFS_NAME


def load_prefs() -> dict[str, Any]:
    path = prefs_path()
    if not path.is_file():
        return {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(data, dict):
        return {}
    out: dict[str, Any] = {}
    theme = str(data.get("theme") or "").strip()
    if theme in _THEMES:
        out["theme"] = theme
    lang = str(data.get("ui_lang") or "").strip()
    if lang:
        out["ui_lang"] = lang
    provider = str(data.get("provider") or "").strip()
    if provider in _PROVIDERS:
        out["provider"] = provider
    models = data.get("model_by_provider")
    if isinstance(models, dict):
        cleaned = {str(k): str(v) for k, v in models.items() if k and v}
        if cleaned:
            out["model_by_provider"] = cleaned
    if "concurrency" in data:
        out["concurrency"] = clamp_concurrency(data.get("concurrency"))
    if "per_provider" in data:
        out["per_provider"] = _clamp_per(data.get("per_provider"))
    if "chain" in data:
        out["chain"] = _clean_chain(data.get("chain"))
    if data.get("purpose") in _PURPOSES:
        out["purpose"] = data["purpose"]
    if "ui_lang_follow" in data:
        out["ui_lang_follow"] = bool(data.get("ui_lang_follow"))
    for key, allowed in _CHOICES.items():
        val = str(data.get(key) or "").strip()
        if val in allowed:
            out[key] = val
    for key in ("target_lang", "source_choice"):
        val = str(data.get(key) or "").strip()
        if _CODE_RE.match(val):
            out[key] = val
    other = str(data.get("target_other") or "").strip()[:40]
    if other:
        out["target_other"] = other
    return out


def save_prefs(**values: Any) -> None:
    """Merge known keys into the prefs file; unknown or invalid values are dropped."""
    current = load_prefs()
    theme = values.get("theme")
    if theme in _THEMES:
        current["theme"] = theme
    if values.get("ui_lang"):
        current["ui_lang"] = values["ui_lang"]
    if values.get("ui_lang_follow") is not None:
        current["ui_lang_follow"] = bool(values["ui_lang_follow"])
    if values.get("provider") in _PROVIDERS:
        current["provider"] = values["provider"]
    models = values.get("model_by_provider")
    if models is not None:
        current["model_by_provider"] = {str(k): str(v) for k, v in models.items() if k and v}
    if values.get("concurrency") is not None:
        current["concurrency"] = clamp_concurrency(values["concurrency"])
    if values.get("per_provider") is not None:
        current["per_provider"] = _clamp_per(values["per_provider"])
    if values.get("chain") is not None:
        current["chain"] = _clean_chain(values["chain"])
    if values.get("purpose") in _PURPOSES:
        current["purpose"] = values["purpose"]
    for key, allowed in _CHOICES.items():
        if values.get(key) in allowed:
            current[key] = values[key]
    for key in ("target_lang", "source_choice"):
        val = str(values.get(key) or "")
        if _CODE_RE.match(val):
            current[key] = val
    if values.get("target_other") is not None:
        current["target_other"] = str(values["target_other"]).strip()[:40]
    path = prefs_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(current, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
