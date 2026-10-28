"""Draw the FreePort icon: resources/freeport.ico (16-256 px) and resources/freeport.png (256 px).

Development tool only (Pillow); the EXEs embed the generated .ico through build.rs.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
S = 1024  # Master canvas; every size is downsampled from it.


def gradient(size, top, bottom):
    image = Image.new("RGBA", (size, size))
    draw = ImageDraw.Draw(image)
    for y in range(size):
        t = y / (size - 1)
        draw.line([(0, y), (size, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    return image


def master():
    canvas = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([40, 40, S - 40, S - 40], radius=210, fill=255)
    canvas.paste(gradient(S, (37, 99, 235), (13, 148, 136)), (0, 0), mask)
    draw = ImageDraw.Draw(canvas)
    # Folder: tab plus body, slightly left of centre.
    white = (255, 255, 255, 255)
    draw.rounded_rectangle([170, 300, 470, 420], radius=48, fill=white)
    draw.rounded_rectangle([170, 360, 700, 760], radius=64, fill=white)
    # Arrow leaving the folder towards the right edge: the move to another drive.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    arrow = [(470, 500), (700, 500), (700, 400), (880, 560), (700, 720), (700, 620), (470, 620)]
    ImageDraw.Draw(shadow).polygon([(x + 10, y + 14) for x, y in arrow], fill=(15, 23, 42, 90))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(14)))
    draw = ImageDraw.Draw(canvas)
    draw.polygon(arrow, fill=(251, 146, 60, 255))
    return canvas


def main():
    image = master()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [image.resize((n, n), Image.LANCZOS) for n in sizes]
    frames[-1].save(ROOT / "resources/freeport.ico", sizes=[(n, n) for n in sizes], append_images=frames[:-1])
    frames[-1].save(ROOT / "resources/freeport.png", optimize=True)
    print("resources/freeport.ico, resources/freeport.png")


if __name__ == "__main__":
    main()
