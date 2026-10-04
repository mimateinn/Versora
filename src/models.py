"""Model suggestions per translator. Suggestions only: any id the provider accepts can be typed in.

Lists date quickly, so none of them is a default for a CLI: a blank model drops the flag and the
CLI's own configured default is used. API defaults come from the *_MODEL env settings.
"""

from __future__ import annotations

import re

from .config import (
    get_anthropic_config,
    get_gemini_config,
    get_openai_config,
    get_xai_config,
)

MODEL_ID = re.compile(r"^[A-Za-z0-9._:/-]{1,64}$")

_SUGGEST: dict[str, list[str]] = {
    "openai": ["gpt-4.1-mini", "gpt-4.1", "gpt-4o-mini", "gpt-4o"],
    "anthropic": ["claude-sonnet-4-5", "claude-haiku-4-5", "claude-opus-4-1"],
    "gemini": ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"],
    "xai": ["grok-4", "grok-3-mini", "grok-3"],
    "claude_cli": ["sonnet", "opus", "haiku"],
    "codex_cli": ["gpt-5-codex", "gpt-5", "gpt-5-mini"],
    "grok_cli": ["grok-4", "grok-code-fast-1", "grok-3-mini"],
    "demo": ["demo"],
}


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
    return ""  # CLIs: their own default


def models_for(provider: str, extra: tuple[str, ...] = ()) -> list[str]:
    """Suggestions: the env default first, then ids the CLI reported (``extra``), then our list."""
    choice = (provider or "auto").lower().strip()
    if choice == "auto":
        return []
    names = [default_model(choice), *extra, *(_SUGGEST.get(choice) or [])]
    return [n for n in dict.fromkeys(names) if n]


def resolve_model(provider: str, chosen: str | None) -> str | None:
    """A typed custom id is kept when it is a plausible id; otherwise the provider default."""
    choice = (provider or "auto").lower().strip()
    if choice == "auto":
        return None
    name = (chosen or "").strip()
    if name and MODEL_ID.match(name):
        return name
    return default_model(choice) or None
