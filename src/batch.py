"""Folder and zip batch jobs. Local parse only. Never overwrite source files."""

from __future__ import annotations

import os
import re
import threading
import zipfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from .config import outputs_dir
from .extractors import DOCUMENT_SUFFIXES, STRUCTURED_SUFFIXES, SUPPORTED_SUFFIXES, is_supported
from .extractors import extract_text, write_translated
from .extractors.textish import (
    translate_csv,
    translate_html,
    translate_json,
    translate_po,
    translate_srt,
    translate_tsv,
    translate_vtt,
    translate_xliff,
    translate_xlsx,
    translate_yaml,
)
from .game_text import SCRIPT_SUFFIXES, extract_script_literals, replace_script_literals, should_translate_string
from .providers.base import Cancelled, TranslationError
from .translator import translate_document, translate_string_list

SKIP_DIR_NAMES = {
    ".git",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    "node_modules",
    ".streamlit",
}

DEFAULT_MAX_FILES = 400
DEFAULT_MAX_BYTES = 80_000_000
DEFAULT_CONCURRENCY = 2
MIN_CONCURRENCY = 1
MAX_CONCURRENCY = 8


def clamp_concurrency(value) -> int:
    try:
        n = int(value)
    except (TypeError, ValueError):
        return DEFAULT_CONCURRENCY
    return max(MIN_CONCURRENCY, min(MAX_CONCURRENCY, n))


@dataclass
class BatchItem:
    rel: str
    out: str = ""
    skipped: str = ""
    error: str = ""


@dataclass
class BatchReport:
    """Filled in place while the job runs, so a caller holding it sees partial results."""

    written: list[BatchItem] = field(default_factory=list)
    skipped: list[BatchItem] = field(default_factory=list)  # never attempted: type, size, path rules
    failed: list[BatchItem] = field(default_factory=list)  # attempted and errored, or cancelled
    output_root: str = ""
    planned: list[str] = field(default_factory=list)  # rel paths in run order
    cancelled: bool = False


CANCELLED = "cancelled"

# (done, total, item_or_None), called on the caller's thread: once with item=None
# before the first file, then once per finished file (per chunk for a single file).
ProgressFn = Callable[[int, int, "BatchItem | None"], None]


def _max_files() -> int:
    try:
        return max(1, int(os.getenv("MAX_BATCH_FILES", str(DEFAULT_MAX_FILES))))
    except ValueError:
        return DEFAULT_MAX_FILES


def _max_bytes() -> int:
    try:
        return max(1, int(os.getenv("MAX_BATCH_BYTES", str(DEFAULT_MAX_BYTES))))
    except ValueError:
        return DEFAULT_MAX_BYTES


def _english_job_name(kind: str, name: str) -> str:
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name or "").strip("._") or "job"
    if not re.match(r"^[A-Za-z]", safe):
        safe = "job_" + safe
    return f"{kind}_{safe[:40]}"


def resolve_inside(root: Path, path: Path) -> Path:
    """realpath must stay inside root. Blocks symlink escape."""
    root_r = root.resolve()
    real = path.resolve()
    try:
        real.relative_to(root_r)
    except ValueError as e:
        raise PermissionError("Path escapes the chosen folder.") from e
    return real


def stay_inside(root: Path, path: Path) -> bool:
    try:
        resolve_inside(root, path)
        return True
    except (PermissionError, OSError):
        return False


def looks_binary(path: Path) -> bool:
    if path.suffix.lower() in {".docx", ".pdf", ".xlsx", ".zip"}:
        return False
    try:
        chunk = path.read_bytes()[:2048]
    except OSError:
        return True
    return b"\x00" in chunk


def iter_source_files(root: Path) -> list[Path]:
    root_r = root.resolve()
    files: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root_r, followlinks=False):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIR_NAMES and not d.startswith(".")]
        current = Path(dirpath)
        if not stay_inside(root_r, current):
            dirnames[:] = []
            continue
        for name in filenames:
            p = current / name
            if not stay_inside(root_r, p):
                continue
            files.append(p)
    return files


def safe_extract_zip(zip_path: Path, dest: Path) -> tuple[list[Path], list[BatchItem]]:
    dest_r = dest.resolve()
    dest_r.mkdir(parents=True, exist_ok=True)
    extracted: list[Path] = []
    skipped: list[BatchItem] = []
    with zipfile.ZipFile(zip_path) as zf:
        for info in zf.infolist():
            name = info.filename.replace("\\", "/")
            if name.startswith("/") or name.startswith("../") or "/../" in name or name.endswith("/.."):
                skipped.append(BatchItem(rel=name, skipped="unsafe path"))
                continue
            if re.match(r"^[A-Za-z]:", name):
                skipped.append(BatchItem(rel=name, skipped="absolute path"))
                continue
            if ".." in Path(name).parts:
                skipped.append(BatchItem(rel=name, skipped="unsafe path"))
                continue
            target = (dest_r / name).resolve()
            if not stay_inside(dest_r, target):
                skipped.append(BatchItem(rel=name, skipped="zip-slip"))
                continue
            if info.is_dir() or name.endswith("/"):
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(info) as src, open(target, "wb") as out:
                out.write(src.read())
            extracted.append(target)
    return extracted, skipped


