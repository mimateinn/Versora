"""Batch progress / cancel / retry, and translate choices surviving a Settings visit."""

from __future__ import annotations

import io
import json
import re
import threading
import tomllib
import zipfile
from pathlib import Path

import pytest

import src.batch as batch
import src.config as config
import src.runtime as runtime
from src.batch import CANCELLED, BatchItem, BatchReport, translate_tree
from src.providers.base import Engine, ProviderError

AUTH_FAIL = "[auth] HTTP 401 invalid api key"


class FakeEngine(Engine):
    """Answers the runtime's numbered lines like a translator; any line holding a ``fail`` word is refused."""

    id = "demo"
    fail: set[str] = set()

    def complete(self, system, user, *, cancel=None):
        target = re.search(r"^Target language: (\S+)", user, re.M).group(1)
        lines = re.findall(r"^(\d+)\. (.*)$", user, re.M)
        if any(w in text for _n, text in lines for w in self.fail):
            raise ProviderError("auth", "HTTP 401 invalid api key", self.id)
        return "\n".join(f"{n}. [{target}] {text}" for n, text in lines)


def _fake_provider(monkeypatch, fail: set[str] = frozenset()):
    FakeEngine.fail = set(fail)
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    monkeypatch.setattr(runtime, "make_engine", lambda link: FakeEngine())


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
    assert sorted(report.started) == sorted(report.planned)  # every file was really picked up by a worker
    assert not report.cancelled

    # Cancel after the first file: queued files are dropped and recorded as cancelled.
    cancel = threading.Event()
    shared = BatchReport()
    translate_tree(src, job_name="c", report=shared, cancel=cancel, on_progress=lambda d, t, i: d and cancel.set(), **kw)
    assert shared.cancelled
    assert len(shared.written) < 6 and any(f.error == CANCELLED for f in shared.failed)
    assert len(shared.written) + len(shared.failed) == 7
    assert all(f.error in {CANCELLED, AUTH_FAIL} for f in shared.failed)

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


class _Stop(BaseException):
    """Stands in for Streamlit's stop/rerun exception (a BaseException) raised inside a draw."""


def _slow_docs(monkeypatch, gate: str) -> dict:
    """Each file waits on ``gate`` ("release": a test Event; "cancel": the job's cancel event),
    then writes its output whatever the cancel state is, like a last chunk already answered."""
    seen = {"calls": 0, "running": 0, "release": threading.Event(), "entered": threading.Event()}

    def fake(src, dest, game_mode, **kw):
        seen["calls"] += 1
        seen["running"] += 1
        seen["entered"].set()
        try:
            assert (seen["release"] if gate == "release" else kw["cancel"]).wait(10), "gate never opened"
            Path(dest).parent.mkdir(parents=True, exist_ok=True)
            Path(dest).write_text("[ja] done", encoding="utf-8")
        finally:
            seen["running"] -= 1

    monkeypatch.setattr(batch, "_translate_document", fake)
    return seen


def _three_files(tmp_path, monkeypatch) -> tuple[Path, dict]:
    src = tmp_path / "src"
    src.mkdir()
    for i in range(3):
        (src / f"f{i}.txt").write_text(f"hello {i}\n", encoding="utf-8")
    monkeypatch.setattr(batch, "outputs_dir", lambda: tmp_path / "out")
    return src, dict(target_lang="ja", source_lang=None, project=None, provider_choice="auto", game_mode=False, concurrency=1)


