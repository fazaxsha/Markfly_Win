"""Generates Markfly app icons (PNG + ICO) with pure Python (no PIL).

Design: rounded-square terracotta tile (#cc785c) with a white abstract
butterfly mark, echoing the app's brand colors (see src/style.css).
"""
import struct
import zlib


def rounded_rect_mask(size, radius):
    mask = bytearray(size * size)
    for y in range(size):
        for x in range(size):
            inside = True
            # rounded corners
            for cx, cy in ((radius, radius), (size - 1 - radius, radius),
                           (radius, size - 1 - radius), (size - 1 - radius, size - 1 - radius)):
                pass
            # simple corner circles
            corners = [
                (radius, radius, x < radius and y < radius, radius - x, radius - y),
                (size - 1 - radius, radius, x >= size - 1 - radius and y < radius, x - (size - 1 - radius), radius - y),
                (radius, size - 1 - radius, x < radius and y >= size - 1 - radius, radius - x, y - (size - 1 - radius)),
                (size - 1 - radius, size - 1 - radius, x >= size - 1 - radius and y >= size - 1 - radius, x - (size - 1 - radius), y - (size - 1 - radius)),
            ]
            for _, _, in_corner, dx, dy in corners:
                if in_corner and dx * dx + dy * dy > radius * radius:
                    inside = False
            if inside:
                mask[y * size + x] = 1
    return mask


def ellipse(x, y, cx, cy, rx, ry, angle):
    import math
    ca, sa = math.cos(angle), math.sin(angle)
    dx, dy = x - cx, y - cy
    lx = ca * dx + sa * dy
    ly = -sa * dx + ca * dy
    return (lx / rx) ** 2 + (ly / ry) ** 2 <= 1.0


def render(size):
    import math
    mask = rounded_rect_mask(size, int(size * 0.22))
    bg = (0xCC, 0x78, 0x5C)  # terracotta
    fg = (0xFA, 0xF9, 0xF5)  # warm white
    px = bytearray(size * size * 4)
    c = size / 2.0
    # butterfly wings: two rotated ellipses above/below center + body bar
    wing_rx, wing_ry = size * 0.19, size * 0.30
    for y in range(size):
        for x in range(size):
            i = (y * size + x) * 4
            if not mask[y * size + x]:
                px[i] = px[i + 1] = px[i + 2] = px[i + 3] = 0
                continue
            # background
            r, g, b = bg
            angle = math.radians(38)
            if (ellipse(x, y, c - size * 0.10, c - size * 0.05, wing_rx, wing_ry, angle)
                    or ellipse(x, y, c - size * 0.10, c + size * 0.05, wing_rx, wing_ry, -angle)
                    or ellipse(x, y, c + size * 0.10, c - size * 0.05, wing_rx, wing_ry, -angle)
                    or ellipse(x, y, c + size * 0.10, c + size * 0.05, wing_rx, wing_ry, angle)):
                r, g, b = fg
            # body: vertical capsule
            body_rx, body_ry = size * 0.035, size * 0.22
            if ellipse(x, y, c, c, body_rx, body_ry, 0):
                r, g, b = (0x7A, 0x3B, 0x28)  # dark terracotta body
            px[i] = r
            px[i + 1] = g
            px[i + 2] = b
            px[i + 3] = 255
    return bytes(px)


def png_bytes(size, raw):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    # add filter byte 0 per scanline
    stride = size * 4
    filtered = b"".join(b"\x00" + raw[y * stride:(y + 1) * stride] for y in range(size))
    idat = zlib.compress(filtered, 9)
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def ico_bytes(images):
    # images: list of (size, png_bytes)
    count = len(images)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries = b""
    blob = b""
    for size, data in images:
        dim = size if size < 256 else 0
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        blob += data
        offset += len(data)
    return header + entries + blob


def main():
    import os
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "icons")
    os.makedirs(out, exist_ok=True)
    images = []
    for size in (16, 32, 48, 256):
        png = png_bytes(size, render(size))
        with open(os.path.join(out, f"tmp-{size}.png"), "wb") as fh:
            fh.write(png)
        images.append((size, png))
    for src, dst in (("tmp-32.png", "32x32.png"), ("tmp-128.png", None),
                     ("tmp-256.png", "128x128@2x.png")):
        pass
    # canonical tauri icon set
    mapping = {
        "tmp-16.png": None,
        "tmp-32.png": "32x32.png",
        "tmp-48.png": None,
        "tmp-256.png": "128x128@2x.png",
    }
    # 128x128 needs its own render
    png128 = png_bytes(128, render(128))
    with open(os.path.join(out, "128x128.png"), "wb") as fh:
        fh.write(png128)
    with open(os.path.join(out, "32x32.png"), "wb") as fh:
        fh.write([d for s, d in images if s == 32][0])
    with open(os.path.join(out, "128x128@2x.png"), "wb") as fh:
        fh.write([d for s, d in images if s == 256][0])
    # icon.ico embeds 16/32/48/256
    with open(os.path.join(out, "icon.ico"), "wb") as fh:
        fh.write(ico_bytes(images))
    for name in os.listdir(out):
        if name.startswith("tmp-"):
            os.remove(os.path.join(out, name))
    print("icons written to", out)


if __name__ == "__main__":
    main()
