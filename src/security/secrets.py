"""Load and store API keys on this machine only. Never echo a full secret."""

from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Iterable

_ROOT = Path(__file__).resolve().parent.parent.parent

SECRET_NAMES = _SECRET_NAMES = (
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "XAI_API_KEY",
    "OPENROUTER_API_KEY",
    "AZURE_OPENAI_API_KEY",
)


def _from_streamlit(name: str) -> str:
    try:
        import streamlit as st

        val = st.secrets.get(name, "")
        return str(val or "").strip()
    except Exception:
        return ""


def _from_keychain(name: str) -> str:
    try:
        import keyring

        return (keyring.get_password("smart-file-translation", name) or "").strip()
    except Exception:
        return ""


def load_secret(name: str) -> str:
    """Read a secret from env / .env, Streamlit secrets, or OS keychain. Never NEXT_PUBLIC_*."""
    if name.startswith("NEXT_PUBLIC_"):
        return ""
    env_val = (os.getenv(name) or "").strip()
    if env_val:
        return env_val
    st_val = _from_streamlit(name)
    if st_val:
        return st_val
    return _from_keychain(name)


def mask_secret(value: str) -> str:
    if not value:
        return ""
    tail = value[-4:] if len(value) >= 4 else ""
    return f"connected · ••••{tail}" if tail else "connected"


def collected_secrets() -> list[str]:
    found = []
    for name in _SECRET_NAMES:
        val = load_secret(name)
        if val:
            found.append(val)
    return found


def redact_secrets(text: str, extra: Iterable[str] = ()) -> str:
    out = str(text or "")
    blobs = list(extra) + collected_secrets()
    for secret in blobs:
        if secret and secret in out:
            out = out.replace(secret, "****")
    out = re.sub(r"(sk-|rk-|xai-|gsk_|AIza)[A-Za-z0-9_-]{8,}", "****", out)
    return out


def save_secret_to_env(name: str, value: str) -> None:
    """Write or replace one key in local .env. Does not print the value."""
    if name.startswith("NEXT_PUBLIC_"):
        raise ValueError("Browser-exposed names are not allowed.")
    if name not in _SECRET_NAMES and name not in {
        "OPENAI_BASE_URL",
        "ALLOWED_API_HOSTS",
        "OPENAI_MODEL",
        "ANTHROPIC_MODEL",
        "GEMINI_MODEL",
        "XAI_MODEL",
        "GROK_CLI_PATH",
        "CODEX_CLI_PATH",
        "CLAUDE_CLI_PATH",
        "DEFAULT_PROVIDER",
    }:
        raise ValueError("Unknown setting name.")
    value = (value or "").strip()
    if any(ch in value for ch in "\r\n"):
        raise ValueError("A key is one line.")
    _rewrite_env(name, value)
    os.environ[name] = value


def remove_secret_from_env(name: str) -> None:
    """Delete one key from local .env and from this process."""
    if name not in _SECRET_NAMES:
        raise ValueError("Unknown key name.")
    _rewrite_env(name, None)
    os.environ.pop(name, None)


def _rewrite_env(name: str, value: str | None) -> None:
    """Replace (or drop, when value is None) ``name`` in .env, atomically: a temp file in the same
    folder, then os.replace, so a crash never leaves a half-written .env.
    Storage stays plaintext on this machine (same as before); the OS keychain is read when present."""
    path = _ROOT / ".env"
    lines = path.read_text(encoding="utf-8").splitlines() if path.is_file() else []
    key = f"{name}="
    out, done = [], False
    for line in lines:
        if line.startswith(key) or line.startswith(f"export {key}"):
            if value is not None and not done:
                out.append(f"{name}={value}")
            done = True
        else:
            out.append(line)
    if value is not None and not done:
        out.append(f"{name}={value}")
    tmp = path.with_name(".env.tmp")
    tmp.write_text("\n".join(out).rstrip() + "\n", encoding="utf-8")
    os.replace(tmp, path)
