"""CS3 bridge client (stdlib only): locate the exe, probe COM briefly, launch JSX.

Nothing here blocks on the Photoshop GUI. COM probing runs in a thread with a
short timeout because Dispatch() will try to spin up the GUI app; script
launches use Popen without waiting.
"""

from __future__ import annotations

import concurrent.futures
import os
import subprocess
import tempfile
from pathlib import Path

EXE_ENV = "ULTIMA3D_PHOTOSHOP"
DEFAULT_EXE = (
    r"C:\Users\buffb\Desktop\GitHub\Adobe Photoshop CS3 Portable\App\Photoshop.exe"
)

SCRIPT_DIR = Path(__file__).resolve().parent / "scripts"

PRESETS = sorted(p.stem for p in SCRIPT_DIR.glob("*.jsx")) if SCRIPT_DIR.is_dir() else []


def exe_path() -> Path:
    return Path(os.environ.get(EXE_ENV, DEFAULT_EXE))


def exe_exists() -> bool:
    return exe_path().is_file()


def _com_version() -> str | None:
    try:
        import win32com.client  # type: ignore
    except ImportError:
        return None
    try:
        app = win32com.client.Dispatch("Photoshop.Application")
        return str(app.Version)
    except Exception:
        return None


def com_version(timeout: float = 8.0) -> str | None:
    """Best-effort COM probe; None when CS3 is unreachable (portable builds are)."""
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
        fut = pool.submit(_com_version)
        try:
            return fut.result(timeout=timeout)
        except concurrent.futures.TimeoutError:
            return None


def status() -> dict:
    exe = exe_path()
    return {
        "ok": True,
        "exe": str(exe),
        "exe_present": exe.is_file(),
        "com_version": com_version(),
        "scripts": PRESETS,
    }


def run_script(name_or_code: str, timeout: float = 8.0) -> dict:
    """Fire-and-forget a JSX preset (by stem) or inline code via Photoshop.exe.

    Returns the staged script path and whether the GUI was launched; never
    waits for Photoshop to finish, because portable CS3 has no script-done
    signal over this channel.
    """
    exe = exe_path()
    if not exe.is_file():
        raise RuntimeError(f"Photoshop.exe not found at {exe}; set {EXE_ENV}")
    candidate = SCRIPT_DIR / f"{name_or_code}.jsx"
    if candidate.is_file():
        code = candidate.read_text(encoding="utf-8", errors="replace")
        source = f"preset:{candidate.stem}"
    else:
        code = name_or_code
        source = "inline"
    if "app." not in code and "activeDocument" not in code:
        raise ValueError("refusing to launch Photoshop with code that never touches app/activeDocument")
    fd, tmppath = tempfile.mkstemp(prefix="ultima3d_", suffix=".jsx")
    with open(fd, "w", encoding="utf-8") as f:
        f.write(code)
    try:
        subprocess.Popen([str(exe), tmppath])  # noqa: S603 — exe path is user-configured
    except OSError as e:
        raise RuntimeError(f"failed to launch Photoshop.exe: {e}") from e
    return {"launched": True, "source": source, "script": tmppath}