def test_batch_heartbeat_while_a_file_runs(tmp_path, monkeypatch) -> None:
    """F1: the caller gets a heartbeat while a slow file has not finished; per-file progress is unchanged."""
    src, kw = _three_files(tmp_path, monkeypatch)
    seen = _slow_docs(monkeypatch, "release")
    events = []

    def tick(done, total):
        events.append(("tick", done, total))
        seen["release"].set()  # only a heartbeat can let the first file finish

    report = translate_tree(src, job_name="h", on_tick=tick,
                            on_progress=lambda d, t, item: events.append(("progress", d, t)), **kw)
    progress = [e for e in events if e[0] == "progress"]
    assert progress == [("progress", d, 3) for d in range(4)]  # (0, total), then once per finished file
    assert events.index(("tick", 0, 3)) < events.index(("progress", 1, 3))
    assert len(report.written) == 3 and not report.failed and not report.cancelled


def test_batch_heartbeat_interrupt_stops_active_and_queued(tmp_path, monkeypatch) -> None:
    """A Cancel click raised from the heartbeat mid-file: the running file is told to stop and waited
    for, queued files never start and are recorded once each as cancelled, and nothing is still
    running on return."""
    src, kw = _three_files(tmp_path, monkeypatch)
    seen = _slow_docs(monkeypatch, "cancel")

    def tick(done, total):
        assert seen["entered"].wait(10)
        raise _Stop

    report = BatchReport()
    try:
        translate_tree(src, job_name="t", report=report, on_tick=tick, **kw)
        raise AssertionError("the heartbeat's exception must propagate")
    except _Stop:
        pass
    assert report.cancelled and seen["calls"] == 1 and seen["running"] == 0
    assert len(report.started) == 1
    # The running file wrote after cancel (see the race test); the two queued ones are cancelled once each.
    assert sorted(f.rel for f in report.failed) == sorted(r for r in report.planned if r not in report.started)
    assert all(f.error == CANCELLED for f in report.failed)
    assert len(report.written) + len(report.failed) == 3


def test_batch_interrupt_on_first_frame(tmp_path, monkeypatch) -> None:
    """Early Cancel: the (0, total) frame raises. The report says cancelled, not done, and keeps every file."""
    src, kw = _three_files(tmp_path, monkeypatch)
    seen = _slow_docs(monkeypatch, "cancel")

    def progress(d, t, item):
        if d == 0:
            raise _Stop

    report = BatchReport()
    try:
        translate_tree(src, job_name="e", report=report, on_progress=progress, **kw)
        raise AssertionError("the first frame's exception must propagate")
    except _Stop:
        pass
    assert report.cancelled and seen["running"] == 0 and seen["calls"] <= 1
    rels = [i.rel for i in report.written + report.failed]
    assert sorted(rels) == sorted(report.planned) and len(rels) == 3  # each file once
    assert all(f.error == CANCELLED for f in report.failed)


def test_batch_output_written_while_stopping_is_kept(tmp_path, monkeypatch) -> None:
    """F4: a file whose output lands after cancel is set stays in written (not failed), and the
    retry set (the failed files) does not redo it."""
    src, kw = _three_files(tmp_path, monkeypatch)
    _slow_docs(monkeypatch, "cancel")  # each file writes only once the job is cancelled
    cancel = threading.Event()
    report = BatchReport()

    def tick(done, total):
        cancel.set()  # same as a Stop raised here, minus the exception

    translate_tree(src, job_name="w", report=report, cancel=cancel, on_tick=tick, **kw)
    assert report.cancelled
    assert [w.rel for w in report.written] == report.started and len(report.written) == 1
    assert Path(report.written[0].out).read_text(encoding="utf-8") == "[ja] done"
    retry = {f.rel for f in report.failed}
    assert report.written[0].rel not in retry and len(retry) == 2
    assert all(f.error == CANCELLED for f in report.failed)


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
    at.selectbox(key="qb_purpose").set_value("game").run()
    assert at.session_state["purpose"] == "game"

    at.button(key="nav_settings").click().run()
    at.button(key="nav_translate").click().run()
    assert at.session_state["target_lang"] == "ja"
    assert at.selectbox(key="qb_target").value == "ja"
    assert at.selectbox(key="qb_source").value == "en"
    assert at.session_state["purpose"] == "game" and at.selectbox(key="qb_purpose").value == "game"
    saved = json.loads(prefs.read_text(encoding="utf-8"))
    assert saved["target_lang"] == "ja" and saved["source_choice"] == "en"

    # A fresh session starts from the remembered choices.
    at2 = AppTest.from_file(app, default_timeout=60).run()
    assert at2.selectbox(key="qb_target").value == "ja"


