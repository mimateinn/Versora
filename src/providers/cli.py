"""Local CLI translators (official Claude Code, Codex, Grok), modelled on Litora's engine.

Rules (security first):
- argv lists only, never a shell string; document text never in argv (stdin, or a prompt file for grok);
- the child env has every saved API key / token removed (``child_env``);
- every run has a deadline; on timeout or cancel the whole process tree is killed;
- output is capped at 4 MB; stdout and stderr are both drained, so a chatty CLI cannot hang us;
- detection and login status are probed at most once per TTL, never per chunk.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from ..security.secrets import SECRET_NAMES, redact_secrets
from .base import Cancelled, Engine, ProviderError

OUTPUT_CAP = 4 * 1024 * 1024
CALL_TIMEOUT = 120.0
PROBE_TIMEOUT = 15.0
STATUS_TTL = 300.0
SAFE_ARG = re.compile(r"^[A-Za-z0-9._:/-]{1,64}$")  # model / effort ids that may sit in argv
_SECRET_SUFFIXES = ("_API_KEY", "_SECRET", "_SECRET_KEY", "_TOKEN", "_ACCESS_KEY", "_PASSWORD")
_BIN_SUFFIXES = ("", ".exe", ".cmd", ".bat")  # npm installs a codex.CMD shim on Windows


def is_secret_name(name: str) -> bool:
    up = name.upper()
    return up in SECRET_NAMES or up.endswith(_SECRET_SUFFIXES)


def child_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    """os.environ minus every key / token Versora may have loaded (a CLI signs in on its own)."""
    env = {k: v for k, v in os.environ.items() if not is_secret_name(k)}
    env.update(extra or {})
    return env


# ── running a child ─────────────────────────────────────────────────────────


@dataclass
class CliResult:
    code: int | None
    stdout: str
    stderr: str
    timed_out: bool = False
    done_early: bool = False  # ``finished`` saw the end marker; the child was stopped after it


def kill_tree(proc: subprocess.Popen) -> None:
    try:
        if os.name == "nt":
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True, timeout=10, check=False)
        else:
            os.killpg(proc.pid, signal.SIGKILL)
    except Exception:
        pass
    try:
        proc.kill()
    except Exception:
        pass


def run_cli(
    argv: list[str],
    *,
    cwd: str | Path,
    env: dict[str, str],
    stdin_text: str | None = None,
    timeout: float = CALL_TIMEOUT,
    first_byte: float | None = None,
    idle: float | None = None,
    cancel: threading.Event | None = None,
    finished: Callable[[bytes], bool] | None = None,
) -> CliResult:
    """Run one child with deadlines. Raises Cancelled, ProviderError('spawn') or ('truncated': > 4 MB)."""
    kw: dict = dict(cwd=str(cwd), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False,
                    stdin=subprocess.PIPE if stdin_text is not None else subprocess.DEVNULL)
    if os.name == "nt":
        kw["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP | getattr(subprocess, "CREATE_NO_WINDOW", 0)
    else:
        kw["start_new_session"] = True
    try:
        proc = subprocess.Popen(argv, **kw)
    except OSError as e:
        raise ProviderError("spawn", str(e)) from e
    out, err = bytearray(), bytearray()
    lock = threading.Lock()
    state = {"last": None, "over": False}

    def pump(stream, buf: bytearray, is_out: bool) -> None:
        try:
            while True:
                chunk = stream.read1(65536)
                if not chunk:
                    return
                with lock:
                    if len(out) + len(err) + len(chunk) > OUTPUT_CAP:
                        state["over"] = True
                        return
                    buf.extend(chunk)
                    if is_out:
                        state["last"] = time.monotonic()
        except (OSError, ValueError):
            return

    threads = [threading.Thread(target=pump, args=(proc.stdout, out, True), daemon=True),
               threading.Thread(target=pump, args=(proc.stderr, err, False), daemon=True)]
    if stdin_text is not None:
        def feed() -> None:
            try:
                proc.stdin.write(stdin_text.encode("utf-8"))
                proc.stdin.close()
            except (OSError, ValueError):
                pass
        threads.append(threading.Thread(target=feed, daemon=True))
    for t in threads:
        t.start()
    start, reason = time.monotonic(), None
    while proc.poll() is None:
        now = time.monotonic()
        last = state["last"]
        if cancel is not None and cancel.is_set():
            reason = "cancel"
        elif state["over"]:
            reason = "over"
        elif now - start > timeout or (first_byte and last is None and now - start > first_byte) \
                or (idle and last is not None and now - last > idle):
            reason = "timeout"
        elif finished is not None:
            with lock:
                if finished(bytes(out)):
                    reason = "done"
        if reason:
            kill_tree(proc)
            break
        time.sleep(0.05)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    for t in threads[:2]:
        t.join(timeout=2)
    if reason == "cancel":
        raise Cancelled()
    if reason == "over" or state["over"]:
        raise ProviderError("truncated", "output over 4 MB")
    return CliResult(proc.returncode, out.decode("utf-8", "replace"), err.decode("utf-8", "replace"),
                     timed_out=reason == "timeout", done_early=reason == "done")


# ── classifying output (Litora's AGENT_SIGNS, data-driven) ─────────────────

_SIGNS = (
    ("spawn", re.compile(r"\b(?:ENOENT|EACCES)\b|is not recognized as an internal or external command", re.I)),
    ("limit", re.compile(r"^.*(?:hit your usage limit|usage limit reached|rate limit(?:ed)? |quota exceeded|too many requests|"
                         r"insufficient(?: account)? balance|insufficient_balance|credit balance is too low).*$", re.I | re.M)),
    ("auth", re.compile(r"^.*(?:not logged in|please run .{0,20}login|log ?in to continue|oauth token (?:has )?expired|"
                        r"session (?:has )?expired|invalid api key|authentication_error|401 unauthorized|not authenticated).*$", re.I | re.M)),
    ("refused", re.compile(r"^.*(?:safeguards flagged|usage polic(?:y|ies)|content policy).*$", re.I | re.M)),
)
# Only a short refusal-shaped reply counts: a long translation that quotes "I can't help" is a translation.
_REFUSAL = re.compile(r"^\s*(?:I(?:'|’)m sorry|I(?:'|’)m unable|I (?:can(?:'|’)t|cannot|won(?:'|’)t) (?:help|assist|comply|do that)|"
                      r"Sorry,? I (?:can(?:'|’)t|cannot))", re.I)


def classify(res: CliResult, body: str, provider: str) -> str:
    """Return the usable text or raise ProviderError. Auth/limit evidence counts only on a failed run."""
    if res.timed_out:
        raise ProviderError("timeout", "no answer in time", provider)
    if res.code not in (0, None) and not res.done_early:
        both = f"{res.stdout}\n{res.stderr}"
        for kind, rx in _SIGNS:
            hit = rx.search(both)
            if hit:
                raise ProviderError(kind, redact_secrets(hit.group(0).strip())[:200], provider)
        raise ProviderError("spawn", redact_secrets((res.stderr.strip() or res.stdout.strip() or f"exit {res.code}"))[:200], provider)
    text = (body or "").strip()
    if text and len(text) < 400 and _REFUSAL.match(text):
        raise ProviderError("refused", text[:200], provider)
    if not text:
        raise ProviderError("empty", "no output", provider)
    return text


# ── grok streaming-json (NDJSON events: text / thought / end / error / refusal / max_tokens …) ──

_GROK_STOP = {"refusal": "refused", "max_tokens": "truncated", "max_turn_requests": "truncated",
              "max_turns_reached": "truncated", "cancelled": "spawn", "error": "spawn"}


def _grok_events(raw: str) -> list[dict]:
    events = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(ev, dict) and isinstance(ev.get("type"), str):
            events.append(ev)
    return events


def grok_finished(raw: bytes) -> bool:
    """True once a terminal event has arrived: the CLI may linger after it, so we stop it."""
    tail = raw[-65536:].decode("utf-8", "replace")
    return any(ev["type"] == "end" or ev["type"] in _GROK_STOP for ev in _grok_events(tail))


def parse_grok(raw: str, provider: str = "grok_cli") -> str:
    parts = []
    for ev in _grok_events(raw):
        kind = ev["type"]
        if kind == "text" and isinstance(ev.get("data"), str):
            parts.append(ev["data"])
        elif kind in _GROK_STOP:
            raise ProviderError(_GROK_STOP[kind], str(ev.get("message") or kind)[:200], provider)
        elif kind == "end":
            if ev.get("stopReason") not in (None, "end_turn"):
                raise ProviderError("truncated", str(ev.get("stopReason")), provider)
            return "".join(parts).strip()
    raise ProviderError("truncated", "no end event", provider)


# ── login status: tri-state, never defaults to signed in (Litora's parsers) ───


def parse_claude_status(text: str) -> tuple[bool | None, list[str]]:
    m = re.search(r"\{[\s\S]*\}", text or "")
    if m:
        try:
            data = json.loads(m.group(0))
            if isinstance(data.get("loggedIn"), bool):
                return data["loggedIn"], []
        except json.JSONDecodeError:
            pass
    return None, []


def parse_codex_status(text: str) -> tuple[bool | None, list[str]]:
    s = text or ""
    if re.search(r"not logged ?in|logged out|login required", s, re.I):
        return False, []
    if re.search(r"logged in", s, re.I):
        return True, []
    return None, []


def parse_grok_models(text: str) -> tuple[bool | None, list[str]]:
    s = text or ""
    models = re.findall(r"^[ \t]*[*-][ \t]+([A-Za-z0-9._:/-]{1,64})", s, re.M)
    if re.search(r"not authenticated", s, re.I):
        return False, models
    if models or re.search(r"available models:", s, re.I):
        return True, models
    return None, models


# ── presets ─────────────────────────────────────────────────────────────────


def _flag(name: str, value: str) -> list[str]:
    return [name, value] if value else []  # blank model / effort drops the flag: the CLI's own default wins


@dataclass(frozen=True)
class Preset:
    id: str
    name: str
    bin: str
    path_env: str
    status_args: tuple[str, ...]
    parse_status: Callable[[str], tuple[bool | None, list[str]]]
    login_cmd: str
    install_url: str
    args: Callable[[str, str, str, str], list[str]]  # (model, effort, prompt_file, cwd) -> argv tail
    stdin: bool = True
    first_byte: float | None = None
    idle: float | None = None
    env_extra: dict[str, str] = field(default_factory=dict)
    banned: tuple[str, ...] = ()
    efforts: tuple[str, ...] = ("low", "medium", "high")


PRESETS: dict[str, Preset] = {
    "claude_cli": Preset(
        "claude_cli", "Claude Code", "claude", "CLAUDE_CLI_PATH", ("auth", "status"), parse_claude_status,
        "claude auth login", "https://docs.anthropic.com/en/docs/claude-code",
        lambda m, e, f, cwd: ["-p", *_flag("--model", m), *_flag("--effort", e)],
        banned=("--dangerously-skip-permissions",),
    ),
    "codex_cli": Preset(
        "codex_cli", "Codex", "codex", "CODEX_CLI_PATH", ("login", "status"), parse_codex_status,
        "codex login", "https://developers.openai.com/codex/cli",
        lambda m, e, f, cwd: ["exec", "--skip-git-repo-check", "--sandbox", "read-only", "--cd", cwd,
                              *_flag("-m", m), *(["-c", f"model_reasoning_effort={e}"] if e else []), "-"],
        banned=("--full-auto", "--yolo", "--dangerously-bypass-approvals-and-sandbox", "danger-full-access", "workspace-write"),
    ),
    "grok_cli": Preset(
        "grok_cli", "Grok", "grok", "GROK_CLI_PATH", ("models",), parse_grok_models,
        "grok login", "https://x.ai/cli",
        lambda m, e, f, cwd: ["--prompt-file", f, "--output-format", "streaming-json",
                              "--tools=Read", "--deny", "Read", "--deny", "MCPTool", "--permission-mode", "dontAsk",
                              "--no-plan", "--no-subagents", "--disable-web-search", "--max-turns", "1",
                              "--system-prompt-override", "Translate-text-only.Never-use-tools.", "--verbatim",
                              *_flag("-m", m), *_flag("--reasoning-effort", e)],
        stdin=False, first_byte=100.0, idle=30.0,
        env_extra={"GROK_DISABLE_AUTOUPDATER": "1", "GROK_MEMORY": "0", "GROK_SUBAGENTS": "0", "GROK_WRITE_FILE": "0"},
        banned=("--always-approve", "--yolo", "--dangerously-skip-permissions", "--oauth"),
    ),
}


def path_setting(pid: str) -> str:
    return (os.getenv(PRESETS[pid].path_env) or "").strip()


def _bin_ok(path: Path, name: str) -> bool:
    return path.stem.lower() == name and path.suffix.lower() in _BIN_SUFFIXES


def resolve_bin(pid: str, override: str | None = None) -> Path | None:
    """The official binary on PATH (``.exe/.cmd/.bat`` included) or the user's path; never downloads."""
    preset = PRESETS[pid]
    raw = (override if override is not None else path_setting(pid)).strip()
    if raw:
        p = Path(raw).expanduser()
        return p.resolve() if p.is_file() and _bin_ok(p, preset.bin) else None
    found = shutil.which(preset.bin)
    if not found:
        return None
    p = Path(found)
    return p.resolve() if _bin_ok(p, preset.bin) else None


