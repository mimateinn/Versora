"""Online API translators: one base, an OpenAI-compatible subclass (OpenAI, xAI), Anthropic, Gemini.

Official developer APIs only, through the allowlisted HTTPS client. One SDK client per provider
and key, reused and closed at exit. No hard-coded temperature (reasoning models reject it). SDK
retries are off: the runtime owns retry, backoff and failover. A cut-off or filtered answer is an
error (``truncated`` / ``refused``), never silently partial text.
"""

from __future__ import annotations

import atexit
import threading

import httpx

from ..config import get_anthropic_config, get_gemini_config, get_openai_config, get_xai_config
from ..security.hosts import assert_public_https_url
from ..security.http import make_secure_client
from ..security.secrets import redact_secrets
from .base import Engine, ProviderError, TranslationError

TIMEOUT = 120.0
MAX_TOKENS = 8192

_clients: dict[tuple, object] = {}
_clients_lock = threading.Lock()


def _client(key: tuple, make):
    with _clients_lock:
        if key not in _clients:
            _clients[key] = make()
        return _clients[key]


@atexit.register
def close_clients() -> None:
    with _clients_lock:
        for c in _clients.values():
            try:
                c.close()
            except Exception:
                pass
        _clients.clear()


def retry_after(headers) -> float | None:
    try:
        raw = (headers or {}).get("retry-after")
        return max(0.0, float(raw)) if raw is not None else None
    except (TypeError, ValueError):
        return None  # an HTTP-date Retry-After: fall back to our own backoff


def kind_for_status(code: int) -> str:
    if code in (401, 403):
        return "auth"
    if code in (408, 504):
        return "timeout"
    if code == 429 or code >= 500:
        return "limit"  # busy / overloaded / quota: transient, retried then failed over
    return "spawn"  # 400 / 404 (bad model id, bad request): this provider cannot run it


class APIEngine(Engine):
    transport = "api"
    host = ""

    def __init__(self, model: str = "") -> None:
        self.model = (model or "").strip() or self.default_model()

    def default_model(self) -> str:
        return ""

    def fail(self, kind: str, detail: str, retry: float | None = None) -> ProviderError:
        return ProviderError(kind, redact_secrets(detail)[:240], self.id, retry)


class OpenAICompatEngine(APIEngine):
    """Chat Completions shape. Subclasses only differ in config (key, base URL, default model)."""

    def config(self):
        raise NotImplementedError

    def default_model(self) -> str:
        return self.config().model

    def _sdk(self):
        from openai import OpenAI

        cfg = self.config()
        if not cfg.available or not cfg.api_key or not cfg.base_url:
            raise ProviderError("auth", f"{self.id} key missing or base URL not allowed", self.id)
        return _client((self.id, cfg.api_key, cfg.base_url), lambda: OpenAI(
            api_key=cfg.api_key, base_url=cfg.base_url, max_retries=0, timeout=TIMEOUT,
            http_client=make_secure_client(TIMEOUT),
        ))

    def complete(self, system: str, user: str, *, cancel=None) -> str:
        import openai

        client = self._sdk()
        try:
            resp = client.chat.completions.create(
                model=self.model, messages=[{"role": "system", "content": system}, {"role": "user", "content": user}],
            )
        except openai.APITimeoutError as e:
            raise self.fail("timeout", str(e)) from e
        except openai.APIConnectionError as e:
            raise self.fail("spawn", f"could not reach {self.id}: {e}") from e
        except openai.APIStatusError as e:
            raise self.fail(kind_for_status(e.status_code), f"HTTP {e.status_code}: {getattr(e, 'message', e)}",
                            retry_after(e.response.headers)) from e
        choice = resp.choices[0] if resp.choices else None
        text = ((choice.message.content if choice else "") or "").strip()
        reason = getattr(choice, "finish_reason", "") if choice else ""
        if reason == "length":
            raise self.fail("truncated", "finish_reason=length")
        if reason == "content_filter":
            raise self.fail("refused", "finish_reason=content_filter")
        if not text:
            raise self.fail("empty", "no content")
        return text


class OpenAIEngine(OpenAICompatEngine):
    id = "openai"
    host = "api.openai.com"

    def config(self):
        return get_openai_config()


