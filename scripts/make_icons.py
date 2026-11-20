"""Draw the app icon: one plate seen face-on, white on true black."""
from PIL import Image, ImageDraw

def icon(size, path, pad=0.0):
    s = size * 4
    im = Image.new("RGB", (s, s), (0, 0, 0))
    d = ImageDraw.Draw(im)
    c = s / 2
    def disc(r, **kw):
        d.ellipse([c - r, c - r, c + r, c + r], **kw)
    r = s * (0.34 - pad)
    disc(r, fill=(236, 236, 240))
    disc(r * 0.80, fill=(24, 24, 26))
    disc(r * 0.70, outline=(80, 80, 86), width=max(2, s // 180))
    disc(r * 0.24, fill=(200, 200, 206))
    disc(r * 0.10, fill=(0, 0, 0))
    im.resize((size, size), Image.LANCZOS).save(path)

if __name__ == "__main__":
    out = "web/static/icons"
    icon(192, f"{out}/icon-192.png")
    icon(512, f"{out}/icon-512.png")
    icon(512, f"{out}/icon-maskable-512.png", pad=0.06)
    icon(180, f"{out}/apple-touch-icon.png")


def favicons(out):
    """Browser tab icons: PNG sizes, a multi-size .ico, and an SVG that stays sharp."""
    from PIL import Image
    icon(512, "/tmp/_plate512.png")
    big = Image.open("/tmp/_plate512.png")
    for size in (16, 32, 48):
        big.resize((size, size), Image.LANCZOS).save(f"{out}/favicon-{size}.png")
    big.save(f"{out}/../favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])
    svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
<rect width="64" height="64" rx="14" fill="#000"/>
<circle cx="32" cy="32" r="21.8" fill="#ececf0"/>
<circle cx="32" cy="32" r="17.4" fill="#18181a"/>
<circle cx="32" cy="32" r="15.2" fill="none" stroke="#505056" stroke-width="1"/>
<circle cx="32" cy="32" r="5.2" fill="#c8c8ce"/>
<circle cx="32" cy="32" r="2.2" fill="#000"/>
</svg>
"""
    open(f"{out}/../favicon.svg", "w").write(svg)


if __name__ == "__main__":
    favicons("web/static/icons")
