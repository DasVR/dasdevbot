#!/usr/bin/env python3
"""Per-frame pixel change for shell-video-match.mjs.

usage: frame-diff.py <dir> <prefix> <width> <height>
Prints JSON: {"prev": [...], "first": [...]} where each entry is the number of
pixels (at width x height, greyscale) that moved by more than THRESH levels
against the previous frame and against frame 0. Video frames carry encoder
noise, so "changed" means more than NOISE pixels, not a byte difference.
"""
import json
import os
import sys

import numpy as np
from PIL import Image

THRESH = 6

def load(path, size):
    img = Image.open(path).convert("L")
    if img.size != size:
        img = img.resize(size, Image.BILINEAR)
    return np.asarray(img, dtype=np.int16)

def main():
    folder, prefix, width, height = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    names = sorted(n for n in os.listdir(folder) if n.startswith(prefix + "-"))
    size = (width, height)
    first = None
    prev = None
    out = {"prev": [], "first": []}
    for name in names:
        cur = load(os.path.join(folder, name), size)
        if first is None:
            first = cur
        out["prev"].append(0 if prev is None else int((np.abs(cur - prev) > THRESH).sum()))
        out["first"].append(int((np.abs(cur - first) > THRESH).sum()))
        prev = cur
    json.dump(out, sys.stdout)

if __name__ == "__main__":
    main()
