"""High-level translate pipeline: chunk → provider → join."""

from __future__ import annotations

from typing import Callable, List, Optional, Tuple

from .config import get_chunk_size
from .glossary import glossary_to_prompt_block, load_glossary
from .providers import translate_text
from .providers.base import Cancelled, TranslationError


def _split_chunks(text: str, max_chars: int) -> List[str]:
    if len(text) <= max_chars:
        return [text]
    chunks: List[str] = []
    # Prefer paragraph boundaries
    paragraphs = text.split("\n")
    buf: List[str] = []
    size = 0
    for p in paragraphs:
        plen = len(p) + 1
        if size + plen > max_chars and buf:
            chunks.append("\n".join(buf))
            buf = [p]
            size = plen
        else:
            buf.append(p)
            size += plen
    if buf:
        chunks.append("\n".join(buf))
    # Hard-split any still-too-large chunk
    final: List[str] = []
    for c in chunks:
        if len(c) <= max_chars:
            final.append(c)
        else:
            for i in range(0, len(c), max_chars):
                final.append(c[i : i + max_chars])
    return final


def translate_document(
    text: str,
    target_lang: str,
    source_lang: Optional[str] = None,
    project: Optional[str] = None,
    provider_choice: str = "auto",
    model: str | None = None,
    cancel=None,
    on_chunk: Optional[Callable[[int, int], None]] = None,
) -> Tuple[str, int]:
    """
    Returns (translated_full_text, number_of_chunks).
    Raises TranslationError on failure, Cancelled once ``cancel`` (a threading.Event) is set.
    ``on_chunk(done, total)`` runs after each chunk.
    """
    if not text or not text.strip():
        raise TranslationError("Empty text; nothing to translate.")

    pairs = load_glossary(project) if project else []
    glossary_block = glossary_to_prompt_block(pairs)
    max_chars = get_chunk_size()
    chunks = _split_chunks(text, max_chars)
    results: List[str] = []
    if on_chunk:
        on_chunk(0, len(chunks))
    for n, chunk in enumerate(chunks, 1):
        if cancel is not None and cancel.is_set():
            raise Cancelled()
        out = translate_text(
            text=chunk,
            target_lang=target_lang,
            source_lang=source_lang,
            glossary_block=glossary_block,
            provider_choice=provider_choice,
            model=model,
        )
        results.append(out)
        if on_chunk:
            on_chunk(n, len(chunks))
    return "\n".join(results), len(chunks)


_MARK = "§SFTS{i}§"


def translate_string_list(
    strings: list[str],
    target_lang: str,
    source_lang: Optional[str] = None,
    project: Optional[str] = None,
    provider_choice: str = "auto",
    model: str | None = None,
    cancel=None,
    on_chunk: Optional[Callable[[int, int], None]] = None,
) -> list[str]:
    """Translate player-facing strings. Identical inputs share one result.
    ``cancel`` / ``on_chunk`` work as in translate_document."""
    if not strings:
        return []
    unique: list[str] = []
    index: list[int] = []
    seen: dict[str, int] = {}
    for s in strings:
        if s not in seen:
            seen[s] = len(unique)
            unique.append(s)
        index.append(seen[s])

    pairs = load_glossary(project) if project else []
    glossary_block = glossary_to_prompt_block(pairs)
    extra = (
        "Keep every §SFTSn§ marker exactly. Translate only the text between markers. "
        "Do not merge blocks. Do not translate the markers."
    )
    if glossary_block:
        glossary_block = glossary_block + "\n" + extra
    else:
        glossary_block = extra

    translated_unique = [""] * len(unique)
    max_chars = get_chunk_size()
    batches: list[list[int]] = [[]]
    size = 0
    for i, s in enumerate(unique):
        need = len(s) + 16
        if batches[-1] and size + need > max_chars:
            batches.append([])
            size = 0
        batches[-1].append(i)
        size += need

    def flush(batch: list[int]) -> None:
        parts = []
        for i in batch:
            parts.append(_MARK.format(i=i))
            parts.append(unique[i])
        blob = "\n".join(parts)
        out = translate_text(
            text=blob,
            target_lang=target_lang,
            source_lang=source_lang,
            glossary_block=glossary_block,
            provider_choice=provider_choice,
            model=model,
        )
        leftover = out
        for n, i in enumerate(batch):
            token = _MARK.format(i=i)
            if token not in leftover:
                translated_unique[i] = unique[i]
                continue
            _pre, rest = leftover.split(token, 1)
            leftover = rest
            nxt = _MARK.format(i=batch[n + 1]) if n + 1 < len(batch) else None
            piece = leftover.split(nxt, 1)[0] if nxt and nxt in leftover else leftover
            translated_unique[i] = piece.strip()

    if on_chunk:
        on_chunk(0, len(batches))
    for n, batch in enumerate(batches, 1):
        if cancel is not None and cancel.is_set():
            raise Cancelled()
        flush(batch)
        if on_chunk:
            on_chunk(n, len(batches))
    return [translated_unique[i] for i in index]
