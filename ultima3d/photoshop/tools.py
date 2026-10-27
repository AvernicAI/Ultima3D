"""Photoshop CS3 tools registered on the unified Ultima3D server."""

from __future__ import annotations

import json
import os
from pathlib import Path

from ..registry import mcp
from . import bridge
from .textures import PRESETS, make_texture

OUT_DIR = Path(os.environ.get("ULTIMA3D_OUT", Path.cwd() / "output"))


@mcp.tool()
def photoshop_status() -> str:
    """Check the Photoshop CS3 backend: exe presence, COM reachability, JSX presets."""
    try:
        return json.dumps(bridge.status())
    except Exception as e:
        return json.dumps({"ok": False, "error": str(e)})


@mcp.tool()
def photoshop_run_script(script: str) -> str:
    """Run a JSX preset (prep_texture, export_png) or inline ExtendScript in CS3.

    Fire-and-forget: portable CS3 reports no completion signal, so this stages
    the script, launches Photoshop.exe with it, and returns immediately.
    """
    try:
        return json.dumps(bridge.run_script(script))
    except Exception as e:
        return json.dumps({"ok": False, "error": str(e)})


@mcp.tool()
def photoshop_make_texture(preset: str, size: int = 256, seed: int = 0) -> str:
    """Synthesize a tileable texture trio (albedo/rough/normal PNGs) headlessly.

    Stdlib only — Photoshop never opens. Presets: stone, wood_grain, noise.
    Pair with prep_texture/export_png inside CS3 for finishing touches.
    """
    try:
        d = OUT_DIR / "textures"
        r = make_texture(preset, size=size, seed=seed, out_dir=d)
        return json.dumps(r)
    except Exception as e:
        return json.dumps({"ok": False, "error": str(e),
                           "presets": list(PRESETS)})