def _make_translator(
    target_lang: str,
    source_lang: str | None,
    project: str | None,
    provider_choice: str,
    model: str | None = None,
    cancel: threading.Event | None = None,
    on_chunk: Callable[[int, int], None] | None = None,
):
    def translate(strings: list[str]) -> list[str]:
        return translate_string_list(
            strings,
            target_lang=target_lang,
            source_lang=source_lang,
            project=project,
            provider_choice=provider_choice,
            model=model,
            cancel=cancel,
            on_chunk=on_chunk,
        )

    return translate


def _translate_structured(
    src: Path,
    dest: Path,
    game_mode: bool,
    translate,
) -> None:
    suffix = src.suffix.lower()
    dest.parent.mkdir(parents=True, exist_ok=True)
    if suffix == ".json":
        dest.write_text(translate_json(src, translate, game_mode), encoding="utf-8")
    elif suffix in {".yaml", ".yml"}:
        dest.write_text(translate_yaml(src, translate, game_mode), encoding="utf-8")
    elif suffix == ".csv":
        dest.write_text(translate_csv(src, translate, game_mode), encoding="utf-8")
    elif suffix == ".tsv":
        dest.write_text(translate_tsv(src, translate, game_mode), encoding="utf-8")
    elif suffix in {".po", ".pot"}:
        dest.write_text(translate_po(src, translate, game_mode), encoding="utf-8")
    elif suffix == ".xliff":
        dest.write_text(translate_xliff(src, translate, game_mode), encoding="utf-8")
    elif suffix in {".html", ".htm"}:
        dest.write_text(translate_html(src, translate, game_mode), encoding="utf-8")
    elif suffix == ".srt":
        dest.write_text(translate_srt(src, translate, game_mode), encoding="utf-8")
    elif suffix == ".vtt":
        dest.write_text(translate_vtt(src, translate, game_mode), encoding="utf-8")
    elif suffix == ".xlsx":
        translate_xlsx(src, dest, translate, game_mode)
    else:
        raise ValueError(f"unsupported structured type: {suffix}")


def _translate_script(src: Path, dest: Path, translate) -> None:
    text = src.read_text(encoding="utf-8", errors="replace")
    lits = extract_script_literals(text)
    inners = [inner for _a, _b, inner in lits if should_translate_string(inner, True)]
    mapping = dict(zip(inners, translate(inners))) if inners else {}
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(replace_script_literals(text, mapping), encoding="utf-8")


def _translate_document(src: Path, dest: Path, game_mode: bool, **kw) -> None:
    text = extract_text(src)
    if game_mode and src.suffix.lower() in {".txt", ".md", ".markdown"}:
        lines = text.splitlines()
        picks = [ln for ln in lines if should_translate_string(ln, True)]
        mapped = dict(zip(picks, _make_translator(**kw)(picks))) if picks else {}
        new = "\n".join(mapped.get(ln, ln) for ln in lines)
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(new + ("\n" if text.endswith("\n") else ""), encoding="utf-8")
        return
    translated, _n = translate_document(text=text, **kw)
    write_translated(src, translated, dest)


