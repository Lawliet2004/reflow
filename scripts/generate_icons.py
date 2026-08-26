#!/usr/bin/env python3
"""Generate the Reflow app icon set (PNG / ICO / ICNS / SVG) from src/assets/logo_master.png.

Design: Modern Glowing Audio Wave Ribbon 'W' Logo.
Usage:  python scripts/generate_icons.py
"""

import os
import base64
from PIL import Image

ROOT_DIR = os.path.join(os.path.dirname(__file__), "..")
ICONS_DIR = os.path.join(ROOT_DIR, "src-tauri", "icons")
ASSETS_DIR = os.path.join(ROOT_DIR, "src", "assets")
PUBLIC_DIR = os.path.join(ROOT_DIR, "public")
MASTER_PATH = os.path.join(ASSETS_DIR, "logo_master.png")


def main():
    os.makedirs(ICONS_DIR, exist_ok=True)
    os.makedirs(ASSETS_DIR, exist_ok=True)
    os.makedirs(PUBLIC_DIR, exist_ok=True)

    if not os.path.exists(MASTER_PATH):
        raise FileNotFoundError(f"Master logo not found at {MASTER_PATH}")

    master = Image.open(MASTER_PATH).convert("RGBA")

    # 1. Standard Tauri PNGs
    master.resize((512, 512), Image.LANCZOS).save(os.path.join(ICONS_DIR, "icon.png"), "PNG")
    master.resize((32, 32), Image.LANCZOS).save(os.path.join(ICONS_DIR, "32x32.png"), "PNG")
    master.resize((128, 128), Image.LANCZOS).save(os.path.join(ICONS_DIR, "128x128.png"), "PNG")
    master.resize((256, 256), Image.LANCZOS).save(os.path.join(ICONS_DIR, "128x128@2x.png"), "PNG")

    # 2. Windows Store Logos
    for n in [30, 44, 71, 89, 107, 142, 150, 284, 310]:
        master.resize((n, n), Image.LANCZOS).save(os.path.join(ICONS_DIR, f"Square{n}x{n}Logo.png"), "PNG")
    master.resize((50, 50), Image.LANCZOS).save(os.path.join(ICONS_DIR, "StoreLogo.png"), "PNG")

    # 3. Web & Frontend Assets
    master.resize((512, 512), Image.LANCZOS).save(os.path.join(ASSETS_DIR, "logo.png"), "PNG")
    master.resize((512, 512), Image.LANCZOS).save(os.path.join(PUBLIC_DIR, "icon.png"), "PNG")

    # 4. Windows Multi-Resolution ICO
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    ico_images = [master.resize((s, s), Image.LANCZOS) for s in ico_sizes]
    ico_path = os.path.join(ICONS_DIR, "icon.ico")
    ico_images[-1].save(ico_path, format="ICO", sizes=[(s, s) for s in ico_sizes])
    print("Generated:", ico_path)

    # 5. macOS ICNS
    try:
        icns_path = os.path.join(ICONS_DIR, "icon.icns")
        master.resize((512, 512), Image.LANCZOS).save(icns_path, format="ICNS")
        print("Generated:", icns_path)
    except Exception as e:
        print("ICNS warning:", e)

    # 6. SVG wrapper with embedded high-res PNG
    with open(os.path.join(PUBLIC_DIR, "icon.png"), "rb") as f:
        b64 = base64.b64encode(f.read()).decode("utf-8")

    svg_content = f'''<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 512 512" width="100%" height="100%">
  <image width="512" height="512" href="data:image/png;base64,{b64}"/>
</svg>
'''
    with open(os.path.join(PUBLIC_DIR, "icon.svg"), "w", encoding="utf-8") as f:
        f.write(svg_content)
    with open(os.path.join(ASSETS_DIR, "logo.svg"), "w", encoding="utf-8") as f:
        f.write(svg_content)
    print("Generated SVG wrappers in public/ and src/assets/")


if __name__ == "__main__":
    main()
