"""Local tests: overlay safety, game strings, folder mirror. No GitHub required."""

from __future__ import annotations

import io
import json
import zipfile
from pathlib import Path

from src.game_text import is_identifier_like, is_player_facing, should_translate_string
from src.updater.github_http import GitHubHostError, assert_github_overlay_url, is_release_tag
from src.updater.overlay import apply_overlay_from_dir, is_protected, verify_zip_pk


def test_pin_rejects_other_hosts_and_branches() -> None:
    assert is_release_tag("v0.1.0")
    assert not is_release_tag("main")
    try:
        assert_github_overlay_url(
            "https://api.github.com/repos/mimateinn/Versora/zipball/main"
        )
        raise AssertionError("branch zipball must be rejected")
    except GitHubHostError:
        pass
    try:
        assert_github_overlay_url("https://github.com/mimateinn/Versora/archive/refs/heads/main.zip")
        raise AssertionError("github.com archive host must be rejected")
    except GitHubHostError:
        pass
    try:
        assert_github_overlay_url(
            "https://api.github.com/repos/someone-else/Versora/releases"
        )
        raise AssertionError("other owner must be rejected")
    except GitHubHostError:
        pass


def test_protected_paths() -> None:
    assert is_protected(".env")
    assert is_protected(".env.local")
    assert is_protected(".streamlit/secrets.toml")
    assert is_protected("projects/default/glossary.json")
    assert is_protected("data/outputs/a.txt")
    assert is_protected(".venv/bin/python")
    assert not is_protected("app.py")
    assert not is_protected("src/updater/overlay.py")


def test_fail_closed_and_skips_env(tmp_path: Path | None = None) -> None:
    import tempfile

    root = Path(tmp_path) if tmp_path else Path(tempfile.mkdtemp(prefix="sfts_ov_"))
    (root / "keep.txt").write_text("OLD\n", encoding="utf-8")
    (root / ".env").write_text("SECRET=1\n", encoding="utf-8")
    src = root / "incoming"
    src.mkdir()
    (src / "keep.txt").write_text("NEW\n", encoding="utf-8")
    (src / ".env").write_text("SECRET=HACK\n", encoding="utf-8")
    apply_overlay_from_dir(root, src, "v9.9.9", "deadbeef")
    assert (root / "keep.txt").read_text(encoding="utf-8") == "NEW\n"
    assert (root / ".env").read_text(encoding="utf-8") == "SECRET=1\n"
    assert (root / ".sfts-release").read_text(encoding="utf-8").splitlines()[0] == "v9.9.9"

    src2 = root / "incoming2"
    src2.mkdir()
    (src2 / "keep.txt").write_text("NEWER\n", encoding="utf-8")
    (src2 / "brand_new.txt").write_text("only in failed overlay\n", encoding="utf-8")

    def boom(*_a, **_k):
        raise RuntimeError("forced fail")

    # Fail closed: stamp write after copy; force failure by protecting dest via monkeypatch
    original = apply_overlay_from_dir
    _ = original
    from src.updater import overlay as ov

    real_write = ov.write_stamp

    def bad_write(r, tag, sha):
        raise RuntimeError("stamp fail")

    ov.write_stamp = bad_write  # type: ignore[assignment]
    try:
        try:
            ov.apply_overlay_from_dir(root, src2, "v9.9.10", "cafebabe")
            raise AssertionError("should fail")
        except RuntimeError:
            pass
        assert (root / "keep.txt").read_text(encoding="utf-8") == "NEW\n"
        assert not (root / "brand_new.txt").exists()
    finally:
        ov.write_stamp = real_write  # type: ignore[assignment]


def test_zip_pk_and_game_strings() -> None:
    verify_zip_pk(b"PK\x03\x04abcd")
    try:
        verify_zip_pk(b"not-a-zip")
        raise AssertionError("must reject")
    except ValueError:
        pass
    assert is_identifier_like("player_hp")
    assert is_identifier_like("OPEN_DOOR")
    assert is_player_facing("The door opens.")
    assert not should_translate_string("player_hp", True)
    assert should_translate_string("The door opens.", True)


def test_folder_one_to_one(tmp_path: Path | None = None) -> None:
    import tempfile

    from src.extractors.textish import translate_json

    root = Path(tmp_path) if tmp_path else Path(tempfile.mkdtemp(prefix="sfts_js_"))
    src = root / "dialog.json"
    src.write_text(json.dumps({"id": "npc_01", "line": "Hello there"}, indent=2), encoding="utf-8")

    def identity(items):
        return ["你好" if i == "Hello there" else i for i in items]

    out = translate_json(src, identity, True)
    data = json.loads(out)
    assert data["id"] == "npc_01"
    assert data["line"] == "你好"


if __name__ == "__main__":
    test_pin_rejects_other_hosts_and_branches()
    test_protected_paths()
    test_fail_closed_and_skips_env()
    test_zip_pk_and_game_strings()
    test_folder_one_to_one()
    print("ok")