def test_glossary_save_only_with_changes(tmp_path, monkeypatch) -> None:
    """Glossary Save is off until the rows differ from the saved file; an edit and a removal of a
    saved term both enable it, and saving writes the file and turns it off again."""
    import src.config as config
    import src.glossary as glossary
    import src.ui_prefs as ui_prefs
    from streamlit.testing.v1 import AppTest

    monkeypatch.setattr(ui_prefs, "prefs_path", lambda: tmp_path / ".sfts-ui.json")
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    monkeypatch.setattr(glossary, "projects_dir", lambda: tmp_path / "projects")
    (tmp_path / "projects").mkdir()
    glossary.save_glossary("default", [("API", "接口")])
    saved = lambda: json.loads((tmp_path / "projects" / "default" / "glossary.json").read_text(encoding="utf-8"))

    app = str(Path(__file__).resolve().parents[1] / "app.py")
    at = AppTest.from_file(app, default_timeout=60)
    at.session_state["page"] = "settings"
    at.session_state["settings_pane"] = "glossary"
    at.run()
    assert not at.exception
    assert at.button(key="glossary_save").disabled  # unchanged

    n = at.session_state["glossary_nonce"]
    at.text_input(key=f"gtr_{n}_0").set_value("  介面 ").run()  # edited (normalized on compare)
    assert not at.button(key="glossary_save").disabled
    at.button(key="glossary_save").click().run()
    assert saved() == [{"term": "API", "translation": "介面"}]
    assert at.button(key="glossary_save").disabled

    n = at.session_state["glossary_nonce"]
    at.button(key=f"gdel_{n}_0").click().run()  # removing a saved term is a change too
    assert not at.button(key="glossary_save").disabled
    at.button(key="glossary_save").click().run()
    assert saved() == [] and at.button(key="glossary_save").disabled


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
    (tmp_path / "notes.en.txt").write_text("older result", encoding="utf-8")  # forces the timestamp suffix
    app = str(Path(__file__).resolve().parents[1] / "app.py")
    at = AppTest.from_file(app, default_timeout=60)
    at.session_state["source_type"] = "file"
    at.session_state["picked_name"] = "notes.txt"
    at.session_state["picked_bytes"] = b"hello"
    at.session_state["picked_size"] = 5
    at.run()
    assert at.button(key="start_translate").proto.type == "primary"
    assert any("Switching clears" in m.value for m in at.markdown)  # a selected, idle file
    at.button(key="start_translate").click().run()
    assert at.session_state["result"] and not at.exception
    assert at.button(key="start_translate").proto.type == "secondary"
    assert at.button(key="open_out_single").label
    real = Path(at.session_state["result"]["path"]).name
    assert real != "notes.en.txt" and any(f"<code>{real}</code>" in m.value for m in at.markdown)  # Saves as = real name
    assert not any("Switching clears" in m.value for m in at.markdown)  # hidden once a result shows

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


