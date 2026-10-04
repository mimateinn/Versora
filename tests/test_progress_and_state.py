"""Batch progress / cancel / retry, and translate choices surviving a Settings visit."""

from __future__ import annotations

import json
import threading
from pathlib import Path

import src.batch as batch
import src.translator as translator
from src.batch import CANCELLED, BatchReport, translate_tree


def _fake_provider(monkeypatch, fail: set[str] = frozenset()):
    def fake(text, target_lang, **_kw):
        if any(word in text for word in fail):
            raise batch.TranslationError("HTTP 401 invalid api key")
        return f"[{target_lang}] {text}"

    monkeypatch.setattr(translator, "translate_text", fake)


def test_batch_progress_cancel_retry(tmp_path, monkeypatch) -> None:
    src = tmp_path / "src"
    src.mkdir()
    for i in range(6):
        (src / f"f{i}.txt").write_text(f"hello {i}\n", encoding="utf-8")
    (src / "bad.txt").write_text("boom\n", encoding="utf-8")
    monkeypatch.setattr(batch, "outputs_dir", lambda: tmp_path / "out")
    kw = dict(target_lang="ja", source_lang=None, project=None, provider_choice="auto", game_mode=False, concurrency=1)

    # Progress: a (0, total) start, then one call per file; a provider error lands in failed.
    _fake_provider(monkeypatch, fail={"boom"})
    calls = []
    report = translate_tree(src, job_name="p", on_progress=lambda d, t, item: calls.append((d, t, item)), **kw)
    assert calls[0] == (0, 7, None)
    assert [c[0] for c in calls[1:]] == list(range(1, 8))
    assert len(report.written) == 6 and [f.rel for f in report.failed] == ["bad.txt"] and not report.skipped
    assert not report.cancelled

    # Cancel after the first file: queued files are dropped and recorded as cancelled.
    cancel = threading.Event()
    shared = BatchReport()
    translate_tree(src, job_name="c", report=shared, cancel=cancel, on_progress=lambda d, t, i: d and cancel.set(), **kw)
    assert shared.cancelled
    assert len(shared.written) < 6 and any(f.error == CANCELLED for f in shared.failed)
    assert len(shared.written) + len(shared.failed) == 7
    assert all(f.error in {CANCELLED, "HTTP 401 invalid api key"} for f in shared.failed)

    # A callback that raises (Streamlit interrupting on a Cancel click) still cleans up.
    raised = BatchReport()
    try:
        translate_tree(src, job_name="r", report=raised, on_progress=lambda d, t, i: (_ for _ in ()).throw(KeyboardInterrupt) if d else None, **kw)
        raise AssertionError("callback exception must propagate")
    except KeyboardInterrupt:
        pass
    assert raised.cancelled and len(raised.written) + len(raised.failed) == 7

    # Retry only the failed ones, with a working provider this time.
    _fake_provider(monkeypatch)
    only = {f.rel for f in shared.failed}
    retry = translate_tree(src, job_name="c", only=only, **kw)
    assert {w.rel for w in retry.written} == only and not retry.failed
    assert (tmp_path / "out" / "folder_c" / "bad.txt").read_text(encoding="utf-8").startswith("[ja]")


def test_choices_survive_settings_visit(tmp_path, monkeypatch) -> None:
    """Bug 1: target/source/type/mode reset after opening Settings (widget keys were dropped)."""
    import src.config as config
    import src.ui_prefs as ui_prefs
    from streamlit.testing.v1 import AppTest

    prefs = tmp_path / ".sfts-ui.json"
    monkeypatch.setattr(ui_prefs, "prefs_path", lambda: prefs)
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)

    app = str(Path(__file__).resolve().parents[1] / "app.py")
    at = AppTest.from_file(app, default_timeout=60).run()
    at.selectbox(key="qb_target").set_value("ja").run()
    at.selectbox(key="qb_source").set_value("en").run()
    at.button_group(key="content_mode_seg").set_value("game").run()
    assert at.session_state["content_mode"] == "game"

    at.button(key="nav_settings").click().run()
    at.button(key="nav_translate").click().run()
    assert at.session_state["target_lang"] == "ja"
    assert at.selectbox(key="qb_target").value == "ja"
    assert at.selectbox(key="qb_source").value == "en"
    assert at.session_state["content_mode"] == "game"
    saved = json.loads(prefs.read_text(encoding="utf-8"))
    assert saved["target_lang"] == "ja" and saved["source_choice"] == "en"

    # A fresh session starts from the remembered choices.
    at2 = AppTest.from_file(app, default_timeout=60).run()
    assert at2.selectbox(key="qb_target").value == "ja"


def test_single_file_result_error_and_notice(tmp_path, monkeypatch) -> None:
    """Single file: once a result shows, Translate steps down to secondary and Open folder appears;
    a failure is an error card with Try again (not a toast); no translator shows the notice."""
    import src.batch as batch_mod
    import src.config as config
    import src.ui_prefs as ui_prefs
    from streamlit.testing.v1 import AppTest

    monkeypatch.setattr(ui_prefs, "prefs_path", lambda: tmp_path / ".sfts-ui.json")
    monkeypatch.setattr(config, "outputs_dir", lambda: tmp_path)
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    calls = []

    def fake(src, dest, *, on_progress=None, **_kw):
        calls.append(dest)
        if len(calls) == 2:
            raise RuntimeError("boom 7f3a")
        on_progress and on_progress(1, 1, None)
        Path(dest).write_text("[ja] hello", encoding="utf-8")

    monkeypatch.setattr(batch_mod, "translate_single_file", fake)
    app = str(Path(__file__).resolve().parents[1] / "app.py")
    at = AppTest.from_file(app, default_timeout=60)
    at.session_state["source_type"] = "file"
    at.session_state["picked_name"] = "notes.txt"
    at.session_state["picked_bytes"] = b"hello"
    at.session_state["picked_size"] = 5
    at.run()
    assert at.button(key="start_translate").proto.type == "primary"
    at.button(key="start_translate").click().run()
    assert at.session_state["result"] and not at.exception
    assert at.button(key="start_translate").proto.type == "secondary"
    assert at.button(key="open_out_single").label

    at.button(key="start_translate").click().run()  # second call fails
    assert at.session_state["single_error"]["msg"] == "boom 7f3a" and not at.session_state["result"]
    assert at.button(key="retry_single").proto.type == "primary"
    assert any("Something went wrong" in m.value for m in at.markdown)
    at.button(key="retry_single").click().run()
    assert len(calls) == 3 and at.session_state["result"] and not at.session_state["single_error"]

    monkeypatch.setattr(config, "_probe_available", lambda: [])
    monkeypatch.setattr(config, "_available_cache", None)
    at2 = AppTest.from_file(app, default_timeout=60).run()
    assert any("No translator set up yet" in m.value for m in at2.markdown)