@dataclass(frozen=True)
class CliStatus:
    id: str
    binary: str | None
    version: str = ""
    logged_in: bool | None = None
    models: tuple[str, ...] = ()
    checked_at: float = 0.0

    @property
    def present(self) -> bool:
        return self.binary is not None

    @property
    def usable(self) -> bool:  # unknown login still gets a try; a seen "not signed in" does not
        return self.present and self.logged_in is not False


_status: dict[str, CliStatus] = {}
_status_lock = threading.Lock()


def probe(pid: str, *, fresh: bool = False) -> CliStatus:
    """``bin --version`` then the preset's status command, 15 s each; cached STATUS_TTL."""
    with _status_lock:
        hit = _status.get(pid)
        if hit and not fresh and time.monotonic() - hit.checked_at < STATUS_TTL:
            return hit
    binary = resolve_bin(pid)
    status = CliStatus(pid, None, checked_at=time.monotonic())
    if binary is not None:
        preset = PRESETS[pid]
        version, logged, models = "", None, ()
        with tempfile.TemporaryDirectory(prefix="versora_probe_", ignore_cleanup_errors=True) as tmp:
            env = child_env(preset.env_extra)
            try:
                res = run_cli([str(binary), "--version"], cwd=tmp, env=env, timeout=PROBE_TIMEOUT)
                m = re.search(r"\d+\.\d+(?:\.\d+)?", f"{res.stdout} {res.stderr}")
                version = m.group(0) if m else ""
                res = run_cli([str(binary), *preset.status_args], cwd=tmp, env=env, timeout=PROBE_TIMEOUT)
                if not res.timed_out:
                    logged, found = preset.parse_status(f"{res.stdout}\n{res.stderr}")
                    models = tuple(found)
            except ProviderError:
                pass
        status = CliStatus(pid, str(binary), version, logged, models, time.monotonic())
    with _status_lock:
        _status[pid] = status
    return status


