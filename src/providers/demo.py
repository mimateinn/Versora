"""Offline demo provider. Only exists when SFTS_DEMO=1 (screenshots, trying the UI).

It does not translate: it tags each line with the target code, keeps every
§SFTSn§ marker, and waits a little so progress is visible.
"""

from __future__ import annotations

import os
import re
import time
from typing import Optional

from .base import BaseProvider

_MARKER = re.compile(r"^§SFTS\d+§$")


def demo_enabled() -> bool:
    return os.getenv("SFTS_DEMO", "").strip() == "1"


def _delay() -> float:
    try:
        return max(0.0, float(os.getenv("SFTS_DEMO_DELAY", "0.3")))
    except ValueError:
        return 0.3


class DemoProvider(BaseProvider):
    name = "demo"

    def __init__(self, model: str | None = None) -> None:
        self.model = model or "demo"

    def translate(
        self,
        text: str,
        target_lang: str,
        source_lang: Optional[str] = None,
        glossary_block: str = "",
    ) -> str:
        time.sleep(_delay())
        return "\n".join(
            line if not line.strip() or _MARKER.match(line.strip()) else f"[{target_lang}] {line}"
            for line in text.split("\n")
        )