def test_unavailable_choice_and_safe_language_filename(tmp_path, monkeypatch) -> None:
    """A failed probe preserves the explicit choice; a free-text target cannot escape outputs."""
    import src.ui_prefs as prefs
    from streamlit.testing.v1 import AppTest
    monkeypatch.setattr(prefs, "prefs_path", lambda: tmp_path / "prefs.json")
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    monkeypatch.setattr(config, "outputs_dir", lambda: tmp_path)
    prefs.save_prefs(provider="claude_cli")
    calls = []
    def translate(src, dest, **kw):
        calls.append(Path(dest))
        Path(dest).write_text("translated", encoding="utf-8")
    monkeypatch.setattr(batch, "translate_single_file", translate)
    at = AppTest.from_file(str(Path(__file__).resolve().parents[1] / "app.py"), default_timeout=60)
    at.session_state["picked_name"] = "notes.txt"
    at.session_state["picked_bytes"] = b"hello"
    at.session_state["picked_size"] = 5
    at.run()
    assert not at.exception and at.selectbox(key="qb_provider").value == "claude_cli"
    at.selectbox(key="qb_target").set_value("ja").run()
    assert prefs.load_prefs()["provider"] == "claude_cli"
    at.button(key="start_translate").click().run()
    assert not calls and not at.exception  # another available service never silently replaces the choice
    at.selectbox(key="qb_provider").set_value("demo").run()
    at.selectbox(key="qb_target").set_value("other").run()
    at.text_input(key="qb_other").set_value("x/../../escape:<>?*").run()
    at.button(key="start_translate").click().run()
    assert not at.exception and calls and calls[-1].resolve().parent == tmp_path.resolve()
    assert not any(c in calls[-1].name for c in '/\\:<>?*')


@pytest.mark.parametrize("new_name", ["My Project", "my project"])
def test_created_glossary_uses_canonical_name_and_keeps_existing_terms(tmp_path, monkeypatch, new_name) -> None:
    import src.glossary as glossary
    import src.ui_prefs as prefs
    from streamlit.testing.v1 import AppTest
    monkeypatch.setattr(prefs, "prefs_path", lambda: tmp_path / "prefs.json")
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    monkeypatch.setattr(glossary, "projects_dir", lambda: tmp_path / "projects")
    existing = glossary.ensure_project("My Project").name
    glossary.save_glossary(existing, [("API", "interface")])
    if new_name != "My Project" and not (tmp_path / "projects" / "my_project").is_dir():
        pytest.skip("Case-sensitive filesystems permit a distinct case-only project.")
    at = AppTest.from_file(str(Path(__file__).resolve().parents[1] / "app.py"), default_timeout=60)
    at.session_state["page"] = "settings"
    at.session_state["settings_pane"] = "glossary"
    at.run()
    at.text_input(key="new_project_name").set_value(new_name).run()
    at.button(key="create_project").click().run()
    assert not at.exception and at.session_state["project"] == existing
    assert at.selectbox(key="project_select").value == existing
    assert at.session_state["glossary_pairs"] == [("API", "interface")]

def test_cancel_cleanup_uses_serialized_reruns() -> None:
    """The browser runner must not render a shared report while its old runner is harvesting it."""
    config_path = Path(__file__).resolve().parents[1] / ".streamlit" / "config.toml"
    assert tomllib.loads(config_path.read_text(encoding="utf-8"))["runner"]["fastReruns"] is False


def _batch_app(tmp_path, monkeypatch):
    """An isolated zip selection; fake core functions below interrupt via a real Streamlit rerun."""
    import src.glossary as glossary
    import src.ui_prefs as prefs
    from streamlit.testing.v1 import AppTest

    monkeypatch.setenv("SFTS_DEMO", "1")
    monkeypatch.setattr(prefs, "prefs_path", lambda: tmp_path / "prefs.json")
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    monkeypatch.setattr(glossary, "projects_dir", lambda: tmp_path / "projects")
    at = AppTest.from_file(str(Path(__file__).resolve().parents[1] / "app.py"), default_timeout=60)
    at.session_state["ui_lang"] = "en"
    at.session_state["provider"] = "demo"
    at.session_state["source_type"] = "zip"
    at.session_state["picked_name"] = "fixture.zip"
    at.session_state["picked_bytes"] = b"zip"
    at.session_state["picked_size"] = 3
    return at


