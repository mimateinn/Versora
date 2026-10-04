from __future__ import annotations

from typing import Optional

from ..config import list_available_providers
from .base import BaseProvider, TranslationError
from .openai_provider import OpenAIProvider
from .anthropic_provider import AnthropicProvider
from .gemini_provider import GeminiProvider
from .xai_provider import XAIProvider
from .grok_cli import GrokCLIProvider
from .codex_cli import CodexCLIProvider
from .demo import DemoProvider


def resolve_provider(choice: str = "auto", model: str | None = None) -> BaseProvider:
    """
    choice: 'auto' | 'openai' | 'anthropic' | 'gemini' | 'xai' | 'grok_cli'
    Raises TranslationError with a clear message when nothing is available.
    Official developer API keys, plus the official local Grok CLI if present.
    Never grok.com web login.
    """
    available = list_available_providers()
    if not available:
        raise TranslationError(
            "No translation provider is available. "
            "Set an official developer API key in .env, then restart the app."
        )

    choice = (choice or "auto").lower().strip()
    if choice == "auto":
        choice = available[0]

    if choice == "openai":
        if "openai" not in available:
            raise TranslationError(
                "OpenAI provider selected but OPENAI_API_KEY is missing.",
                provider="openai",
            )
        return OpenAIProvider(model=model)
    if choice == "anthropic":
        if "anthropic" not in available:
            raise TranslationError(
                "Anthropic provider selected but ANTHROPIC_API_KEY is missing.",
                provider="anthropic",
            )
        return AnthropicProvider(model=model)
    if choice == "gemini":
        if "gemini" not in available:
            raise TranslationError(
                "Gemini provider selected but GEMINI_API_KEY is missing.",
                provider="gemini",
            )
        return GeminiProvider(model=model)
    if choice == "xai":
        if "xai" not in available:
            raise TranslationError(
                "xAI provider selected but XAI_API_KEY is missing.",
                provider="xai",
            )
        return XAIProvider(model=model)
    if choice == "grok_cli":
        if "grok_cli" not in available:
            raise TranslationError(
                "Grok CLI selected but the official `grok` binary is not available.",
                provider="grok_cli",
            )
        return GrokCLIProvider(model=model)
    if choice == "codex_cli":
        if "codex_cli" not in available:
            raise TranslationError(
                "Codex CLI selected but the official `codex` binary is not available.",
                provider="codex_cli",
            )
        return CodexCLIProvider(model=model)
    if choice == "demo" and "demo" in available:
        return DemoProvider(model=model)

    raise TranslationError(f"Unknown provider: {choice}")


def translate_text(
    text: str,
    target_lang: str,
    source_lang: Optional[str] = None,
    glossary_block: str = "",
    provider_choice: str = "auto",
    model: str | None = None,
) -> str:
    provider = resolve_provider(provider_choice, model=model)
    return provider.translate(
        text=text,
        target_lang=target_lang,
        source_lang=source_lang,
        glossary_block=glossary_block,
    )
