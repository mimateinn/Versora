"""Translation purposes ("用途"): one system prompt per kind of text, each file carrying a version.

Shipped presets live in ``prompts/<id>.md`` (bump ``version:`` whenever the text changes); the one
Custom purpose lives in ``data/prompts/custom.md`` and bumps its own version on every save.
``prompt_key`` (preset id + its version + the runtime format version) is what any output cache
must key on; Versora has no output cache yet, so nothing stores it today.
The glossary block is always appended last and always wins.
"""

from __future__ import annotations

import re
from pathlib import Path

from .config import project_root

PRESETS = ("general", "technical", "ui", "subtitles", "game", "legal", "academic", "business")
CUSTOM = "custom"
PURPOSES = PRESETS + (CUSTOM,)
DEFAULT = "general"
FORMAT_VERSION = 2  # lossless layout / literal-mark framing; bump with RULES

RULES = (
    "Input: numbered lines, each `N. text`. Reply with exactly the same numbers, in the same order, "
    "one line per number, each holding only the translation of that line. The mark ⏎ is a line break "
    "inside an item: keep every ⏎ where it belongs. A batch header may name another break mark; use that mark instead. "
    "A doubled break mark is literal text: keep it doubled. Never merge or split the item's lines. "
    "Output nothing else: no preface, notes, quotes or code fences."
)
_HEAD = re.compile(r"\Aversion:\s*(\d+)\s*\n---\s*\n", re.M)


def _prompt_dir() -> Path:
    return project_root() / "prompts"


def custom_path() -> Path:
    return project_root() / "data" / "prompts" / "custom.md"


def _parse(text: str) -> tuple[int, str]:
    m = _HEAD.match(text or "")
    if not m:
        return 0, (text or "").strip()
    return int(m.group(1)), text[m.end():].strip()


def load(purpose: str) -> tuple[int, str]:
    """(version, body). Unknown purposes, or a Custom that was never saved, fall back to general."""
    if purpose == CUSTOM and custom_path().is_file():
        version, body = _parse(custom_path().read_text(encoding="utf-8"))
        if body:
            return version, body
    pid = purpose if purpose in PRESETS else DEFAULT
    return _parse((_prompt_dir() / f"{pid}.md").read_text(encoding="utf-8"))


def save_custom(body: str) -> int:
    """Store the Custom purpose; returns its new version."""
    path = custom_path()
    old = _parse(path.read_text(encoding="utf-8"))[0] if path.is_file() else 0
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(f"version: {old + 1}\n---\n{body.strip()}\n", encoding="utf-8")
    tmp.replace(path)
    return old + 1


def prompt_key(purpose: str) -> str:
    return f"{purpose}@{load(purpose)[0]}/f{FORMAT_VERSION}"


def system_prompt(purpose: str, target_lang: str, source_lang: str | None, glossary_block: str = "") -> str:
    _version, body = load(purpose)
    parts = [
        f"You are a professional translator. Translate from {source_lang or 'the detected source language'} "
        f"into {target_lang}.",
        body,
        RULES,
    ]
    if glossary_block:
        parts.append(glossary_block + "\nThese glossary terms override everything above.")
    return "\n\n".join(parts)