def test_cancelled_batch_rerun_shows_harvested_outputs_and_unfinished_files(tmp_path, monkeypatch) -> None:
    """The next render sees the output finalized during unwind, all remaining files, and one retry."""
    import streamlit as st

    out = tmp_path / "output"
    out.mkdir()
    active = {"running": False}

    def interrupted(_zip, _tree, *, report, cancel, on_progress, **_kw):
        active["running"] = True
        report.output_root = str(out)
        report.planned = [f"f{i}.txt" for i in range(8)]
        report.started = ["f0.txt", "f1.txt"]
        report.skipped = [BatchItem(rel="art/logo.png", skipped="unsupported type")]
        first = out / "f0.txt"
        first.write_text("first saved", encoding="utf-8")
        report.written.append(BatchItem(rel=first.name, out=str(first)))
        try:
            on_progress(1, 8, report.written[0])
            st.rerun()  # deterministic interruption: no browser timer or sleep
        finally:
            cancel.set()
            last = out / "f1.txt"  # the core harvests a file already finishing when cancellation lands
            last.write_text("saved during cleanup", encoding="utf-8")
            report.written.append(BatchItem(rel=last.name, out=str(last)))
            report.failed = [BatchItem(rel=rel, error=CANCELLED) for rel in report.planned[2:]]
            report.cancelled = True
            active["running"] = False

    monkeypatch.setattr(batch, "translate_zip", interrupted)
    at = _batch_app(tmp_path, monkeypatch).run()
    at.button(key="start_translate").click().run()
    assert not at.exception and not active["running"]
    report = at.session_state["batch_report"]
    assert report.cancelled and len(report.written) == 2 and len(report.failed) == 6
    assert sorted(i.rel for i in report.written + report.failed) == sorted(report.planned)
    markup = "\n".join(m.value for m in at.markdown)
    assert 'sfts-done-title">Stopped</div>' in markup and "All done" not in markup
    assert 'data-tone="warn"' in markup and "2 saved · 6 not finished · 1 skipped" in markup
    assert "Not finished · 6" in markup and all(rel in markup for rel in report.planned[2:])
    assert at.button(key="retry_failed").label == "Retry 6 files"
    assert "start_translate" not in [b.key for b in at.button]
    with zipfile.ZipFile(io.BytesIO(at.session_state["batch_zip"])) as archive:
        assert sorted(archive.namelist()) == ["f0.txt", "f1.txt"]
        assert archive.read("f1.txt") == b"saved during cleanup"


