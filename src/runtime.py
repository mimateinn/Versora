"""Shared translation runtime for every translator (CLI, API, demo).

- Batch format: numbered lines ``N. text`` (a line break inside an item travels as ⏎). The reply is
  parsed strictly; when it does not parse, or comes back cut off, the batch is split in half and
  each half retried, down to one item.
- Calls: retried twice for transient kinds (limit / timeout / empty) with backoff 1 → 30 s that
  honours Retry-After; a wait over 30 s, or auth / refused / spawn, fails over to the next
  translator in the user's ordered chain. "Auto" means that chain.
- Gates: a global limit (default 3) and a per-translator limit (default 1) hold on every call.
- Health: the last failure per translator is kept for the Settings state chips.
"""

from __future__ import annotations

import re
import threading
import time
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Callable, Iterator

from .providers.base import Cancelled, Engine, ProviderError, TranslationError

RETRIES = 2
MAX_WAIT = 30.0
BATCH_ITEMS = 60
GLOBAL_LIMIT = (1, 16, 3)  # min, max, default
PER_LIMIT = (1, 8, 1)
TRANSIENT = ("limit", "timeout", "empty")
NL = "⏎"


class ParseError(Exception):
    pass


# ── numbered lines ──────────────────────────────────────────────────────────


def encode(text: str) -> str:
    return re.sub(r"\r?\n", f" {NL} ", text)


def decode(text: str) -> str:
    return re.sub(rf"[ \t]*{NL}[ \t]*", "\n", text)


def numbered(items: list[str]) -> str:
    return "\n".join(f"{i}. {encode(t)}" for i, t in enumerate(items, 1))


_LINE = re.compile(r"^\s*(\d+)[.)]\s?(.*)$")
_FENCE = re.compile(r"^\s*```")


def parse_numbered(reply: str, n: int) -> list[str]:
    """Exactly items 1..n, in order. A line that is not the next number continues the current item
    (a model that broke a line); anything before item 1, a gap or a missing tail is a ParseError."""
    items: list[str] = []
    for line in (reply or "").strip().splitlines():
        if _FENCE.match(line):
            continue
        m = _LINE.match(line)
        if m and int(m.group(1)) == len(items) + 1 and len(items) < n:
            items.append(m.group(2))
        elif items:
            items[-1] += "\n" + line
        elif line.strip():
            raise ParseError(f"text before item 1: {line[:40]!r}")
    if len(items) != n:
        raise ParseError(f"expected {n} items, got {len(items)}")
    return [decode(t).strip() for t in items]


# ── gates ───────────────────────────────────────────────────────────────────


def clamp(value, bounds: tuple[int, int, int]) -> int:
    lo, hi, default = bounds
    try:
        return max(lo, min(hi, int(value)))
    except (TypeError, ValueError):
        return default


_gate_lock = threading.Lock()
_limits = {"global": GLOBAL_LIMIT[2], "per": PER_LIMIT[2]}
_global_sem = threading.BoundedSemaphore(_limits["global"])
_per_sem: dict[str, threading.BoundedSemaphore] = {}


def set_limits(global_n=None, per_n=None) -> tuple[int, int]:
    """New limits apply to calls that start after this; calls in flight finish on the old gates."""
    global _global_sem, _per_sem
    g, p = clamp(global_n, GLOBAL_LIMIT), clamp(per_n, PER_LIMIT)
    with _gate_lock:
        if g != _limits["global"]:
            _limits["global"], _global_sem = g, threading.BoundedSemaphore(g)
        if p != _limits["per"]:
            _limits["per"], _per_sem = p, {}
    return g, p


def _acquire(sem: threading.Semaphore, cancel) -> None:
    while not sem.acquire(timeout=0.2):
        if cancel is not None and cancel.is_set():
            raise Cancelled()


@contextmanager
def gates(pid: str, cancel=None) -> Iterator[None]:
    with _gate_lock:
        per = _per_sem.setdefault(pid, threading.BoundedSemaphore(_limits["per"]))
        glob = _global_sem
    _acquire(per, cancel)  # per first: a call waiting on a busy translator holds no global slot
    try:
        _acquire(glob, cancel)
        try:
            yield
        finally:
            glob.release()
    finally:
        per.release()


# ── health (for the Settings chips) ─────────────────────────────────────────

_health: dict[str, dict] = {}


def note(pid: str, kind: str | None, retry_until: float = 0.0) -> None:
    _health[pid] = {"kind": kind, "at": time.time(), "retry_until": retry_until}


def health(pid: str) -> dict:
    return dict(_health.get(pid) or {})


# ── chain ───────────────────────────────────────────────────────────────────


@dataclass
class Link:
    id: str
    model: str = ""
    effort: str = ""
    enabled: bool = True


def backoff(attempt: int, retry_after: float | None = None) -> float | None:
    """Seconds to wait before retry ``attempt`` (0-based): 1, 2, 4 … capped at 30; Retry-After wins
    when longer. None = the wait would exceed 30 s: give up on this translator and fail over."""
    wait = min(MAX_WAIT, float(2 ** attempt))
    if retry_after is not None:
        if retry_after > MAX_WAIT:
            return None
        wait = max(wait, retry_after)
    return wait


def make_engine(link: Link) -> Engine:
    from .providers.api import API_ENGINES, make_api_engine
    from .providers.cli import PRESETS, CLIEngine
    from .providers.demo import DemoEngine

    if link.id in PRESETS:
        return CLIEngine(link.id, link.model, link.effort)
    if link.id in API_ENGINES:
        return make_api_engine(link.id, link.model)
    if link.id == "demo":
        return DemoEngine(link.model)
    raise TranslationError(f"Unknown provider: {link.id}")


