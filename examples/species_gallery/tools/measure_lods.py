# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Measure each LOD's screen coverage and mean colour from `render_lods.py`.

    blender --background --python examples/species_gallery/tools/measure_lods.py -- .local/gallery/species-gallery/oak-seed1

Reads `lods.png` and `lods-small.png`, splits each into one equal column per
level and impostor (the layout `render_lods.py` uses), and prints, per level, the fraction
of the column the tree covers and the mean colour of the covered pixels,
relative to LOD1. A crossfade between two levels pops when either differs
much; the goal is coverage within a few percent and colour within a few
percent per channel. Writes `lods-measure.json` next to the input.
"""

import json
import sys
from pathlib import Path

import bpy
import numpy as np


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: measure_lods.py -- <species_gallery seed dir>")
    return Path(argv[0])


def load(path):
    image = bpy.data.images.load(str(path))
    width, height = image.size
    pixels = np.array(image.pixels[:], dtype=np.float32).reshape(height, width, 4)
    return pixels[:, :, :3]


def measure(pixels, columns):
    # The background is flat; anything clearly off it is tree.
    background = np.median(pixels[:4, :, :].reshape(-1, 3), axis=0)
    covered = np.abs(pixels - background).max(axis=2) > 0.04
    width = pixels.shape[1]
    out = []
    for n in range(columns):
        lo, hi = n * width // columns, (n + 1) * width // columns
        mask = covered[:, lo:hi]
        count = int(mask.sum())
        colour = pixels[:, lo:hi][mask].mean(axis=0) if count else np.zeros(3)
        out.append({"coverage": count / mask.size, "colour": colour.tolist()})
    return out


def main():
    out_dir = args()
    lods = out_dir / "lods"
    columns = (
        len(sorted(lods.glob("lod*-bark.obj")))
        + int((lods / "impostor.obj").exists())
        + int((lods / "octahedral.json").exists())
    )
    report = {}
    for name in ("lods.png", "lods-small.png"):
        levels = measure(load(out_dir / name), columns)
        reference = levels[1] if len(levels) > 1 else levels[0]
        print(f"{name}:")
        for n, level in enumerate(levels):
            cov = level["coverage"] / max(reference["coverage"], 1e-9)
            ratio = [c / max(r, 1e-9) for c, r in zip(level["colour"], reference["colour"])]
            print(
                f"  level {n}: coverage {level['coverage']:.4f} ({cov:.2f}x lod1), "
                f"colour {[round(c, 3) for c in level['colour']]} "
                f"({[round(r, 2) for r in ratio]}x lod1)"
            )
        report[name] = levels
    (out_dir / "lods-measure.json").write_text(json.dumps(report, indent=2))


main()