def translate_tree(
    source_root: Path,
    *,
    target_lang: str,
    source_lang: str | None,
    project: str | None,
    provider_choice: str,
    game_mode: bool,
    job_kind: str = "folder",
    job_name: str = "batch",
    model: str | None = None,
    concurrency: int = DEFAULT_CONCURRENCY,
    report: BatchReport | None = None,
    on_progress: ProgressFn | None = None,
    cancel: threading.Event | None = None,
    only: set[str] | None = None,
) -> BatchReport:
    """``only`` limits the run to those rel paths (retry). Setting ``cancel`` drops queued
    files and stops running ones at their next chunk. If ``on_progress`` raises (Streamlit
    interrupts the script on a Cancel click) the same cleanup runs, then it propagates."""
    source_root = source_root.resolve()
    if not source_root.is_dir():
        raise TranslationError("That folder was not found.")

    out_root = (outputs_dir() / _english_job_name(job_kind, job_name)).resolve()
    out_root.mkdir(parents=True, exist_ok=True)
    report = report if report is not None else BatchReport()
    report.output_root = str(out_root)
    cancel = cancel or threading.Event()
    workers = clamp_concurrency(concurrency)
    kw = dict(
        target_lang=target_lang,
        source_lang=source_lang,
        project=project,
        provider_choice=provider_choice,
        model=model,
        cancel=cancel,
    )

    files = iter_source_files(source_root)
    if len(files) > _max_files():
        raise TranslationError(f"Too many files (max {_max_files()}).")
    total = 0
    for p in files:
        try:
            total += p.stat().st_size
        except OSError:
            continue
    if total > _max_bytes():
        raise TranslationError("Folder is larger than the size cap.")

    jobs: list[tuple[Path, Path, str]] = []
    for src in files:
        rel = str(src.relative_to(source_root)).replace("\\", "/")
        if only is not None and rel not in only:
            continue
        if not stay_inside(source_root, src):
            report.skipped.append(BatchItem(rel=rel, skipped="outside folder"))
            continue
        suffix = src.suffix.lower()
        if suffix == ".zip":
            report.skipped.append(BatchItem(rel=rel, skipped="nested zip"))
            continue
        if not is_supported(src.name) and suffix not in SCRIPT_SUFFIXES:
            report.skipped.append(BatchItem(rel=rel, skipped="unsupported type"))
            continue
        if looks_binary(src):
            report.skipped.append(BatchItem(rel=rel, skipped="binary"))
            continue
        dest = (out_root / rel).resolve()
        if not stay_inside(out_root, dest):
            report.skipped.append(BatchItem(rel=rel, skipped="unsafe output path"))
            continue
        if dest.resolve() == src.resolve():
            report.skipped.append(BatchItem(rel=rel, skipped="refuses to overwrite source"))
            continue
        jobs.append((src, dest, rel))

    def _run_one(src: Path, dest: Path, rel: str) -> tuple[str, BatchItem]:
        if cancel.is_set():
            return "failed", BatchItem(rel=rel, error=CANCELLED)
        translate = _make_translator(target_lang, source_lang, project, provider_choice, model, cancel)
        suffix = src.suffix.lower()
        try:
            if suffix in SCRIPT_SUFFIXES:
                _translate_script(src, dest, translate)
            elif suffix in STRUCTURED_SUFFIXES:
                _translate_structured(src, dest, game_mode, translate)
            elif suffix in DOCUMENT_SUFFIXES:
                _translate_document(src, dest, game_mode, **kw)
            else:
                return "skipped", BatchItem(rel=rel, skipped="unsupported type")
            return "written", BatchItem(rel=rel, out=str(dest))
        except Cancelled:
            return "failed", BatchItem(rel=rel, error=CANCELLED)
        except Exception as e:  # TranslationError, or anything a parser throws
            return "failed", BatchItem(rel=rel, error=str(e) or type(e).__name__)

    report.planned = [rel for _s, _d, rel in jobs]
    if not jobs:
        return report
    total = len(jobs)
    if on_progress:
        on_progress(0, total, None)
    pool = ThreadPoolExecutor(max_workers=workers)
    futs = [pool.submit(_run_one, src, dest, rel) for src, dest, rel in jobs]
    finished: set[str] = set()
    try:
        for fut in as_completed(futs):
            kind, item = fut.result()
            finished.add(item.rel)
            (report.written if kind == "written" else report.failed).append(item)
            if on_progress:
                on_progress(len(finished), total, item)
            if cancel.is_set():
                break
    finally:
        # Cancelled, or the caller's callback raised: drop queued work, let running
        # files stop at their next chunk, and record what never finished.
        if len(finished) < total:
            cancel.set()
            report.cancelled = True
            for rel in report.planned:
                if rel not in finished:
                    report.failed.append(BatchItem(rel=rel, error=CANCELLED))
        pool.shutdown(wait=False, cancel_futures=True)
    return report


def translate_single_file(
    src: Path,
    dest: Path,
    *,
    target_lang: str,
    source_lang: str | None,
    project: str | None,
    provider_choice: str,
    game_mode: bool,
    model: str | None = None,
    concurrency: int = DEFAULT_CONCURRENCY,
    on_progress: ProgressFn | None = None,
) -> None:
    """``on_progress(done, total, None)`` fires per chunk sent to the provider."""
    dest = dest.resolve()
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.resolve() == src.resolve():
        raise TranslationError("Refuses to overwrite the source file.")
    _ = clamp_concurrency(concurrency)
    on_chunk = (lambda done, total: on_progress(done, total, None)) if on_progress else None
    translate = _make_translator(target_lang, source_lang, project, provider_choice, model, on_chunk=on_chunk)
    kw = dict(
        target_lang=target_lang,
        source_lang=source_lang,
        project=project,
        provider_choice=provider_choice,
        model=model,
        on_chunk=on_chunk,
    )
    suffix = src.suffix.lower()
    if suffix in SCRIPT_SUFFIXES:
        _translate_script(src, dest, translate)
    elif suffix in STRUCTURED_SUFFIXES:
        _translate_structured(src, dest, game_mode, translate)
    elif suffix in DOCUMENT_SUFFIXES:
        _translate_document(src, dest, game_mode, **kw)
    else:
        raise TranslationError("Unsupported type.")


def translate_zip(
    zip_path: Path,
    extract_root: Path,
    **kwargs,
) -> BatchReport:
    extract_root = extract_root.resolve()
    extract_root.mkdir(parents=True, exist_ok=True)
    _extracted, zip_skips = safe_extract_zip(zip_path, extract_root)
    report = translate_tree(extract_root, job_kind="archive", **kwargs)
    report.skipped = zip_skips + report.skipped
    return report