def test_cancelled_batch_retry_keeps_unreached_errors(tmp_path, monkeypatch) -> None:
    """A stopped retry replaces reached outcomes but does not turn earlier real errors into cancellation."""
    import streamlit as st

    out = tmp_path / "output"
    out.mkdir()
    saved = out / "saved.txt"
    saved.write_text("keep this", encoding="utf-8")
    old_error = "cannot open docx: Package not found at 'C:\\private owner\\Temp\\unreached.docx'"
    previous = BatchReport(
        output_root=str(out), written=[BatchItem(rel=saved.name, out=str(saved))],
        failed=[BatchItem(rel="fixed.txt", error=AUTH_FAIL), BatchItem(rel="still-bad.txt", error="old failure"),
                BatchItem(rel="unreached.docx", error=old_error), BatchItem(rel="waiting.txt", error=CANCELLED)],
        skipped=[BatchItem(rel="art/logo.png", skipped="unsupported type")],
        planned=["saved.txt", "fixed.txt", "still-bad.txt", "unreached.docx", "waiting.txt"], cancelled=True,
    )
    calls = []

    def interrupted(_zip, _tree, *, report, cancel, only, **_kw):
        calls.append(set(only))
        report.output_root = str(out)
        report.planned = sorted(only)
        report.started = ["fixed.txt", "still-bad.txt"]
        fixed = out / "fixed.txt"
        fixed.write_text("fixed", encoding="utf-8")
        report.written = [BatchItem(rel=fixed.name, out=str(fixed))]
        report.skipped = [BatchItem(rel="art/logo.png", skipped="unsupported type")]
        try:
            st.rerun()
        finally:
            cancel.set()
            report.failed = [BatchItem(rel="still-bad.txt", error="[timeout] retry timeout"),
                             BatchItem(rel="unreached.docx", error=CANCELLED), BatchItem(rel="waiting.txt", error=CANCELLED)]
            report.cancelled = True

    monkeypatch.setattr(batch, "translate_zip", interrupted)
    at = _batch_app(tmp_path, monkeypatch)
    at.session_state["batch_report"] = previous
    at.session_state["batch_job"] = {"kind": "zip", "name": "fixture", "bytes": b"zip"}
    at.run()
    at.button(key="retry_failed").click().run()
    assert not at.exception and calls == [{"fixed.txt", "still-bad.txt", "unreached.docx", "waiting.txt"}]
    final = at.session_state["batch_report"]
    assert final.cancelled and sorted(i.rel for i in final.written) == ["fixed.txt", "saved.txt"]
    assert {i.rel: i.error for i in final.failed} == {
        "still-bad.txt": "[timeout] retry timeout", "unreached.docx": old_error, "waiting.txt": CANCELLED,
    }
    assert len(final.skipped) == 1 and saved.read_text(encoding="utf-8") == "keep this"
    assert at.button(key="retry_failed").label == "Retry 3 files"
    assert "start_translate" not in [b.key for b in at.button]
    markup = "\n".join(m.value for m in at.markdown)
    assert "2 failed · 1 not finished" in markup and 'sfts-done-title">Stopped</div>' in markup


def test_batch_reasons_are_plain_details_redacted_and_path_separate(tmp_path, monkeypatch) -> None:
    out = tmp_path / "output"
    out.mkdir()
    saved = out / "saved.txt"
    saved.write_text("saved", encoding="utf-8")
    secret = "sk-" + "x" * 40
    report = BatchReport(
        output_root=str(out), written=[BatchItem(rel=saved.name, out=str(saved))],
        failed=[
            BatchItem(rel="docs/damaged.docx", error="cannot open docx: Package not found at 'C:\\private owner\\AppData\\Local\\Temp\\versora_zip_42\\tree\\damaged file.docx'"),
            BatchItem(rel="other/damaged.docx", error=f"reader rejected '{secret}' in '/tmp/private owner/source document.docx'"),
            BatchItem(rel="online.txt", error=f"[auth] HTTP 401 invalid api key {secret}"),
        ],
    )
    at = _batch_app(tmp_path, monkeypatch)
    at.session_state["batch_report"] = report
    at.session_state["batch_job"] = {"kind": "zip", "name": "fixture", "bytes": b"zip"}
    at.run()
    assert not at.exception and at.button(key="retry_failed").label == "Retry 3 failed"
    assert "start_translate" not in [b.key for b in at.button]
    rows = "\n".join(m.value for m in at.markdown if '<div class="sfts-files">' in m.value)
    assert rows.count("This DOCX file looks damaged or empty.") == 1
    assert "Something went wrong with this file." in rows and "The key was refused." in rows
    assert "docs/damaged.docx" in rows and "other/damaged.docx" in rows
    assert "Package not found" not in rows and "reader rejected" not in rows
    details = "\n".join(m.value for m in at.markdown if 'class="sfts-detail"' in m.value)
    assert "Package not found" in details and "reader rejected" in details and "****" in details
    assert secret not in details and "private owner" not in details and "versora_zip_42" not in details
    assert "AppData" not in details
    head = next(m.value for m in at.markdown if 'sfts-done-title' in m.value and '<div class="sfts-done ' in m.value)
    path_line = next(m.value for m in at.markdown if '<div class="sfts-result-path">' in m.value)
    assert "3 failed" in head and 'data-copy=' not in head and str(out) not in head
    assert f'data-copy="{out}"' in path_line and "Copy path" in path_line
    assert len(at.expander) == 2  # one Details disclosure and the existing folded Saved group


