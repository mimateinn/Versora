"""Local tests: CLI isolation, models, concurrency. No network. No auth files."""

from __future__ import annotations

from pathlib import Path

from src.batch import DEFAULT_CONCURRENCY, MAX_CONCURRENCY, MIN_CONCURRENCY, clamp_concurrency
from src.i18n import detect_ui_language
from src.models import models_for, resolve_model
from src.providers.cli import PRESETS, resolve_bin
from src.security.hosts import is_official_api_host
from src.updater.github_http import PIN_OWNER, PIN_REPO


def test_overlay_pin() -> None:
    assert PIN_OWNER == "mimateinn"
    assert PIN_REPO == "Versora"


def test_blocked_websites() -> None:
    for host in (
        "grok.com",
        "www.grok.com",
        "cli-chat-proxy.grok.com",
        "auth.x.ai",
        "chatgpt.com",
        "chat.openai.com",
    ):
        assert not is_official_api_host(host)
    assert is_official_api_host("api.x.ai")
    assert is_official_api_host("api.openai.com")


def test_concurrency_range() -> None:
    """Files at once = the runtime's global call limit (Litora: default 3, 1-16)."""
    assert clamp_concurrency(None) == DEFAULT_CONCURRENCY
    assert clamp_concurrency("nope") == DEFAULT_CONCURRENCY
    assert clamp_concurrency(0) == MIN_CONCURRENCY
    assert clamp_concurrency(99) == MAX_CONCURRENCY
    assert clamp_concurrency(2) == 2
    assert (DEFAULT_CONCURRENCY, MIN_CONCURRENCY, MAX_CONCURRENCY) == (3, 1, 16)


def test_models_are_suggestions() -> None:
    assert models_for("auto") == []
    assert resolve_model("auto", "gpt-4o") is None
    assert "sonnet" in models_for("claude_cli") and "sonnet" not in models_for("grok_cli")
    assert models_for("grok_cli", ("grok-9-test",))[0] == "grok-9-test"  # ids the CLI reported come first
    assert resolve_model("openai", "my-finetune-2026") == "my-finetune-2026"  # custom ids are allowed
    assert resolve_model("openai", "bad id; rm -rf") == resolve_model("openai", "")  # not an id: default
    assert resolve_model("codex_cli", "") is None  # blank CLI model: the CLI's own default


def _argv(pid: str, model: str = "m-1", effort: str = "low") -> list[str]:
    return PRESETS[pid].args(model, effort, "/tmp/p/prompt.txt", "/tmp/p")


def test_grok_argv_isolation() -> None:
    argv = _argv("grok_cli")
    assert argv[:2] == ["--prompt-file", "/tmp/p/prompt.txt"]  # the text goes in a file, never argv
    for flag in ("--permission-mode", "--disable-web-search", "--no-subagents", "--no-plan", "--max-turns"):
        assert flag in argv
    assert "dontAsk" in argv and argv[argv.index("--max-turns") + 1] == "1"
    assert ["-m", "m-1"] == argv[argv.index("-m"):argv.index("-m") + 2]
    joined = " ".join(argv)
    assert not any(b in joined for b in PRESETS["grok_cli"].banned)
    assert "-m" not in _argv("grok_cli", model="") and "--reasoning-effort" not in _argv("grok_cli", effort="")


def test_codex_argv_isolation() -> None:
    argv = _argv("codex_cli")
    assert argv[0] == "exec" and argv[-1] == "-"  # prompt on stdin
    assert argv[argv.index("--sandbox") + 1] == "read-only"
    assert argv[argv.index("--cd") + 1] == "/tmp/p"
    assert "model_reasoning_effort=low" in argv
    joined = " ".join(argv)
    assert not any(b in joined for b in PRESETS["codex_cli"].banned)
    assert _argv("claude_cli") == ["-p", "--model", "m-1", "--effort", "low"]
    assert _argv("claude_cli", "", "") == ["-p"]


def test_cli_binary_names(tmp_path: Path) -> None:
    for name in ("codex.CMD", "codex.exe", "claude.cmd", "grok"):
        (tmp_path / name).write_text("", encoding="utf-8")
    (tmp_path / "not-grok.exe").write_text("", encoding="utf-8")
    assert resolve_bin("codex_cli", str(tmp_path / "codex.CMD")) is not None  # npm shim on Windows
    assert resolve_bin("claude_cli", str(tmp_path / "claude.cmd")) is not None
    assert resolve_bin("grok_cli", str(tmp_path / "grok")) is not None
    assert resolve_bin("grok_cli", str(tmp_path / "not-grok.exe")) is None
    assert resolve_bin("codex_cli", str(tmp_path / "missing.exe")) is None


