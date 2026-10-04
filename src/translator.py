"""High-level translate calls used by the file handlers. All of them go through src.runtime."""

from __future__ import annotations

import re
from typing import Callable, Optional, Tuple

from .config import get_chunk_size
from .glossary import glossary_to_prompt_block, load_glossary
from .prompts import DEFAULT, system_prompt
from .providers.base import TranslationError
from .runtime import Runner, apply_limits_from_prefs, resolve_chain, translate_units

_PARA = re.compile(r"(\n[ \t]*\n+)")  # blank-line separators are kept as they are


def _translate(units, target_lang, source_lang, project, provider_choice, model, purpose, cancel, on_chunk):
    pairs = load_glossary(project) if project else []
    system = system_prompt(purpose or DEFAULT, target_lang, source_lang, glossary_to_prompt_block(pairs))
    apply_limits_from_prefs()
    runner = Runner(resolve_chain(provider_choice, model), cancel)
    return translate_units(units, system=system, target_lang=target_lang, runner=runner,
                           max_chars=get_chunk_size(), cancel=cancel, on_chunk=on_chunk)


def translate_document(
    text: str,
    target_lang: str,
    source_lang: Optional[str] = None,
    project: Optional[str] = None,
    provider_choice: str = "auto",
    model: str | None = None,
    purpose: str = DEFAULT,
    cancel=None,
    on_chunk: Optional[Callable[[int, int], None]] = None,
) -> Tuple[str, int]:
    """Paragraphs are the units, so sentences keep their context. Returns (text, batches)."""
    if not text or not text.strip():
        raise TranslationError("Empty text; nothing to translate.")
    seen = {"n": 0}

    def count(done: int, total: int) -> None:
        seen["n"] = total
        if on_chunk:
            on_chunk(done, total)

    parts = _PARA.split(text)
    out = _translate(parts, target_lang, source_lang, project, provider_choice, model, purpose, cancel, count)
    return "".join(out), seen["n"]


def translate_string_list(
    strings: list[str],
    target_lang: str,
    source_lang: Optional[str] = None,
    project: Optional[str] = None,
    provider_choice: str = "auto",
    model: str | None = None,
    purpose: str = DEFAULT,
    cancel=None,
    on_chunk: Optional[Callable[[int, int], None]] = None,
) -> list[str]:
    """Player-facing / cell strings. Identical inputs share one result."""
    if not strings:
        return []
    unique = list(dict.fromkeys(strings))
    done = _translate(unique, target_lang, source_lang, project, provider_choice, model, purpose, cancel, on_chunk)
    mapping = dict(zip(unique, done))
    return [mapping[s] for s in strings]
