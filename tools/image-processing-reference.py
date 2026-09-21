#!/usr/bin/env python3
"""Generate 1-bit quantization fixtures using the user's img2idx reference.

Run from the repository root; requires Pillow and NumPy. Resampling is tested
separately because Pillow and Rust image use different Lanczos implementations.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np

path = Path(sys.argv[1] if len(sys.argv) > 1 else
            "../synapdeck-togo/nix/packages/img2idx/img2idx.py")
spec = importlib.util.spec_from_file_location("img2idx", path)
ref = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ref)
w, h = 19, 9
rgba = np.array([
    [(x * 37 + y * 19) % 256, (x * 11 + y * 43) % 256,
     (x * 71 + y * 7) % 256, (x * 53 + y * 29) % 256]
    for y in range(h) for x in range(w)
], dtype=np.uint8).reshape(h, w, 4)
rgb = ref.srgb_to_linear(rgba[:, :, :3].astype(float) / 255)
alpha = rgba[:, :, 3:4].astype(float) / 255
work = (rgb * alpha + (1 - alpha)) @ ref.REC709
levels = np.array([0., 1.])
outputs = {}
for name, kernel in ref.DIFFUSION.items():
    outputs[name] = ref.quantise_diffused(
        work, levels, kernel, ref.DIVISOR.get(name, sum(k[2] for k in kernel)),
        True, 1.0)
m = ref.bayer_matrix(8)
threshold = np.tile((m + .5) / m.size, (2, 3))[:h, :w]
outputs["bayer"] = ref.quantise_ordered(work, levels, threshold)
# Rongta polarity is 1=black, unlike the reference's palette indices.
fixture = dict(reference_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
               width=w, height=h, rgba=rgba.flatten().tolist(),
               rasters={name: ref.pack_bits(1 - idx, 1, True, True).hex()
                        for name, idx in outputs.items()})
Path("rongta-rs/tests/fixtures/dither.json").write_text(json.dumps(fixture, indent=2) + "\n")