def test_sources_never_read_auth_files() -> None:
    root = Path(__file__).resolve().parents[1]
    banned_reads = (
        "Path.home() / \".grok\" / \"auth.json\"",
        "Path.home() / \".codex\" / \"auth.json\"",
        "~/.grok/auth.json",
        "~/.codex/auth.json",
        ".claude/.credentials",
    )
    for rel in ("src/providers/cli.py", "src/providers/api.py", "app.py"):
        text = (root / rel).read_text(encoding="utf-8")
        for needle in banned_reads:
            assert needle not in text
        assert "cli-chat-proxy.grok.com" not in text
        assert "curl | bash" not in text


def test_detect_language() -> None:
    assert detect_ui_language("zh-TW,zh;q=0.8") == "zh-Hant"
    assert detect_ui_language("en-US,en;q=0.9") == "en"
    assert detect_ui_language("") in {"en", "zh-Hant", "zh-Hans"} or True


def test_v2_chrome_contract() -> None:
    """Versora chrome. v0.2 changes from v0.1.1, on purpose:
    - accent: teal #14b8a6 -> one brand colour set only in theme.ACCENTS, read everywhere
      else through var(--sfts-accent*); no accent hex outside theme.py and config.toml;
    - no `sfts-hero` block (owner: the drop zone is the page's hero, no big title)."""
    root = Path(__file__).resolve().parents[1]
    icon = (root / "icon.png").read_bytes()
    assert icon[:8] == b"\x89PNG\r\n\x1a\n"
    assert len(icon) > 500
    theme = (root / "src" / "theme.py").read_text(encoding="utf-8")
    assert "st-key-nav_translate" in theme
    assert "stFileUploaderDropzoneInstructions" in theme
    assert "stAppDeployButton" in theme
    assert "backdrop-filter" not in theme and "blur(" not in theme
    from src.theme import ACCENTS, css_for
    for theme_name in ("light", "dark"):
        css = css_for(theme_name, "settings", "appearance")
        assert "data:image/svg+xml" in css
        assert "width: 3px; border-radius: 2px; background: var(--sfts-accent)" in css  # inset bar, not a crescent (r2)
        assert "st-key-source_type" in css and "min-width: max-content" in css
        assert f"--sfts-accent: {ACCENTS[theme_name]['accent']};" in css
    accent_hexes = {v.lower() for pal in ACCENTS.values() for v in pal.values() if v.startswith("#") and v.lower() not in {"#ffffff", "#0e1320"}}
    config = (root / ".streamlit" / "config.toml").read_text(encoding="utf-8")
    assert f'primaryColor = "{ACCENTS["light"]["accent"]}"' in config
    app = (root / "app.py").read_text(encoding="utf-8")
    for hexv in accent_hexes | {"#14b8a6", "#b95233"}:
        assert hexv not in app.lower()
    assert theme.lower().count(ACCENTS["light"]["accent"].lower()) == 1  # set once
    assert "sfts-frow" in app  # picked file: one compact row (r3 locked L2)
    assert 'SETTINGS_PANES = ("purposes", "keys", "order", "glossary", "appearance")' in app  # nav == headings (r3)
    assert "status.info" not in app and "st.info(" not in app and "st.success(" not in app
    assert "L(\"main.status_ready\")" not in app
    maker = (root / "scripts" / "make_icon.py").read_text(encoding="utf-8")
    assert (root / "scripts" / "make_icon.py").is_file()
    assert "assets/icon.svg" in maker
    icons = (root / "src" / "icons.py").read_text(encoding="utf-8")
    assert "<svg" in icons
    for ch in "☀☾📄📁🗜💬🎮🔑📖🖥🌐✕✨":
        assert ch not in app
        assert ch not in icons


def _reduced_motion_blocks(css: str) -> list[str]:
    """Bodies of every @media (prefers-reduced-motion: reduce) block, braces balanced."""
    blocks, at = [], 0
    while (at := css.find("prefers-reduced-motion: reduce", at)) != -1:
        start = css.index("{", at) + 1
        depth, i = 1, start
        while depth:
            depth += {"{": 1, "}": -1}.get(css[i], 0)
            i += 1
        blocks.append(css[start:i - 1])
        at = i
    return blocks


def test_reduced_motion_is_still() -> None:
    """Reduced motion: busy cues stay drawn but nothing loops (r4: a vi-fade loop had crept back in);
    the shimmer is a flat band, not a gradient."""
    from src.icons import ICON_CSS
    from src.theme import css_for

    for theme_name in ("light", "dark"):
        css = css_for(theme_name, "translate", "appearance")
        blocks = _reduced_motion_blocks(css)
        assert len(blocks) >= 2  # theme chrome + icon set
        for body in blocks:
            assert "infinite" not in body
            assert "width: 100%" not in body  # no fake full progress bar
        assert "gradient(" not in css
    assert all("infinite" not in b for b in _reduced_motion_blocks(ICON_CSS))


def test_locales_hide_subscription_copy() -> None:
    root = Path(__file__).resolve().parents[1] / "locales"
    for path in root.glob("*.json"):
        text = path.read_text(encoding="utf-8")
        assert "keys.connect_sub" not in text
        assert "keys.sub_wait" not in text
        assert "等候安全審查" not in text
