"""In-place official-release overlay. Fail closed. Never writes user secrets."""

from __future__ import annotations

import json
import shutil
import tempfile
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

from .github_http import (
    PIN_OWNER,
    PIN_REPO,
    GitHubHostError,
    assert_github_overlay_url,
    github_request,
    is_release_tag,
)

STAMP_NAME = ".sfts-release"
STATE_NAME = ".sfts-update-state"
MAX_FILES = 2000
MAX_BYTES = 80_000_000
DAILY = timedelta(hours=24)

STATUS_UP_TO_DATE = "UP_TO_DATE"
STATUS_UPDATED = "UPDATED"
STATUS_SKIPPED_DAILY = "SKIPPED_DAILY"
STATUS_FAILED = "FAILED"

_PROTECTED_EXACT = {
    ".env",
    ".streamlit/secrets.toml",
    ".sfts-update-state",
}
_PROTECTED_PREFIXES = (
    "projects/",
    "data/outputs/",
    "data/outputs",
    "data/prompts/",  # the user's Custom purpose
    ".venv/",
    "venv/",
    ".git/",
)
_PROTECTED_NAMES = {".venv", "venv", ".git", "projects"}


def app_root() -> Path:
    return Path(__file__).resolve().parent.parent.parent


def read_stamp(root: Path | None = None) -> tuple[str, str]:
    path = (root or app_root()) / STAMP_NAME
    if not path.is_file():
        return "", ""
    lines = [ln.strip() for ln in path.read_text(encoding="utf-8").splitlines() if ln.strip()]
    tag = lines[0] if lines else ""
    sha = lines[1] if len(lines) > 1 else ""
    return tag, sha


def write_stamp(root: Path, tag: str, sha: str) -> None:
    dest = (root / STAMP_NAME).resolve()
    _assert_inside(root, dest)
    dest.write_text(f"{tag}\n{sha}\n", encoding="utf-8")


def _now() -> datetime:
    return datetime.now(timezone.utc)


def read_last_check(root: Path) -> datetime | None:
    path = root / STATE_NAME
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        raw = data.get("lastCheckAt")
        if not raw:
            return None
        return datetime.fromisoformat(raw)
    except (OSError, ValueError, json.JSONDecodeError):
        return None


def write_last_check(root: Path) -> None:
    dest = (root / STATE_NAME).resolve()
    _assert_inside(root, dest)
    dest.write_text(
        json.dumps({"lastCheckAt": _now().isoformat()}, indent=2) + "\n",
        encoding="utf-8",
    )


def is_protected(rel: str) -> bool:
    rel = rel.replace("\\", "/")
    while rel.startswith("./"):
        rel = rel[2:]
    rel = rel.lstrip("/")
    if not rel or rel == ".":
        return True
    if rel in _PROTECTED_EXACT:
        return True
    name = Path(rel).name
    if name.startswith(".env"):
        return True
    if rel == ".streamlit/secrets.toml":
        return True
    parts = rel.split("/")
    if parts and parts[0] in _PROTECTED_NAMES:
        return True
    for prefix in _PROTECTED_PREFIXES:
        if rel == prefix.rstrip("/") or rel.startswith(prefix):
            return True
    return False


def _assert_inside(root: Path, path: Path) -> Path:
    root_r = root.resolve()
    real = path.resolve()
    try:
        real.relative_to(root_r)
    except ValueError as e:
        raise PermissionError("Path escapes the app root.") from e
    return real


def verify_zip_pk(data: bytes) -> None:
    if len(data) < 4 or data[:2] != b"PK":
        raise ValueError("Not a ZIP file.")


def _safe_extract_zip(data: bytes, dest: Path) -> Path:
    dest = dest.resolve()
    dest.mkdir(parents=True, exist_ok=True)
    verify_zip_pk(data)
    if len(data) > MAX_BYTES:
        raise ValueError("Archive is larger than the size cap.")
    tmp_zip = dest / "payload.zip"
    tmp_zip.write_bytes(data)
    extracted = dest / "tree"
    extracted.mkdir()
    count = 0
    total = 0
    with zipfile.ZipFile(tmp_zip) as zf:
        if zf.testzip() is not None:
            raise ValueError("ZIP is corrupt.")
        for info in zf.infolist():
            name = info.filename.replace("\\", "/")
            if name.startswith("/") or ".." in Path(name).parts:
                raise ValueError("Unsafe path in archive.")
            target = (extracted / name).resolve()
            try:
                target.relative_to(extracted)
            except ValueError as e:
                raise ValueError("Zip-slip path rejected.") from e
            if info.is_dir() or name.endswith("/"):
                target.mkdir(parents=True, exist_ok=True)
                continue
            count += 1
            total += info.file_size
            if count > MAX_FILES or total > MAX_BYTES:
                raise ValueError("Archive exceeds file or byte cap.")
            target.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(info) as src, open(target, "wb") as out:
                out.write(src.read())
    kids = [p for p in extracted.iterdir() if p.name != "payload.zip"]
    if len(kids) == 1 and kids[0].is_dir():
        return kids[0]
    return extracted


