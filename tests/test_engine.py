"""AI engine: CLI safety, output classifying, numbered-line runtime, retry / failover, keys, prompts.
No network, no real CLI: children are this Python interpreter."""

from __future__ import annotations

import sys
import threading
import time
from pathlib import Path

import pytest

import src.providers.cli as cli
from src import prompts, runtime
from src.providers.api import kind_for_status, retry_after
from src.providers.base import Cancelled, Engine, ProviderError
from src.runtime import Link, ParseError, Runner, backoff, numbered, parse_numbered, translate_units

PY = sys.executable


def test_child_env_scrubs_secrets(monkeypatch) -> None:
    for name in ("OPENAI_API_KEY", "XAI_API_KEY", "GITHUB_TOKEN", "MY_SERVICE_SECRET", "anthropic_api_key"):
        monkeypatch.setenv(name, "sk-test-123456789")
    monkeypatch.setenv("PATH_HINT_OK", "keep")
    env = cli.child_env({"GROK_MEMORY": "0"})
    assert not [k for k in env if k.upper().endswith(("_API_KEY", "_TOKEN", "_SECRET"))]
    assert env["PATH_HINT_OK"] == "keep" and env["GROK_MEMORY"] == "0"
    assert "sk-test-123456789" not in "".join(env.values())


def test_document_text_never_in_argv(monkeypatch, tmp_path) -> None:
    seen = {}

    def fake_run(argv, **kw):
        seen.update(argv=argv, stdin=kw.get("stdin_text"), env=kw["env"])
        prompt_file = argv[argv.index("--prompt-file") + 1] if "--prompt-file" in argv else ""
        seen["file"] = Path(prompt_file).read_text(encoding="utf-8") if prompt_file else ""
        out = '{"type":"text","data":"1. hola"}\n{"type":"end","stopReason":"end_turn"}\n' if prompt_file else "1. hola"
        return cli.CliResult(0, out, "")

    monkeypatch.setattr(cli, "run_cli", fake_run)
    monkeypatch.setattr(cli, "resolve_bin", lambda pid, override=None: tmp_path / pid.split("_")[0])
    monkeypatch.setenv("OPENAI_API_KEY", "sk-never-leak-0000")
    secret_text = "CONFIDENTIAL-CLAUSE-42"
    for pid in ("claude_cli", "codex_cli", "grok_cli"):
        assert cli.CLIEngine(pid, "m-1").complete("sys", f"1. {secret_text}") == "1. hola"
        assert not any(secret_text in a for a in seen["argv"]), pid
        assert secret_text in (seen["stdin"] or seen["file"]), pid
        assert "OPENAI_API_KEY" not in seen["env"]
    with pytest.raises(ProviderError) as e:
        cli.CLIEngine("codex_cli", "x; rm -rf /")
    assert e.value.kind == "spawn"


def test_run_cli_kills_on_timeout_and_cancel(tmp_path) -> None:
    t0 = time.monotonic()
    res = cli.run_cli([PY, "-c", "import time; time.sleep(30)"], cwd=tmp_path, env=cli.child_env(), timeout=1.0)
    assert res.timed_out and time.monotonic() - t0 < 10
    stop = threading.Event()
    threading.Timer(0.5, stop.set).start()
    t0 = time.monotonic()
    with pytest.raises(Cancelled):
        cli.run_cli([PY, "-c", "import time; time.sleep(30)"], cwd=tmp_path, env=cli.child_env(), cancel=stop)
    assert time.monotonic() - t0 < 10
    # stderr is drained while stdout is read: a chatty child cannot block on a full pipe
    res = cli.run_cli([PY, "-c", "import sys; sys.stderr.write('e'*300000); print('ok')"], cwd=tmp_path, env=cli.child_env(), timeout=20)
    assert res.stdout.strip() == "ok" and len(res.stderr) == 300000


