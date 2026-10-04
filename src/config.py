"""Runtime configuration from environment variables. No secrets are hard-coded."""

from __future__ import annotations

import os
import time
from pathlib import Path
from dataclasses import dataclass
from typing import Optional

from dotenv import load_dotenv

from .security.hosts import HostNotAllowed, assert_public_https_url
from .security.secrets import load_secret

# Load .env from project root if present
_ROOT = Path(__file__).resolve().parent.parent
load_dotenv(_ROOT / ".env")
# Never read NEXT_PUBLIC_* — those names are for browsers and are rejected.


@dataclass(frozen=True)
class ProviderConfig:
    name: str
    api_key: Optional[str]
    base_url: Optional[str]
    model: str
    available: bool


def _get(key: str, default: str = "") -> str:
    if key.startswith("NEXT_PUBLIC_"):
        return default
    if key.endswith("_API_KEY") or key.endswith("_TOKEN"):
        return load_secret(key) or default
    return (os.getenv(key) or default).strip()


def safe_openai_base_url() -> str:
    raw = _get("OPENAI_BASE_URL", "https://api.openai.com/v1") or "https://api.openai.com/v1"
    assert_public_https_url(raw if "://" in raw else f"https://{raw}")
    return raw


def get_openai_config() -> ProviderConfig:
    key = _get("OPENAI_API_KEY")
    raw = _get("OPENAI_BASE_URL", "https://api.openai.com/v1") or "https://api.openai.com/v1"
    base = None
    if key:
        try:
            base = safe_openai_base_url()
        except HostNotAllowed:
            base = None
    else:
        base = raw
    return ProviderConfig(
        name="openai",
        api_key=key or None,
        base_url=base,
        model=_get("OPENAI_MODEL", "gpt-4.1-mini") or "gpt-4.1-mini",
        available=bool(key) and bool(base),
    )


def get_anthropic_config() -> ProviderConfig:
    key = _get("ANTHROPIC_API_KEY")
    return ProviderConfig(
        name="anthropic",
        api_key=key or None,
        base_url="https://api.anthropic.com",
        model=_get("ANTHROPIC_MODEL", "claude-haiku-4-5") or "claude-haiku-4-5",
        available=bool(key),
    )


def get_gemini_config() -> ProviderConfig:
    key = _get("GEMINI_API_KEY")
    return ProviderConfig(
        name="gemini",
        api_key=key or None,
        base_url="https://generativelanguage.googleapis.com",
        model=_get("GEMINI_MODEL", "gemini-2.5-flash") or "gemini-2.5-flash",
        available=bool(key),
    )


def get_xai_config() -> ProviderConfig:
    """Official xAI API key only. Not a grok.com login and not the Grok CLI."""
    key = _get("XAI_API_KEY")
    return ProviderConfig(
        name="xai",
        api_key=key or None,
        base_url="https://api.x.ai/v1",
        model=_get("XAI_MODEL", "grok-3-mini") or "grok-3-mini",
        available=bool(key),
    )


_AVAILABLE_TTL = 60.0
_available_cache: tuple[float, list[str]] | None = None


def list_available_providers(*, fresh: bool = False) -> list[str]:
    """Cached for 60 s: the CLI probes spawn processes (8 s timeout each) and the
    router asks once per chunk. ``fresh=True`` after a key or CLI path is saved."""
    global _available_cache
    now = time.monotonic()
    if not fresh and _available_cache and now - _available_cache[0] < _AVAILABLE_TTL:
        return list(_available_cache[1])
    names = _probe_available()
    _available_cache = (now, names)
    return list(names)


def _probe_available() -> list[str]:
    names = []
    if get_openai_config().available:
        names.append("openai")
    if get_anthropic_config().available:
        names.append("anthropic")
    if get_gemini_config().available:
        names.append("gemini")
    if get_xai_config().available:
        names.append("xai")
    from .providers.cli import PRESETS, probe

    names += [pid for pid in PRESETS if probe(pid).usable]  # cached per STATUS_TTL, never per chunk
    from .providers.demo import demo_enabled

    if demo_enabled():
        names.append("demo")
    return names


def get_default_provider() -> str:
    val = _get("DEFAULT_PROVIDER", "auto").lower()
    if val in ("auto", "openai", "anthropic", "gemini", "xai", "claude_cli", "grok_cli", "codex_cli"):
        return val
    return "auto"


def get_chunk_size() -> int:
    try:
        return max(500, int(_get("TRANSLATE_CHUNK_SIZE", "3000") or "3000"))
    except ValueError:
        return 3000


def project_root() -> Path:
    return _ROOT


def projects_dir() -> Path:
    d = _ROOT / "projects"
    d.mkdir(parents=True, exist_ok=True)
    return d


def outputs_dir() -> Path:
    d = _ROOT / "data" / "outputs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def locales_dir() -> Path:
    return _ROOT / "locales"
