"""Photoshop CS3 module test: headless texture synthesis + offline-tolerant bridge."""
import os
import sys

sys.path.insert(0, ".")

from ultima3d.photoshop import bridge
from ultima3d.photoshop.textures import PRESETS, make_texture

failures = []

# 1. status() never raises, reports scripts
try:
    s = bridge.status()
    assert s["ok"] and "exe_present" in s and "scripts" in s, s
    assert "prep_texture" in s["scripts"] and "export_png" in s["scripts"], s["scripts"]
    print(f"  [ok] status exe_present={s['exe_present']} scripts={s['scripts']}")
except Exception as e:
    failures.append(f"status: {e}")

# 2. missing exe -> clean RuntimeError, no hang
os.environ["ULTIMA3D_PHOTOSHOP"] = r"C:\nonexistent\Photoshop.exe"
try:
    bridge.run_script("prep_texture")
    failures.append("run_script with missing exe should raise")
except RuntimeError as e:
    print(f"  [ok] missing-exe raises: {e}")
except Exception as e:
    failures.append(f"run_script wrong error: {e}")

# 3. guard: code that never touches Photoshop is refused without launching
os.environ["ULTIMA3D_PHOTOSHOP"] = sys.executable  # any real file passes the is_file gate
try:
    bridge.run_script("1+1")
    failures.append("run_script should refuse code without app/activeDocument")
except ValueError as e:
    print(f"  [ok] guard refuses inert code: {e}")
except Exception as e:
    failures.append(f"run_script guard wrong error: {e}")
finally:
    del os.environ["ULTIMA3D_PHOTOSHOP"]

# 4. every preset synthesizes a valid trio, deterministically
for preset in PRESETS:
    try:
        r1 = make_texture(preset, size=64, seed=7, out_dir="output/photoshop_test")
        r2 = make_texture(preset, size=64, seed=7, out_dir="output/photoshop_test_rerun")
        assert set(r1["files"]) == {"albedo", "rough", "normal"}, r1["files"]
        for kind, p in r1["files"].items():
            raw = open(p, "rb").read()
            assert raw[:8] == b"\x89PNG\r\n\x1a\n", f"{preset}/{kind}: bad PNG magic"
            raw2 = open(r2["files"][kind], "rb").read()
            assert raw == raw2, f"{preset}/{kind}: same seed gave different bytes"
            assert len(raw) > 1000, f"{preset}/{kind}: suspiciously small PNG"
        print(f"  [ok] {preset:12s} trio ok, deterministic ({len(open(r1['files']['albedo'],'rb').read())}B albedo)")
    except Exception as e:
        failures.append(f"make_texture {preset}: {e}")

# 5. unknown preset is a clean ValueError
try:
    make_texture("velvet", size=64, out_dir="output/photoshop_test")
    failures.append("unknown preset should raise")
except ValueError:
    print("  [ok] unknown preset raises ValueError")

if failures:
    print(f"\nPHOTOSHOP TEST FAILED ({len(failures)}):")
    for f in failures:
        print(f"  - {f}")
    sys.exit(1)
print(f"\nPHOTOSHOP TEST OK ({len(PRESETS)} presets synthesize + bridge offline-tolerant)")