def forget_status(pid: str | None = None) -> None:
    with _status_lock:
        if pid:
            _status.pop(pid, None)
        else:
            _status.clear()


class CLIEngine(Engine):
    transport = "cli"

    def __init__(self, pid: str, model: str = "", effort: str = "") -> None:
        self.id = pid
        self.preset = PRESETS[pid]
        self.model = (model or "").strip()
        self.effort = (effort or "").strip()
        for value in (self.model, self.effort):
            if value and not SAFE_ARG.match(value):
                raise ProviderError("spawn", f"unsafe model or effort id: {value[:40]!r}", pid)

    def complete(self, system: str, user: str, *, cancel=None) -> str:
        binary = resolve_bin(self.id)
        if binary is None:
            raise ProviderError("spawn", f"{self.preset.bin} not found; install it from {self.preset.install_url}", self.id)
        prompt = f"{system}\n\n{user}"
        if len(prompt.encode("utf-8")) > OUTPUT_CAP:
            raise ProviderError("truncated", "prompt over 4 MB", self.id)
        # a CLI helper process may still hold the cwd when the CLI exits: leave the dir rather than fail
        with tempfile.TemporaryDirectory(prefix="versora_cli_", ignore_cleanup_errors=True) as tmp:
            prompt_file = ""
            if not self.preset.stdin:
                prompt_file = str(Path(tmp) / "prompt.txt")
                Path(prompt_file).write_text(prompt, encoding="utf-8")
            argv = [str(binary), *self.preset.args(self.model, self.effort, prompt_file, tmp)]
            joined = " ".join(argv)
            if any(flag in joined for flag in self.preset.banned):
                raise ProviderError("spawn", "refusing a flag that widens the CLI's permissions", self.id)
            res = run_cli(
                argv, cwd=tmp, env=child_env(self.preset.env_extra),
                stdin_text=prompt if self.preset.stdin else None,
                first_byte=self.preset.first_byte, idle=self.preset.idle, cancel=cancel,
                finished=grok_finished if self.id == "grok_cli" else None,
            )
        if self.id == "grok_cli" and not res.timed_out and (res.code == 0 or res.done_early):
            return classify(res, parse_grok(res.stdout, self.id), self.id)
        return classify(res, res.stdout, self.id)