def _backup_tree(root: Path, backup: Path) -> None:
    backup.mkdir(parents=True, exist_ok=True)
    for src in root.rglob("*"):
        if not src.is_file():
            continue
        rel = src.relative_to(root).as_posix()
        if is_protected(rel):
            continue
        if rel.startswith(".sfts-update-state"):
            continue
        dest = backup / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)


def _restore_tree(root: Path, backup: Path, added: list[str] | None = None) -> None:
    for src in backup.rglob("*"):
        if not src.is_file():
            continue
        rel = src.relative_to(backup).as_posix()
        dest = (root / rel).resolve()
        _assert_inside(root, dest)
        if is_protected(rel):
            continue
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)
    for rel in added or []:
        if is_protected(rel):
            continue
        dest = (root / rel).resolve()
        _assert_inside(root, dest)
        if dest.is_file():
            dest.unlink()


def apply_overlay_from_dir(root: Path, source: Path, tag: str, sha: str) -> None:
    """Copy allowlisted files from source onto root. Restores backup on failure."""
    root = root.resolve()
    source = source.resolve()
    backup = Path(tempfile.mkdtemp(prefix="sfts_overlay_backup_"))
    added: list[str] = []
    try:
        _backup_tree(root, backup)
        for src in source.rglob("*"):
            if not src.is_file():
                continue
            rel = src.relative_to(source).as_posix()
            if is_protected(rel):
                continue
            dest = (root / rel).resolve()
            _assert_inside(root, dest)
            is_new = not dest.exists()
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dest)
            if is_new:
                added.append(rel)
        write_stamp(root, tag, sha)
    except Exception:
        _restore_tree(root, backup, added)
        raise
    finally:
        shutil.rmtree(backup, ignore_errors=True)


def _latest_official_release() -> tuple[str, str]:
    url = f"https://api.github.com/repos/{PIN_OWNER}/{PIN_REPO}/releases"
    resp = github_request("GET", url, timeout=30.0)
    resp.raise_for_status()
    items = resp.json()
    if not isinstance(items, list):
        raise GitHubHostError("Unexpected releases payload.")
    for item in items:
        if not isinstance(item, dict):
            continue
        if item.get("draft") or item.get("prerelease"):
            continue
        tag = str(item.get("tag_name") or "").strip()
        if not is_release_tag(tag):
            continue
        sha = _tag_commit_sha(tag)
        return tag, sha
    raise GitHubHostError("No official release tag found.")


def _tag_commit_sha(tag: str) -> str:
    url = f"https://api.github.com/repos/{PIN_OWNER}/{PIN_REPO}/commits/{tag}"
    resp = github_request("GET", url, expected_tag=tag, timeout=30.0)
    resp.raise_for_status()
    data = resp.json()
    sha = str(data.get("sha") or "")
    if len(sha) < 7:
        raise GitHubHostError("Release commit SHA missing.")
    return sha


def _download_tag_zip(tag: str) -> bytes:
    url = f"https://api.github.com/repos/{PIN_OWNER}/{PIN_REPO}/zipball/{tag}"
    assert_github_overlay_url(url, expected_tag=tag)
    resp = github_request("GET", url, expected_tag=tag, timeout=90.0)
    resp.raise_for_status()
    data = resp.content
    verify_zip_pk(data)
    return data


def run_overlay(root: Path | None = None, *, daily: bool = False, apply: bool = True) -> str:
    """
    Shared implementation for start-script daily checks and the manual button.
    Network I/O stays in this local helper — the browser never talks to GitHub.
    """
    root = (root or app_root()).resolve()
    if daily:
        last = read_last_check(root)
        if last and _now() - last < DAILY:
            return STATUS_SKIPPED_DAILY
    try:
        tag, sha = _latest_official_release()
        write_last_check(root)
        cur_tag, cur_sha = read_stamp(root)
        if cur_tag == tag and cur_sha == sha:
            return STATUS_UP_TO_DATE
        if not apply:
            return STATUS_UP_TO_DATE
        extract_base = Path(tempfile.mkdtemp(prefix="sfts_overlay_extract_"))
        if extract_base.resolve() == Path.cwd().resolve() or extract_base.resolve() == root:
            raise RuntimeError("Extract dir must not be cwd or the app root.")
        try:
            data = _download_tag_zip(tag)
            source = _safe_extract_zip(data, extract_base)
            apply_overlay_from_dir(root, source, tag, sha)
        finally:
            shutil.rmtree(extract_base, ignore_errors=True)
        return STATUS_UPDATED
    except Exception:
        return STATUS_FAILED
