"""Значки программы из векторного знака.

`tauri icon` делает все размеры из одного рисунка, но в 16 и 24 px кольцо и хвостик облачка
пропадают. Поэтому эти два размера в icon.ico берём из отдельного, более жирного рисунка.

    python scripts/make-icons.py

Нужны Node (npx tauri) и Pillow.
"""
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ICONS = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
SMALL = (16, 24)


def tauri_icon(svg: Path, out: Path) -> None:
    subprocess.run(["npx", "tauri", "icon", str(svg), "-o", str(out)], check=True, shell=True)
    # Программа только для Windows — мобильные наборы не нужны.
    for mobile in ("android", "ios"):
        shutil.rmtree(out / mobile, ignore_errors=True)


def frames(ico: Path) -> dict[int, Image.Image]:
    im = Image.open(ico)
    out = {}
    for size in im.info["sizes"]:
        im.size = size
        out[size[0]] = im.convert("RGBA").copy()
    return out


def main() -> None:
    tauri_icon(ICONS / "app-icon.svg", ICONS)
    with tempfile.TemporaryDirectory() as tmp:
        tauri_icon(ICONS / "app-icon-small.svg", Path(tmp))
        small = frames(Path(tmp) / "icon.ico")
    use = frames(ICONS / "icon.ico") | {s: small[s] for s in SMALL}
    sizes = sorted(use)
    use[sizes[-1]].save(ICONS / "icon.ico", sizes=[(s, s) for s in sizes], append_images=[use[s] for s in sizes[:-1]])
    print("icon.ico:", sorted(frames(ICONS / "icon.ico")))


if __name__ == "__main__":
    main()
