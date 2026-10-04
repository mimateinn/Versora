"""Per-project glossary (context memory). Stored as local JSON with English filename."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Dict, List, Tuple

from .config import projects_dir


def _safe_name(name: str) -> str:
    """Sanitize project name to a safe directory name."""
    name = (name or "").strip()
    name = re.sub(r"[^\w\-]+", "_", name, flags=re.UNICODE)
    name = name.strip("_") or "default"
    return name[:64]


def list_projects() -> List[str]:
    root = projects_dir()
    names = []
    for p in sorted(root.iterdir()):
        if p.is_dir() and (p / "glossary.json").is_file():
            names.append(p.name)
        elif p.is_dir() and not any(p.iterdir()):
            # empty project dir still counts
            names.append(p.name)
    return names


def ensure_project(name: str) -> Path:
    safe = _safe_name(name)
    d = projects_dir() / safe
    d.mkdir(parents=True, exist_ok=True)
    # Windows may reuse a differently cased directory. Return its disk name, not the typed spelling.
    # samefile keeps distinct case-sensitive projects distinct on other filesystems.
    for existing in d.parent.iterdir():
        if existing.name.casefold() == safe.casefold() and existing.is_dir() and existing.samefile(d):
            d = existing
            break
    gl = d / "glossary.json"
    if not gl.exists():
        gl.write_text("[]\n", encoding="utf-8")
    return d


def load_glossary(project: str) -> List[Tuple[str, str]]:
    """Return list of (term, translation) pairs."""
    safe = _safe_name(project)
    path = projects_dir() / safe / "glossary.json"
    if not path.is_file():
        return []
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, list):
            return []
        pairs = []
        for item in data:
            if isinstance(item, dict):
                t = str(item.get("term", "")).strip()
                tr = str(item.get("translation", "")).strip()
                if t:
                    pairs.append((t, tr))
            elif isinstance(item, (list, tuple)) and len(item) >= 2:
                t = str(item[0]).strip()
                tr = str(item[1]).strip()
                if t:
                    pairs.append((t, tr))
        return pairs
    except (json.JSONDecodeError, OSError):
        return []


def save_glossary(project: str, pairs: List[Tuple[str, str]]) -> None:
    safe = _safe_name(project)
    d = ensure_project(safe)
    clean = [{"term": t, "translation": tr} for t, tr in pairs if t.strip()]
    (d / "glossary.json").write_text(
        json.dumps(clean, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def glossary_to_prompt_block(pairs: List[Tuple[str, str]]) -> str:
    """Format glossary for injection into the system / user prompt."""
    if not pairs:
        return ""
    lines = ["Preferred terminology (must follow when the term appears):"]
    for term, trans in pairs:
        if trans:
            lines.append(f"- {term} → {trans}")
        else:
            lines.append(f"- {term}")
    return "\n".join(lines)
