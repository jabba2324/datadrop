#!/usr/bin/env python3
"""Draws DataDrop's icon, and writes every size the packages use.

    python3 packaging/make-icon.py

Needs photocraft-cli (https://github.com/storytold/photocraft) to draw it and Pillow to
resize. Writes packaging/icon.png (1024 px; the macOS .icns comes from it),
packaging/icon.ico (Windows) and packaging/icons/<size>.png (Linux, and the window icon).
"""
import json
import os
import subprocess
import tempfile

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
S = 1024
# The macOS icon grid: an 824 px rounded square, 100 px in from each edge.
TILE, RADIUS = [100, 100, 824, 824], 186
LINUX_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def gradient(stops, angle):
    return {"gradient": {"stops": stops, "angle": angle, "style": "linear"}}


def commands():
    """Three stacked database discs on a midnight tile."""
    cmds = []

    def shape(kind, fill, name, **kw):
        cmds.append(("shape.create", {"kind": kind, "fill": fill, "name": name, **kw}))

    def style(*effects):
        cmds.append(("layer.layerStyle.replace",
                     {"effects": [{"kind": k, "params": p} for k, p in effects]}))

    shape("roundedRect", gradient([[0, "#1D2B4A"], [1, "#0B1120"]], -90), "Tile", rect=TILE,
          radii=RADIUS)
    style(("innerShadow", {"color": "#FFFFFF", "opacity": 22, "blend": "screen", "angle": 90,
                           "distance": 3, "size": 2}),
          ("dropShadow", {"color": "#000000", "opacity": 30, "angle": 90, "distance": 14,
                          "size": 28}))
    side = gradient([[0, "#2F8FEA"], [1, "#56B6FF"]], 0)
    cx, w, eh, h, gap = 512, 470, 112, 84, 132
    t0 = 512 - (2 * gap + h + eh) / 2 + eh / 2
    # The bottom disc first, so each one sits on the one below.
    for k in reversed(range(3)):
        top = t0 + k * gap
        shape("rect", side, f"Disc {k + 1} side", rect=[cx - w / 2, top, w, h])
        shape("ellipse", side, f"Disc {k + 1} base", rect=[cx - w / 2, top + h - eh / 2, w, eh])
        shape("ellipse", "#A8DBFF" if k == 0 else "#7FC3FF", f"Disc {k + 1} top",
              rect=[cx - w / 2, top - eh / 2, w, eh])
        if k == 0:
            style(("innerShadow", {"color": "#FFFFFF", "opacity": 35, "blend": "screen",
                                   "angle": -90, "distance": 6, "size": 10}))
    return cmds


def draw(out):
    args = ["photocraft-cli", "run", "--new",
            json.dumps({"width": S, "height": S, "background": "transparent", "name": "DataDrop"})]
    for cid, params in commands():
        args += ["--cmd", cid, "--params", json.dumps(params)]
    args += ["--out", out]
    result = subprocess.run(args, capture_output=True, text=True)
    failed = [line for line in result.stdout.splitlines() if '"ok":false' in line]
    if result.returncode or failed:
        raise SystemExit(f"photocraft-cli failed:\n{result.stderr}{''.join(failed)}")


def main():
    with tempfile.TemporaryDirectory() as tmp:
        master = os.path.join(tmp, "icon.png")
        draw(master)
        icon = Image.open(master).convert("RGBA")
    icon.save(os.path.join(HERE, "icon.png"), optimize=True)
    os.makedirs(os.path.join(HERE, "icons"), exist_ok=True)
    for size in LINUX_SIZES:
        icon.resize((size, size), Image.LANCZOS).save(
            os.path.join(HERE, "icons", f"{size}.png"), optimize=True)
    icon.save(os.path.join(HERE, "icon.ico"), sizes=[(s, s) for s in ICO_SIZES])
    print("Wrote icon.png, icon.ico and icons/")


if __name__ == "__main__":
    main()
