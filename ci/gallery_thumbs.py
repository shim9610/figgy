#!/usr/bin/env python3
"""Write the gallery thumbnails the README shows (#250).

Every gallery image in `crates/renderer/assets` (`gallery-*`, `concept-*` and
`style-*`) gets a copy in `crates/renderer/assets/thumbs/` scaled to
`WIDTH` pixels wide, keeping its aspect ratio. A GIF keeps its frames and
their durations, all frames on one
128-colour palette taken from the first. Run it after regenerating an image:

    python3 ci/gallery_thumbs.py

Requires Pillow (`pip install pillow`).
"""

from pathlib import Path

from PIL import Image, ImageSequence

WIDTH = 320
ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "crates" / "renderer" / "assets"
THUMBS = ASSETS / "thumbs"
PREFIXES = ("gallery-", "concept-", "style-")


def scaled(image: Image.Image) -> Image.Image:
    height = max(1, round(image.height * WIDTH / image.width))
    return image.resize((WIDTH, height), Image.Resampling.LANCZOS)


def thumbnail(source: Path, target: Path) -> None:
    with Image.open(source) as image:
        if source.suffix == ".gif":
            rgb = [scaled(frame.convert("RGB")) for frame in ImageSequence.Iterator(image)]
            for frame in rgb:
                # The source frames' transparency does not apply to opaque RGB.
                frame.info.clear()
            durations = [frame.info.get("duration", 100) for frame in ImageSequence.Iterator(image)]
            # One palette for every frame, taken from the first.
            first = rgb[0].quantize(colors=128, method=Image.Quantize.MEDIANCUT)
            frames = [first] + [frame.quantize(palette=first) for frame in rgb[1:]]
            first.save(
                target,
                save_all=True,
                append_images=frames[1:],
                duration=durations,
                loop=image.info.get("loop", 0),
                optimize=True,
            )
        else:
            scaled(image.convert("RGBA")).save(target, optimize=True)


def main() -> None:
    THUMBS.mkdir(exist_ok=True)
    sources = sorted(
        path
        for path in ASSETS.iterdir()
        if path.suffix in (".png", ".gif") and path.name.startswith(PREFIXES)
    )
    for source in sources:
        target = THUMBS / source.name
        thumbnail(source, target)
        print(f"{target.relative_to(ROOT)}: {target.stat().st_size // 1024} KiB")


if __name__ == "__main__":
    main()
