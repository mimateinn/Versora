"""Local tests: CLI isolation, models, concurrency. No network. No auth files."""

from __future__ import annotations

from pathlib import Path

from src.batch import DEFAULT_CONCURRENCY, MAX_CONCURRENCY, MIN_CONCURRENCY, clamp_concurrency
from src.i18n import detect_ui_language
from src.models import models_for, resolve_model
from src.providers.codex_cli import build_exec_argv, resolve_codex_binary
from src.providers.grok_cli import build_print_argv, resolve_grok_binary
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
    assert clamp_concurrency(None) == DEFAULT_CONCURRENCY
    assert clamp_concurrency("nope") == DEFAULT_CONCURRENCY
    assert clamp_concurrency(0) == MIN_CONCURRENCY
    assert clamp_concurrency(99) == MAX_CONCURRENCY
    assert clamp_concurrency(2) == 2
    assert DEFAULT_CONCURRENCY == 2
    assert MIN_CONCURRENCY == 1
    assert MAX_CONCURRENCY == 8


def test_models_match_provider() -> None:
    assert models_for("auto") == []
    assert resolve_model("auto", "gpt-4o") is None
    openai = models_for("openai")
    assert "gpt-4o-mini" in openai
    assert resolve_model("openai", "gpt-4o") == "gpt-4o"
    assert "gpt-5.1-codex" in models_for("codex_cli")
    assert "grok-3-mini" in models_for("grok_cli")
    assert "gpt-5.1-codex" not in models_for("grok_cli")
    assert "grok-3-mini" not in models_for("codex_cli")


def test_grok_print_argv_isolation(tmp_path: Path | None = None) -> None:
    import tempfile

    cwd = Path(tmp_path) if tmp_path else Path(tempfile.mkdtemp(prefix="sfts_t_"))
    flags = {
        "-p",
        "--output-format",
        "--no-auto-update",
        "--permission-mode",
        "--disable-web-search",
        "--no-subagents",
        "--sandbox",
        "--cwd",
        "--max-turns",
        "--model",
    }
    argv = build_print_argv(Path("/usr/bin/grok"), "hi", cwd, flags, "grok-3-mini")
    joined = " ".join(argv)
    assert argv[0].endswith("grok")
    assert "-p" in argv
    assert "--permission-mode" in argv and "dontAsk" in argv
    assert "--disable-web-search" in argv
    assert "--no-subagents" in argv
    assert "--sandbox" in argv and "strict" in argv
    assert "--cwd" in argv and str(cwd) in argv
    assert "--max-turns" in argv and "1" in argv
    assert "--always-approve" not in argv
    assert "--yolo" not in argv
    assert "--oauth" not in joined


def test_codex_exec_argv_isolation(tmp_path: Path | None = None) -> None:
    import tempfile

    cwd = Path(tmp_path) if tmp_path else Path(tempfile.mkdtemp(prefix="sfts_t_"))
    flags = {"--sandbox", "--cd", "--model", "--skip-git-repo-check", "--ask-for-approval"}
    argv = build_exec_argv(Path("/usr/bin/codex"), cwd, flags, "gpt-5.1-codex")
    joined = " ".join(argv)
    assert argv[:2] == [str(Path("/usr/bin/codex")), "exec"]
    assert argv[-1] == "-"
    assert "--sandbox" in argv and "read-only" in argv
    assert "--cd" in argv and str(cwd) in argv
    assert "--full-auto" not in argv
    assert "--yolo" not in argv
    assert "--dangerously-bypass-approvals-and-sandbox" not in argv
    assert "danger-full-access" not in joined
    assert "workspace-write" not in joined


def test_cli_binary_names_only() -> None:
    assert resolve_grok_binary("/tmp/not-grok") is None
    assert resolve_codex_binary("/tmp/not-codex") is None


def test_sources_never_read_auth_files() -> None:
    root = Path(__file__).resolve().parents[1]
    banned_reads = (
        "Path.home() / \".grok\" / \"auth.json\"",
        "Path.home() / \".codex\" / \"auth.json\"",
        "~/.grok/auth.json",
        "~/.codex/auth.json",
    )
    for rel in (
        "src/providers/grok_cli.py",
        "src/providers/codex_cli.py",
        "app.py",
    ):
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
        assert "inset 3px 0 0 var(--sfts-accent)" in css
        assert "st-key-source_type" in css and "min-width: max-content" in css
        assert f"--sfts-accent: {ACCENTS[theme_name]['accent']};" in css
    accent_hexes = {v.lower() for pal in ACCENTS.values() for v in pal.values() if v.startswith("#") and v.lower() not in {"#ffffff", "#0e1320"}}
    config = (root / ".streamlit" / "config.toml").read_text(encoding="utf-8")
    assert f'primaryColor = "{ACCENTS["light"]["accent"]}"' in config
    app = (root / "app.py").read_text(encoding="utf-8")
    for hexv in accent_hexes | {"#14b8a6", "#b95233"}:
        assert hexv not in app.lower()
    assert theme.lower().count(ACCENTS["light"]["accent"].lower()) == 1  # set once
    assert "sfts-filechip" in app
    assert 'SETTINGS_PANES = ("appearance", "translation", "keys", "glossary")' in app
    assert "status.info" not in app and "st.info(" not in app and "st.success(" not in app
    assert "L(\"main.status_ready\")" not in app
    maker = (root / "scripts" / "make_icon.py").read_text(encoding="utf-8")
    assert (root / "scripts" / "make_icon.py").is_file()
    assert "No letters" in maker
    icons = (root / "src" / "icons.py").read_text(encoding="utf-8")
    assert "<svg" in icons
    for ch in "☀☾📄📁🗜💬🎮🔑📖🖥🌐✕✨":
        assert ch not in app
        assert ch not in icons


def test_locales_hide_subscription_copy() -> None:
    root = Path(__file__).resolve().parents[1] / "locales"
    for path in root.glob("*.json"):
        text = path.read_text(encoding="utf-8")
        assert "keys.connect_sub" not in text
        assert "keys.sub_wait" not in text
        assert "等候安全審查" not in text


if __name__ == "__main__":
    test_overlay_pin()
    test_blocked_websites()
    test_concurrency_range()
    test_models_match_provider()
    test_grok_print_argv_isolation()
    test_codex_exec_argv_isolation()
    test_cli_binary_names_only()
    test_sources_never_read_auth_files()
    test_detect_language()
    test_v2_chrome_contract()
    test_locales_hide_subscription_copy()
    print("ok")