class Runner:
    """Calls down the chain; ``factory`` is injectable for tests."""

    def __init__(self, chain: list[Link], cancel=None, factory: Callable[[Link], Engine] | None = None,
                 sleep: Callable[[float], None] | None = None) -> None:
        if not chain:
            raise TranslationError("No translator is set up. Add one in Settings → Translators.")
        self.chain, self.cancel, self.factory = chain, cancel, factory or make_engine
        self.sleep = sleep
        self._engines: dict[str, Engine] = {}

    def _wait(self, seconds: float) -> None:
        if self.sleep is not None:
            self.sleep(seconds)
        elif self.cancel is not None:
            if self.cancel.wait(seconds):
                raise Cancelled()
        else:
            time.sleep(seconds)

    def call(self, system: str, user: str) -> str:
        last: TranslationError | None = None
        for link in self.chain:
            try:
                engine = self._engines.get(link.id) or self._engines.setdefault(link.id, self.factory(link))
            except TranslationError as e:
                last = e
                continue
            for attempt in range(RETRIES + 1):
                if self.cancel is not None and self.cancel.is_set():
                    raise Cancelled()
                try:
                    with gates(link.id, self.cancel):
                        text = engine.complete(system, user, cancel=self.cancel)
                    note(link.id, None)
                    return text
                except Cancelled:
                    raise
                except ProviderError as e:
                    last = e
                    if e.kind == "truncated":
                        note(link.id, e.kind)
                        raise  # the caller splits the batch
                    wait = backoff(attempt, e.retry_after) if e.kind in TRANSIENT and attempt < RETRIES else None
                    note(link.id, e.kind, time.time() + wait if wait else 0.0)
                    if wait is None:
                        break  # next translator
                    self._wait(wait)
                except TranslationError as e:
                    last = e
                    break
        raise last or TranslationError("No translator answered.")


def resolve_chain(provider_choice: str, model: str | None = None) -> list[Link]:
    """Explicit choice = that translator alone; "auto" = the user's enabled, available chain."""
    from .config import list_available_providers
    from .ui_prefs import load_prefs

    available = list_available_providers()
    prefs = load_prefs()
    rows = {r["id"]: r for r in prefs.get("chain", [])}
    choice = (provider_choice or "auto").strip()
    if choice != "auto":
        if choice not in available:
            raise ProviderError("auth", f"{choice} is not set up", choice)
        row = rows.get(choice, {})
        return [Link(choice, model or row.get("model", ""), row.get("effort", ""))]
    order = [r["id"] for r in prefs.get("chain", [])] + [a for a in available if a not in rows]
    chain = [Link(pid, rows.get(pid, {}).get("model", ""), rows.get(pid, {}).get("effort", ""))
             for pid in order if pid in available and rows.get(pid, {}).get("enabled", True)]
    return chain


def apply_limits_from_prefs() -> None:
    from .ui_prefs import load_prefs

    prefs = load_prefs()
    set_limits(prefs.get("concurrency"), prefs.get("per_provider"))


# ── translating units ───────────────────────────────────────────────────────


def _batches(items: list[str], max_chars: int) -> list[list[int]]:
    out: list[list[int]] = [[]]
    size = 0
    for i, text in enumerate(items):
        need = len(text) + 8
        if out[-1] and (size + need > max_chars or len(out[-1]) >= BATCH_ITEMS):
            out.append([])
            size = 0
        out[-1].append(i)
        size += need
    return [b for b in out if b]


def _strip_number(reply: str) -> str:
    lines = (reply or "").strip().splitlines()
    m = _LINE.match(lines[0]) if lines else None
    if m and m.group(1) == "1":
        lines[0] = m.group(2)
    return decode("\n".join(lines)).strip()


def translate_batch(runner: Runner, system: str, header: str, items: list[str]) -> list[str]:
    """One numbered batch; on a reply that does not parse (or is cut off) split in half and retry."""
    try:
        reply = runner.call(system, f"{header}\n\n{numbered(items)}")
        if len(items) == 1:
            try:
                return parse_numbered(reply, 1)
            except ParseError:
                return [_strip_number(reply)]  # one item cannot be misaligned: take it whole
        return parse_numbered(reply, len(items))
    except (ParseError, ProviderError) as e:
        if isinstance(e, ProviderError) and e.kind != "truncated" or len(items) == 1:
            raise
        mid = len(items) // 2
        return translate_batch(runner, system, header, items[:mid]) + translate_batch(runner, system, header, items[mid:])


def translate_units(
    units: list[str],
    *,
    system: str,
    target_lang: str,
    runner: Runner,
    max_chars: int,
    cancel=None,
    on_chunk: Callable[[int, int], None] | None = None,
) -> list[str]:
    """Translate each unit; blank units come back unchanged and are never sent."""
    out = list(units)
    todo = [i for i, u in enumerate(units) if u.strip()]
    header = f"Target language: {target_lang}\nTranslate each numbered line."
    batches = _batches([units[i] for i in todo], max_chars)
    if on_chunk:
        on_chunk(0, len(batches))
    for n, batch in enumerate(batches, 1):
        if cancel is not None and cancel.is_set():
            raise Cancelled()
        idx = [todo[j] for j in batch]
        for i, text in zip(idx, translate_batch(runner, system, header, [units[i] for i in idx])):
            out[i] = text
        if on_chunk:
            on_chunk(n, len(batches))
    return out
