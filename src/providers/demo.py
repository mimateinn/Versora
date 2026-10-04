"""Offline demo translator. Only exists when SFTS_DEMO=1 (screenshots, trying the UI).

It does not translate: it answers the runtime's numbered lines with each line tagged by the
target code, and waits a little so progress is visible.
"""

from __future__ import annotations

import os
import re

from .base import Cancelled, Engine

_LINE = re.compile(r"^(\d+)\.\s?(.*)$")
_TARGET = re.compile(r"^Target language:\s*(\S+)", re.M)


def demo_enabled() -> bool:
    return os.getenv("SFTS_DEMO", "").strip() == "1"


def _delay() -> float:
    try:
        return max(0.0, float(os.getenv("SFTS_DEMO_DELAY", "0.3")))
    except ValueError:
        return 0.3


class DemoEngine(Engine):
    id = "demo"
    transport = "demo"

    def __init__(self, model: str = "") -> None:
        self.model = model or "demo"

    def complete(self, system: str, user: str, *, cancel=None) -> str:
        if cancel is not None and cancel.wait(_delay()):
            raise Cancelled()
        if cancel is None:
            import time

            time.sleep(_delay())
        m = _TARGET.search(user)
        target = m.group(1) if m else "xx"
        out = []
        for line in user.splitlines():
            hit = _LINE.match(line)
            if hit:
                text = hit.group(2)
                out.append(f"{hit.group(1)}. [{target}] {text}" if text.strip() else f"{hit.group(1)}. {text}")
        return "\n".join(out)
