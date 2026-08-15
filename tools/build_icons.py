from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "assets" / "pacord-icon-master.png"
OUT = ROOT / "assets" / "icons" / "hicolor"
SIZES = (16, 22, 24, 32, 48, 64, 96, 128, 256, 512)

image = Image.open(SOURCE).convert("RGBA")
alpha = image.getchannel("A")
bbox = alpha.getbbox()
if bbox is None:
    raise RuntimeError("o ícone não possui área visível")

# Preserve a small transparent margin so the symbol does not touch the launcher edge.
left, top, right, bottom = bbox
width = right - left
height = bottom - top
margin = max(width, height) // 18
left = max(0, left - margin)
top = max(0, top - margin)
right = min(image.width, right + margin)
bottom = min(image.height, bottom + margin)
cropped = image.crop((left, top, right, bottom))
side = max(cropped.width, cropped.height)
canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
canvas.alpha_composite(cropped, ((side - cropped.width) // 2, (side - cropped.height) // 2))

for size in SIZES:
    destination = OUT / f"{size}x{size}" / "apps" / "org.pacord.PACORD.png"
    destination.parent.mkdir(parents=True, exist_ok=True)
    resized = canvas.resize((size, size), Image.Resampling.LANCZOS)
    resized.save(destination, format="PNG", optimize=True)

master = ROOT / "assets" / "pacord-icon-master.png"
master.parent.mkdir(parents=True, exist_ok=True)
canvas.save(master, format="PNG", optimize=True)
print(f"generated {len(SIZES)} KDE icon sizes from {SOURCE}")