class XAIEngine(OpenAICompatEngine):
    id = "xai"
    host = "api.x.ai"

    def config(self):
        return get_xai_config()


class AnthropicEngine(APIEngine):
    id = "anthropic"
    host = "api.anthropic.com"

    def default_model(self) -> str:
        return get_anthropic_config().model

    def complete(self, system: str, user: str, *, cancel=None) -> str:
        import anthropic

        cfg = get_anthropic_config()
        if not cfg.api_key:
            raise ProviderError("auth", "anthropic key missing", self.id)
        client = _client((self.id, cfg.api_key), lambda: anthropic.Anthropic(
            api_key=cfg.api_key, max_retries=0, timeout=TIMEOUT, http_client=make_secure_client(TIMEOUT),
        ))
        try:
            msg = client.messages.create(model=self.model, max_tokens=MAX_TOKENS, system=system,
                                         messages=[{"role": "user", "content": user}])
        except anthropic.APITimeoutError as e:
            raise self.fail("timeout", str(e)) from e
        except anthropic.APIConnectionError as e:
            raise self.fail("spawn", f"could not reach anthropic: {e}") from e
        except anthropic.APIStatusError as e:
            raise self.fail(kind_for_status(e.status_code), f"HTTP {e.status_code}: {getattr(e, 'message', e)}",
                            retry_after(e.response.headers)) from e
        text = "".join(getattr(b, "text", "") for b in msg.content).strip()
        if msg.stop_reason == "max_tokens":
            raise self.fail("truncated", "stop_reason=max_tokens")
        if msg.stop_reason == "refusal":
            raise self.fail("refused", "stop_reason=refusal")
        if not text:
            raise self.fail("empty", "no content")
        return text


_GEMINI_REFUSED = {"SAFETY", "RECITATION", "PROHIBITED_CONTENT", "BLOCKLIST", "SPII"}


class GeminiEngine(APIEngine):
    id = "gemini"
    host = "generativelanguage.googleapis.com"

    def default_model(self) -> str:
        return get_gemini_config().model

    def complete(self, system: str, user: str, *, cancel=None) -> str:
        cfg = get_gemini_config()
        if not cfg.api_key:
            raise ProviderError("auth", "gemini key missing", self.id)
        url = f"https://{self.host}/v1beta/models/{self.model}:generateContent"
        assert_public_https_url(url)
        client = _client((self.id,), lambda: make_secure_client(TIMEOUT))
        try:
            resp = client.post(url, headers={"x-goog-api-key": cfg.api_key}, json={
                "systemInstruction": {"parts": [{"text": system}]},
                "contents": [{"role": "user", "parts": [{"text": user}]}],
            })
            resp.raise_for_status()
        except httpx.TimeoutException as e:
            raise self.fail("timeout", str(e)) from e
        except httpx.HTTPStatusError as e:
            code = e.response.status_code
            raise self.fail(kind_for_status(code), f"HTTP {code}: {e.response.text[:200]}", retry_after(e.response.headers)) from e
        except httpx.HTTPError as e:
            raise self.fail("spawn", f"could not reach gemini: {e}") from e
        try:
            data = resp.json()
        except ValueError as e:
            raise self.fail("empty", "not JSON") from e
        if (data.get("promptFeedback") or {}).get("blockReason"):
            raise self.fail("refused", f"blocked: {data['promptFeedback']['blockReason']}")
        cand = (data.get("candidates") or [{}])[0]
        reason = cand.get("finishReason", "")
        text = "".join(p.get("text", "") for p in (cand.get("content") or {}).get("parts", []) if isinstance(p, dict)).strip()
        if reason == "MAX_TOKENS":
            raise self.fail("truncated", "finishReason=MAX_TOKENS")
        if reason in _GEMINI_REFUSED:
            raise self.fail("refused", f"finishReason={reason}")
        if not text:
            raise self.fail("empty", "no content")
        return text


API_ENGINES: dict[str, type[APIEngine]] = {
    "openai": OpenAIEngine, "anthropic": AnthropicEngine, "gemini": GeminiEngine, "xai": XAIEngine,
}


def make_api_engine(pid: str, model: str = "") -> APIEngine:
    if pid not in API_ENGINES:
        raise TranslationError(f"Unknown provider: {pid}")
    return API_ENGINES[pid](model)