def test_run_cli_caps_output_and_idle(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(cli, "OUTPUT_CAP", 100_000)
    with pytest.raises(ProviderError) as e:
        cli.run_cli([PY, "-c", "import sys; sys.stdout.write('x'*500000); sys.stdout.flush(); import time; time.sleep(30)"],
                    cwd=tmp_path, env=cli.child_env(), timeout=20)
    assert e.value.kind == "truncated"
    t0 = time.monotonic()  # grok rule: first byte then silence -> idle timeout, not the full 120 s
    res = cli.run_cli([PY, "-c", "import sys,time; print('a', flush=True); time.sleep(30)"], cwd=tmp_path,
                      env=cli.child_env(), timeout=60, first_byte=10, idle=1.0)
    assert res.timed_out and time.monotonic() - t0 < 10
    res = cli.run_cli([PY, "-c", "import time; print('{\"type\":\"end\",\"stopReason\":\"end_turn\"}', flush=True); time.sleep(30)"],
                      cwd=tmp_path, env=cli.child_env(), timeout=60, finished=cli.grok_finished)
    assert res.done_early  # a CLI that lingers after its end event is stopped


def test_classify_cli_output() -> None:
    def kind(code, out="", err="", timed_out=False):
        try:
            cli.classify(cli.CliResult(code, out, err, timed_out), out, "x")
            return "ok"
        except ProviderError as e:
            return e.kind

    assert kind(None, timed_out=True) == "timeout"
    assert kind(1, err="Error: You've hit your usage limit. Try again at 5pm.") == "limit"
    assert kind(1, err="Not logged in. Please run claude auth login") == "auth"
    assert kind(1, err="'codex' is not recognized as an internal or external command") == "spawn"
    assert kind(2, err="segfault") == "spawn"
    # seen live 2026-10-04: grok out of credit, claude's model safeguards
    assert kind(1, err='API error (status 403 Forbidden): INSUFFICIENT_BALANCE: Insufficient account balance') == "limit"
    assert kind(1, out="API Error: Opus's safeguards flagged this session (https://www.anthropic.com/legal/aup).") == "refused"
    assert kind(0, out="I'm sorry, I can't help with that.") == "refused"
    assert kind(0, out="") == "empty"
    assert kind(0, out="1. Not logged in is quoted inside a real translation") == "ok"  # a success is never auth
    assert cli.parse_codex_status("Not logged in") == (False, [])
    assert cli.parse_codex_status("Logged in using ChatGPT") == (True, [])
    assert cli.parse_claude_status('{"loggedIn": true}') == (True, [])
    assert cli.parse_claude_status("weird") == (None, [])  # unknown stays unknown, never "signed in"
    assert cli.parse_grok_models("Available models:\n * grok-4\n - grok-3-mini") == (True, ["grok-4", "grok-3-mini"])
    with pytest.raises(ProviderError) as e:
        cli.parse_grok('{"type":"text","data":"1. a"}\n{"type":"max_tokens"}\n')
    assert e.value.kind == "truncated"


def test_numbered_lines_parser() -> None:
    items = ["Hello", "two\nlines", "3. a list item"]
    assert parse_numbered(numbered(items), 3) == items  # line breaks travel as ⏎, inner numbers survive
    assert parse_numbered("```\n1. a\n2. b\n```", 2) == ["a", "b"]
    assert parse_numbered("1. a\ncontinued\n2. b", 2) == ["a\ncontinued", "b"]
    for bad in ("Here you go:\n1. a\n2. b", "1. a\n3. c", "1. a"):
        with pytest.raises(ParseError):
            parse_numbered(bad, 2)


class Fake(Engine):
    """Translates numbered lines; ``drop_over`` drops the last line of any batch bigger than that."""

    def __init__(self, pid="fake", drop_over=99, fail_kind=None, retry=None):
        self.id, self.drop_over, self.fail_kind, self.retry, self.calls = pid, drop_over, fail_kind, retry, []

    def complete(self, system, user, *, cancel=None):
        lines = [ln for ln in user.splitlines() if ln[:1].isdigit()]
        self.calls.append(len(lines))
        if self.fail_kind:
            raise ProviderError(self.fail_kind, "x", self.id, self.retry)
        if len(lines) > self.drop_over:
            lines = lines[:-1]
        return "\n".join(ln.replace(". ", ". T:", 1) for ln in lines)


def test_split_in_half_retry() -> None:
    eng = Fake(drop_over=2)
    runner = Runner([Link("fake")], factory=lambda link: eng)
    out = translate_units(["a", "", "b", "c", "d", "e"], system="s", target_lang="ja", runner=runner, max_chars=10_000)
    assert out == ["T:a", "", "T:b", "T:c", "T:d", "T:e"]  # blank unit kept, never sent
    assert eng.calls == [5, 2, 3, 1, 2]  # 5 fails, halves 2 + 3, the 3 fails again: 1 + 2


def test_backoff_and_retry_after() -> None:
    assert [backoff(i) for i in range(6)] == [1, 2, 4, 8, 16, 30]
    assert backoff(0, 7) == 7 and backoff(3, 2) == 8
    assert backoff(0, 31) is None  # longer than 30 s: fail over instead of waiting
    assert retry_after({"retry-after": "12"}) == 12 and retry_after({}) is None
    assert retry_after({"retry-after": "Wed, 21 Oct 2026 07:28:00 GMT"}) is None
    assert [kind_for_status(c) for c in (401, 403, 429, 500, 503, 504, 400, 404)] == \
        ["auth", "auth", "limit", "limit", "limit", "timeout", "spawn", "spawn"]


def test_chain_failover_and_retries() -> None:
    waits = []
    limited, signed_out, good = Fake("a", fail_kind="limit", retry=3), Fake("b", fail_kind="auth"), Fake("c")
    engines = {"a": limited, "b": signed_out, "c": good}
    runner = Runner([Link("a"), Link("b"), Link("c")], factory=lambda link: engines[link.id], sleep=waits.append)
    assert runner.call("s", "1. x") == "1. T:x"
    assert len(limited.calls) == 3 and waits == [3, 3]  # 1 + 2 retries, Retry-After honoured
    assert len(signed_out.calls) == 1  # auth: no retry, next translator
    assert runtime.health("a")["kind"] == "limit" and runtime.health("c")["kind"] is None
    lone = Runner([Link("a")], factory=lambda link: Fake("a", fail_kind="refused"), sleep=waits.append)
    with pytest.raises(ProviderError) as e:
        lone.call("s", "1. x")
    assert e.value.kind == "refused"
    slow = Fake("a", fail_kind="limit", retry=120)
    Runner([Link("a"), Link("c")], factory=lambda link: slow if link.id == "a" else good, sleep=waits.append).call("s", "1. y")
    assert len(slow.calls) == 1  # a 120 s wait is not worth it: straight to the next one


def test_gates_hold_both_limits() -> None:
    runtime.set_limits(2, 1)
    live = {"all": 0, "max_all": 0, "a": 0, "max_a": 0}
    lock = threading.Lock()

    def work(pid):
        with runtime.gates(pid):
            with lock:
                live["all"] += 1
                live[pid] = live.get(pid, 0) + 1
                live["max_all"] = max(live["max_all"], live["all"])
                live["max_a"] = max(live["max_a"], live.get("a", 0))
            time.sleep(0.05)
            with lock:
                live["all"] -= 1
                live[pid] -= 1

    threads = [threading.Thread(target=work, args=(pid,)) for pid in ("a", "a", "a", "b", "b", "c")]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert live["max_all"] <= 2 and live["max_a"] == 1
    runtime.set_limits(3, 1)


def test_keys_replace_and_remove(tmp_path, monkeypatch) -> None:
    import src.security.secrets as secrets

    monkeypatch.setattr(secrets, "_ROOT", tmp_path)
    (tmp_path / ".env").write_text("KEEP=1\nOPENAI_API_KEY=old\n", encoding="utf-8")
    secrets.save_secret_to_env("OPENAI_API_KEY", "sk-new-1234")
    assert (tmp_path / ".env").read_text(encoding="utf-8") == "KEEP=1\nOPENAI_API_KEY=sk-new-1234\n"
    secrets.remove_secret_from_env("OPENAI_API_KEY")
    assert (tmp_path / ".env").read_text(encoding="utf-8") == "KEEP=1\n"
    assert not (tmp_path / ".env.tmp").exists()  # written beside, then swapped in
    with pytest.raises(ValueError):
        secrets.save_secret_to_env("OPENAI_API_KEY", "two\nlines")


def test_prompt_presets_versioned(tmp_path, monkeypatch) -> None:
    for pid in prompts.PRESETS:
        version, body = prompts.load(pid)
        assert version >= 1 and len(body) > 80, pid
    assert "{0}" in prompts.load("ui")[1] and "%s" in prompts.load("ui")[1]
    sysmsg = prompts.system_prompt("legal", "ja", None, "Preferred terminology:\n- Lessee → 借主")
    assert sysmsg.index("ja") < sysmsg.index(prompts.RULES) < sysmsg.index("借主")  # glossary last, it wins
    monkeypatch.setattr(prompts, "custom_path", lambda: tmp_path / "custom.md")
    assert prompts.load("custom") == prompts.load("general")  # never saved: falls back
    assert prompts.save_custom("Translate like a pirate. " * 5) == 1
    assert prompts.save_custom("Translate like a poet. " * 5) == 2
    assert prompts.load("custom")[0] == 2 and prompts.prompt_key("custom") == "custom@2/f1"


def test_translators_pane_remove_is_two_step(tmp_path, monkeypatch) -> None:
    """Settings → Translators: a saved key shows only its last 4; Remove asks once, then deletes it."""
    import src.config as config
    import src.security.secrets as secrets
    import src.ui_prefs as ui_prefs
    from streamlit.testing.v1 import AppTest

    monkeypatch.setattr(ui_prefs, "prefs_path", lambda: tmp_path / ".sfts-ui.json")
    monkeypatch.setattr(secrets, "_ROOT", tmp_path)
    monkeypatch.setattr(cli, "probe", lambda pid, fresh=False: cli.CliStatus(pid, None))
    monkeypatch.setattr(config, "_available_cache", None)
    (tmp_path / ".env").write_text("OPENAI_API_KEY=sk-test-abcd1234wxyz\n", encoding="utf-8")
    monkeypatch.setenv("OPENAI_API_KEY", "sk-test-abcd1234wxyz")
    app = str(Path(__file__).resolve().parents[1] / "app.py")
    at = AppTest.from_file(app, default_timeout=60)
    at.query_params["page"], at.query_params["pane"] = "settings", "keys"
    at.run()
    shown = " ".join(m.value for m in at.markdown)
    assert "••••wxyz" in shown and "abcd1234" not in shown
    at.button(key="svc_remove_btn_openai").click().run()
    assert at.button(key="svc_rm_openai") and "OPENAI_API_KEY" in (tmp_path / ".env").read_text(encoding="utf-8")
    at.button(key="svc_rm_openai").click().run()
    assert "OPENAI_API_KEY" not in (tmp_path / ".env").read_text(encoding="utf-8")
    assert not any(b.key == "svc_rm_openai" for b in at.button)


def test_single_file_cancel_kills_cli_child(tmp_path, monkeypatch) -> None:
    """Cancel during a one-file job reaches the running CLI child (run_cli tree-kills it) at once."""
    import src.batch as batch
    import src.config as config

    class Slow(Engine):
        id = "demo"

        def complete(self, system, user, *, cancel=None):
            cli.run_cli([PY, "-c", "import time; time.sleep(60)"], cwd=tmp_path, env=cli.child_env(), cancel=cancel)
            return "1. never"

    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    monkeypatch.setattr(runtime, "make_engine", lambda link: Slow())
    src = tmp_path / "a.txt"
    src.write_text("hello", encoding="utf-8")
    stop = threading.Event()
    threading.Timer(0.5, stop.set).start()
    t0 = time.monotonic()
    with pytest.raises(Cancelled):
        batch.translate_single_file(src, tmp_path / "out.txt", target_lang="ja", source_lang=None, project=None,
                                    provider_choice="auto", game_mode=False, cancel=stop)
    assert time.monotonic() - t0 < 10 and not (tmp_path / "out.txt").exists()
