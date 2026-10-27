"""Self-update support for the portable LAMZU Battery executable.

The updater talks only to the official GitHub repository, downloads the
release asset, verifies the SHA-256 digest published by GitHub, replaces the
running executable in its current directory and starts the new version.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import re
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen

API_URL = (
    "https://api.github.com/repos/lavandiI/lamzu-battery/releases/latest"
)
ASSET_NAME = "LamzuBattery.exe"
USER_AGENT = "LamzuBattery"
MAX_UPDATE_BYTES = 200 * 1024 * 1024
MAX_JSON_BYTES = 1024 * 1024
CHUNK_SIZE = 256 * 1024

_VERSION_RE = re.compile(r"(\d+(?:\.\d+)*)")
_SHA256_RE = re.compile(r"[0-9a-fA-F]{64}")

log = logging.getLogger(__name__)


class UpdateError(RuntimeError):
    """Raised when an update cannot be checked, downloaded or installed."""


@dataclass(frozen=True)
class UpdateInfo:
    version: str
    tag: str
    asset_url: str
    asset_size: int
    digest: str
    page_url: str = ""


def parse_version(value: str) -> tuple[int, ...]:
    """Extract a dotted numeric version from a tag or a display string."""
    match = _VERSION_RE.search(value or "")
    if match is None:
        return ()
    return tuple(int(part) for part in match.group(1).split("."))


def is_newer(candidate: str, current: str) -> bool:
    """Return True when candidate is a newer dotted version than current."""
    candidate_parts = parse_version(candidate)
    current_parts = parse_version(current)
    if not candidate_parts or not current_parts:
        return False
    width = max(len(candidate_parts), len(current_parts))
    left = candidate_parts + (0,) * (width - len(candidate_parts))
    right = current_parts + (0,) * (width - len(current_parts))
    return left > right


def executable_path() -> Optional[Path]:
    """Absolute path of the running exe, or None when running from source.

    The path is taken from ``sys.executable`` and resolved, so the updater
    follows the exe wherever the user moved it instead of assuming ``dist``.
    """
    if not getattr(sys, "frozen", False):
        return None
    try:
        return Path(sys.executable).resolve()
    except OSError:
        return Path(sys.executable)


def update_paths(target: Path) -> tuple[Path, Path]:
    """Return the download and backup paths next to the target exe."""
    target = Path(target)
    stem = target.stem or target.name
    suffix = target.suffix
    return (
        target.with_name(f"{stem}.new{suffix}"),
        target.with_name(f"{stem}.old{suffix}"),
    )


def _normalize_digest(value) -> Optional[str]:
    text = str(value or "").strip()
    if text.lower().startswith("sha256:"):
        text = text[7:]
    if _SHA256_RE.fullmatch(text) is None:
        return None
    return text.lower()


def _is_trusted_asset_url(url: str) -> bool:
    try:
        parsed = urlparse(url)
    except ValueError:
        return False
    if parsed.scheme != "https":
        return False
    host = (parsed.hostname or "").lower()
    if host not in {"github.com", "www.github.com"}:
        return False
    return parsed.path.startswith(
        "/lavandiI/lamzu-battery/releases/download/"
    )


def _is_trusted_download_url(url: str) -> bool:
    try:
        parsed = urlparse(url)
    except ValueError:
        return False
    if parsed.scheme != "https":
        return False
    host = (parsed.hostname or "").lower()
    if host in {"github.com", "www.github.com"}:
        return _is_trusted_asset_url(url)
    # GitHub redirects release assets to objects.githubusercontent.com or
    # release-assets.githubusercontent.com.
    return host == "githubusercontent.com" or host.endswith(
        ".githubusercontent.com"
    )


def _open_url(url: str, timeout: float, accept: str):
    headers = {
        "User-Agent": USER_AGENT,
        "Accept": accept,
    }
    return urlopen(Request(url, headers=headers), timeout=timeout)


def check_for_update(
    current_version: str,
    api_url: str = API_URL,
    timeout: float = 10.0,
) -> Optional[UpdateInfo]:
    """Return the latest release when it is newer than current_version."""
    try:
        response = _open_url(
            api_url, timeout, "application/vnd.github+json"
        )
        with response:
            raw = response.read(MAX_JSON_BYTES)
    except (HTTPError, URLError, TimeoutError, OSError) as exc:
        raise UpdateError(f"update check failed: {exc}") from exc

    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError, TypeError) as exc:
        raise UpdateError("invalid update response") from exc
    if not isinstance(payload, dict):
        raise UpdateError("invalid update response")

    tag = str(payload.get("tag_name") or "")
    if not is_newer(tag, current_version):
        return None

    assets = payload.get("assets")
    if not isinstance(assets, list):
        raise UpdateError("release has no assets")
    asset = next(
        (
            item
            for item in assets
            if isinstance(item, dict) and item.get("name") == ASSET_NAME
        ),
        None,
    )
    if asset is None:
        raise UpdateError(f"release has no {ASSET_NAME}")

    asset_url = str(asset.get("browser_download_url") or "")
    if not _is_trusted_asset_url(asset_url):
        raise UpdateError("untrusted update URL")

    digest = _normalize_digest(asset.get("digest"))
    if digest is None:
        raise UpdateError("release asset has no SHA-256 digest")

    try:
        asset_size = int(asset.get("size") or 0)
    except (TypeError, ValueError):
        asset_size = 0
    if asset_size <= 0 or asset_size > MAX_UPDATE_BYTES:
        raise UpdateError("invalid update size")

    version = tag[1:] if tag.lower().startswith("v") else tag
    return UpdateInfo(
        version=version,
        tag=tag,
        asset_url=asset_url,
        asset_size=asset_size,
        digest=digest,
        page_url=str(payload.get("html_url") or ""),
    )


def download_update(
    info: UpdateInfo,
    destination: Path,
    timeout: float = 120.0,
) -> Path:
    """Download the release asset, verify size/SHA-256 and save it."""
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    part = destination.with_name(destination.name + ".part")
    try:
        part.unlink(missing_ok=True)
    except OSError as exc:
        raise UpdateError(f"could not remove stale download: {exc}") from exc

    hasher = hashlib.sha256()
    total = 0
    try:
        response = _open_url(
            info.asset_url, timeout, "application/octet-stream"
        )
        with response:
            final_url = str(response.geturl() or "")
            if not _is_trusted_download_url(final_url):
                raise UpdateError("untrusted download redirect")
            with part.open("wb") as handle:
                while True:
                    chunk = response.read(CHUNK_SIZE)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > MAX_UPDATE_BYTES:
                        raise UpdateError("update is too large")
                    handle.write(chunk)
                    hasher.update(chunk)
    except Exception:
        try:
            part.unlink(missing_ok=True)
        except OSError:
            pass
        raise

    if total != info.asset_size:
        try:
            part.unlink(missing_ok=True)
        except OSError:
            pass
        raise UpdateError(
            f"update size mismatch: got {total}, expected {info.asset_size}"
        )
    digest = hasher.hexdigest()
    if digest != info.digest:
        try:
            part.unlink(missing_ok=True)
        except OSError:
            pass
        raise UpdateError(
            f"update checksum mismatch: got {digest}, expected {info.digest}"
        )
    with part.open("rb") as handle:
        if handle.read(2) != b"MZ":
            try:
                part.unlink(missing_ok=True)
            except OSError:
                pass
            raise UpdateError("downloaded file is not a Windows executable")
    try:
        os.replace(part, destination)
    except OSError as exc:
        try:
            part.unlink(missing_ok=True)
        except OSError:
            pass
        raise UpdateError(f"could not save update: {exc}") from exc
    return destination


def replace_executable(
    target: Path,
    new_path: Path,
    backup_path: Optional[Path] = None,
) -> Path:
    """Move the running exe aside and put the downloaded exe in its place.

    Windows allows renaming a running executable, so the current process can
    keep running from the backup path while the new file takes over the
    original name.
    """
    target = Path(target)
    new_path = Path(new_path)
    if backup_path is None:
        _, backup_path = update_paths(target)
    backup_path = Path(backup_path)

    if not target.is_file():
        raise UpdateError(f"current exe not found: {target}")
    if not new_path.is_file():
        raise UpdateError(f"downloaded exe not found: {new_path}")

    try:
        if backup_path.exists():
            backup_path.unlink()
        os.replace(target, backup_path)
    except OSError as exc:
        raise UpdateError(f"could not move current exe: {exc}") from exc

    try:
        os.replace(new_path, target)
    except OSError as exc:
        try:
            os.replace(backup_path, target)
        except OSError:
            log.exception("could not restore the original exe")
        raise UpdateError(f"could not install update: {exc}") from exc
    return backup_path


def rollback_executable(
    target: Path,
    backup_path: Path,
    new_path: Path,
) -> bool:
    """Restore the old exe when the new process could not be started."""
    target = Path(target)
    backup_path = Path(backup_path)
    new_path = Path(new_path)
    if not target.is_file() or not backup_path.is_file():
        return False
    try:
        os.replace(target, new_path)
        os.replace(backup_path, target)
        return True
    except OSError:
        log.exception("could not roll back the update")
        return False


def cleanup_update_files(
    target: Path,
    attempts: int = 20,
    delay: float = 0.25,
) -> None:
    """Remove stale download/backup files after a successful update."""
    target = Path(target)
    new_path, backup_path = update_paths(target)
    part_path = new_path.with_name(new_path.name + ".part")
    for path in (part_path, new_path, backup_path):
        for attempt in range(attempts):
            if not path.exists():
                break
            try:
                path.unlink()
                break
            except OSError:
                if attempt == attempts - 1:
                    log.debug("could not remove update file %s", path)
                    break
                time.sleep(delay)
