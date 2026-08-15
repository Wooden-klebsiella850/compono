#!/usr/bin/env python3
"""Régénère res/compono.ico depuis le PNG source (bureau ou chemin passé en argument)."""
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / "Desktop" / "compono.png"
OUT = ROOT / "res" / "compono.ico"
SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]

img = Image.open(SRC).convert("RGBA")
img.save(OUT, format="ICO", sizes=SIZES)
print(f"icône générée : {OUT} ({len(SIZES)} tailles)")
