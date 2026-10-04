"""Shared translation runtime for every translator (CLI, API, demo).

- Batch format: numbered lines ``N. text`` (a line break inside an item travels as `` ⏎ ``). The reply
  is parsed strictly; when it does not parse, or comes back cut off, the batch is split in half and
  each half retried, down to one item.
- Layout never travels: an item's outer whitespace and every line's indent / trailing whitespace stay
  here (``Shape``) and are put back around the reply's lines, so a model or transport that trims or
  re-spaces cannot flatten nested lists, indented code or a file's final newline.
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
MARKS = (NL, "␤", "↵")  # line-break marks; a batch uses the first one its text does not contain
_H = " \t\r\f\v"  # whitespace at a line's edges (layout, not content)
_WS = _H + "\n"


class ParseError(Exception):
    pass


# ── numbered lines ──────────────────────────────────────────────────────────


def encode(text: str, mark: str = NL) -> str:
    """Lossless one-line form: a line break is `` ⏎ `` (one framing space each side), a literal ⏎ is ⏎⏎."""
    return text.replace(mark, mark * 2).replace("\n", f" {mark} ")


def decode(text: str, mark: str = NL) -> str:
    """Inverse of ``encode``; a bare ⏎ (a model that dropped the framing spaces) is a line break too."""
    m = re.escape(mark)
    return re.sub(rf"{m}{m}| {m} |{m}", lambda hit: mark if hit.group(0) == mark * 2 else "\n", text)


def numbered(items: list[str], mark: str = NL) -> str:
    return "\n".join(f"{i}. {encode(t, mark)}" for i, t in enumerate(items, 1))


@dataclass(frozen=True)
class Shape:
    """One item split into what the model sees (``text``: trimmed lines) and the layout it never sees."""

    head: str
    tail: str
    edges: tuple[tuple[str, str], ...]  # (indent, trailing whitespace) of each line
    text: str

    @classmethod
    def of(cls, item: str) -> "Shape":
        lead = len(item) - len(item.lstrip(_WS))
        core = item[lead:].rstrip(_WS)
        edges, lines = [], []
        for line in core.split("\n"):
            body = line.strip(_H)
            indent = line[:len(line) - len(line.lstrip(_H))]
            edges.append((indent, line[len(indent) + len(body):]))
            lines.append(body)
        return cls(item[:lead], item[lead + len(core):], tuple(edges), "\n".join(lines))

    def apply(self, lines: list[str]) -> str:
        """Put source layout around translated lines; changed line counts are a malformed reply."""
        if len(lines) != len(self.edges):
            raise ParseError("translated line-break structure changed")  # fail closed, never flatten an outline
        body = "\n".join(f"{a}{line}{b}" for (a, b), line in zip(self.edges, lines))
        return f"{self.head}{body}{self.tail}"


def _mark(texts: list[str]) -> str:
    return next((m for m in MARKS if not any(m in t for t in texts)), NL)


def _lines(raw: str, mark: str = NL) -> list[str]:
    """One reply item as lines; whitespace next to a line break is framing, never content."""
    raw = re.sub(rf"{re.escape(mark)}[ \t]*\r?\n", mark, raw.strip(_WS))  # a model that also broke the line
    return [line.strip(_H) for line in decode(raw, mark).split("\n")]


_LINE = re.compile(r"^\s*(\d+)[.)]\s?(.*)$")
_FENCE = re.compile(r"^\s*```")


def _items(reply: str, n: int) -> list[str]:
    """Raw (still encoded) items 1..n, in order. A line that is not the next number continues the
    current item (a model that broke a line); anything before item 1, a gap or a missing tail is a
    ParseError. Only a code fence wrapped around the whole reply is dropped; fences inside stay."""
    lines = (reply or "").strip(_WS).split("\n")  # Unicode separators inside content are not protocol rows
    if len(lines) > 1 and _FENCE.match(lines[0]):
        lines = lines[1:-1] if _FENCE.match(lines[-1]) else lines[1:]
    items: list[str] = []
    for line in lines:
        m = _LINE.match(line)
        if m and int(m.group(1)) == len(items) + 1 and len(items) < n:
            items.append(m.group(2))
        elif items:
            items[-1] += "\n" + line
        elif line.strip():
            raise ParseError(f"text before item 1: {line[:40]!r}")
    if len(items) != n:
        raise ParseError(f"expected {n} items, got {len(items)}")
    return items


def parse_numbered(reply: str, n: int, mark: str = NL) -> list[str]:
    """Items 1..n as text, each line trimmed (layout comes back through ``Shape``)."""
    return ["\n".join(_lines(t, mark)) for t in _items(reply, n)]


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
                except Exception as e:  # an engine bug or OS error: this translator is out, try the next
                    last = ProviderError("spawn", f"{type(e).__name__}: {e}"[:240], link.id)
                    note(link.id, "spawn")
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
    lines = (reply or "").strip(_WS).split("\n")
    m = _LINE.match(lines[0]) if lines else None
    if m and m.group(1) == "1":
        lines[0] = m.group(2)
    return "\n".join(lines)


_REASK = ("\nThe previous answer broke a one-line item: return exactly one numbered item on one physical "
          "line, with no added line breaks. Keep all literal line-break marks.")


def translate_batch(runner: Runner, system: str, header: str, items: list[str], *, reask: bool = True) -> list[str]:
    """One numbered batch; on a reply that does not parse (or is cut off) split in half and retry.
    Only each item's trimmed lines are sent; its layout is restored from the source (``Shape``).
    A lone one-line item whose answer gained a line break is asked once more (``reask``), then fails."""
    shapes = [Shape.of(t) for t in items]
    texts = [s.text for s in shapes]
    mark = _mark(texts)
    note_mark = "" if mark == NL else f"\nIn this batch the line-break mark is {mark}, not ⏎: keep every {mark} where it belongs."
    try:
        reply = runner.call(system, f"{header}{note_mark}\n\n{numbered(texts, mark)}")
        if len(items) == 1:
            try:
                raw = _items(reply, 1)
            except ParseError:
                raw = [_strip_number(reply)]  # one item cannot be misaligned: take it whole
        else:
            raw = _items(reply, len(items))
        return [s.apply(_lines(t, mark)) for s, t in zip(shapes, raw)]
    except (ParseError, ProviderError) as e:
        if len(items) == 1 and isinstance(e, ParseError) and len(shapes[0].edges) > 1:
            # A model may reflow a wrapped paragraph. Retry its source lines as separate numbered
            # units, retaining their layout, instead of flattening it or failing the whole file.
            lines = items[0].split("\n")
            todo = [i for i, line in enumerate(lines) if line.strip()]
            if not todo:
                return items
            translated = translate_batch(runner, system, header + "\nEach number is one source line; do not reflow it.", [lines[i] for i in todo])
            for i, text in zip(todo, translated):
                lines[i] = text
            return ["\n".join(lines)]
        if len(items) == 1 and isinstance(e, ParseError) and reask:
            # A model may break a one-line item. Ask once more, parsed just as strictly; a second broken
            # answer fails closed rather than being joined or flattened into one line.
            return translate_batch(runner, system, header + _REASK, items, reask=False)
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
