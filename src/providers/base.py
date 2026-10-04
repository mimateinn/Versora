"""What every translator (local CLI, online API, demo) shares: one call shape and one error type."""

from __future__ import annotations

from abc import ABC, abstractmethod

# Why a call failed. The runtime decides retry / fail over from this; the UI maps it to a sentence.
#   timeout   no answer in time            spawn  could not start or reach it
#   limit     rate limit / quota / busy    auth   not signed in / key refused
#   refused   the model declined           empty  answered with nothing
#   truncated the answer was cut off (the runtime splits the batch)
KINDS = ("timeout", "spawn", "limit", "auth", "refused", "empty", "truncated")


class TranslationError(Exception):
    """Raised when a provider call fails with a clear message."""

    def __init__(self, message: str, provider: str = ""):
        super().__init__(message)
        self.provider = provider


class Cancelled(TranslationError):
    """The user stopped the job."""

    def __init__(self) -> None:
        super().__init__("cancelled")


class ProviderError(TranslationError):
    """A classified failure. ``str()`` starts with ``[kind]`` so batch rows keep the kind as text."""

    def __init__(self, kind: str, detail: str = "", provider: str = "", retry_after: float | None = None):
        assert kind in KINDS, kind
        super().__init__(f"[{kind}] {detail}".strip(), provider)
        self.kind = kind
        self.detail = detail
        self.retry_after = retry_after


class Engine(ABC):
    """One configured translator. ``complete`` sends one system + user message, returns raw text."""

    id: str = ""
    transport: str = ""  # "cli" | "api" | "demo"

    @abstractmethod
    def complete(self, system: str, user: str, *, cancel=None) -> str:
        ...