def test_early_batch_cancel_has_no_copy_path(tmp_path, monkeypatch) -> None:
    import streamlit as st

    def interrupted(*_args, **_kw):
        st.rerun()  # before the core sets output_root or plans any files

    monkeypatch.setattr(batch, "translate_zip", interrupted)
    at = _batch_app(tmp_path, monkeypatch).run()
    at.button(key="start_translate").click().run()
    assert not at.exception and at.session_state["batch_report"].cancelled
    assert not at.session_state["batch_report"].output_root
    markup = "\n".join(m.value for m in at.markdown)
    assert 'sfts-done-title">Stopped</div>' in markup and 'data-copy=' not in markup


def test_successful_batch_keeps_translate_again(tmp_path, monkeypatch) -> None:
    out = tmp_path / "output"
    out.mkdir()
    saved = out / "saved.txt"
    saved.write_text("saved", encoding="utf-8")
    at = _batch_app(tmp_path, monkeypatch)
    at.session_state["batch_report"] = BatchReport(output_root=str(out), written=[BatchItem(rel=saved.name, out=str(saved))])
    at.session_state["batch_job"] = {"kind": "zip", "name": "fixture", "bytes": b"zip"}
    at.run()
    assert not at.exception and at.button(key="start_translate").label == "Translate again"
    assert at.button(key="start_translate").proto.type == "secondary"
    assert "retry_failed" not in [b.key for b in at.button]


@pytest.mark.parametrize("name,variant", [("My Project", "my project"), ("Ünicode Project", "ünicode project")])
def test_ensure_project_returns_disk_casing_without_merging_distinct_dirs(tmp_path, monkeypatch, name, variant) -> None:
    import src.glossary as glossary

    root = tmp_path / "projects"
    monkeypatch.setattr(glossary, "projects_dir", lambda: root)
    original = glossary.ensure_project(name)
    glossary.save_glossary(original.name, [("API", "interface")])
    variant_path = root / variant.replace(" ", "_")
    reused = variant_path.is_dir()  # actual filesystem behavior, not a guessed OS label
    created = glossary.ensure_project(variant)
    if reused:
        assert created.samefile(original) and created.name == original.name
        assert glossary.load_glossary(created.name) == [("API", "interface")]
        assert glossary.list_projects() == [original.name]
    else:
        assert not created.samefile(original) and created.name == variant_path.name
        assert glossary.load_glossary(created.name) == []
        assert glossary.load_glossary(original.name) == [("API", "interface")]



def test_zip_error_keeps_source_extension(tmp_path, monkeypatch) -> None:
    """The batch job's output stem must not turn the damaged-zip heading into 'This ? file'."""
    import src.ui_prefs as prefs
    from streamlit.testing.v1 import AppTest
    monkeypatch.setattr(prefs, "prefs_path", lambda: tmp_path / "prefs.json")
    monkeypatch.setattr(config, "_probe_available", lambda: ["demo"])
    monkeypatch.setattr(config, "_available_cache", None)
    def broken(*_args, **_kw):
        raise ValueError("File is not a zip file")
    monkeypatch.setattr(batch, "translate_zip", broken)
    at = AppTest.from_file(str(Path(__file__).resolve().parents[1] / "app.py"), default_timeout=60)
    at.session_state["source_type"] = "zip"
    at.session_state["picked_name"] = "bad.archive.zip"
    at.session_state["picked_bytes"] = b"not a zip"
    at.session_state["picked_size"] = 9
    at.run()
    at.button(key="start_translate").click().run()
    assert not at.exception and at.session_state["batch_error"]["name"] == "bad.archive.zip"
    assert any("This ZIP file looks damaged or empty." in m.value for m in at.markdown)
    assert not any("This ? file" in m.value for m in at.markdown)
