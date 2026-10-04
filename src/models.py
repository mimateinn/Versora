"""Provider-matched model catalogs. Local lists only — no website login."""

from __future__ import annotations

from .config import (
    get_anthropic_config,
    get_gemini_config,
    get_openai_config,
    get_xai_config,
)

# Official API / CLI model ids shown in Settings. Keep each list on its provider.
_MODELS: dict[str, list[str]] = {
    "openai": ["gpt-4o-mini", "gpt-4o", "gpt-4.1-mini", "gpt-4.1"],
    "anthropic": [
        "claude-3-5-haiku-20241022",
        "claude-3-5-sonnet-20241022",
        "claude-sonnet-4-20250514",
    ],
    "gemini": ["gemini-1.5-flash", "gemini-1.5-pro", "gemini-2.0-flash"],
    "xai": ["grok-3-mini", "grok-3", "grok-2-latest"],
    "grok_cli": ["grok-3-mini", "grok-3", "grok-4", "grok-2-latest"],
    "codex_cli": ["gpt-5.1-codex", "gpt-5", "gpt-5-mini", "o4-mini", "o3"],
    "demo": ["demo"],
}


def models_for(provider: str) -> list[str]:
    choice = (provider or "auto").lower().strip()
    if choice == "auto":
        return []
    names = list(_MODELS.get(choice) or [])
    env_default = default_model(choice)
    if env_default and env_default not in names:
        names.insert(0, env_default)
    return names


def default_model(provider: str) -> str:
    choice = (provider or "").lower().strip()
    if choice == "openai":
        return get_openai_config().model
    if choice == "anthropic":
        return get_anthropic_config().model
    if choice == "gemini":
        return get_gemini_config().model
    if choice == "xai":
        return get_xai_config().model
    catalog = _MODELS.get(choice) or []
    return catalog[0] if catalog else ""


def resolve_model(provider: str, chosen: str | None) -> str | None:
    choice = (provider or "auto").lower().strip()
    if choice == "auto":
        return None
    allowed = models_for(choice)
    name = (chosen or "").strip()
    if name and name in allowed:
        return name
    return default_model(choice) or None
