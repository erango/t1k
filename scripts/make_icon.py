#!/usr/bin/env python3
"""Generate T1K's app icons from the 16x16 pixel face.

Writes resources/icon.svg (the app icon: the stern liquid-metal android) and
resources/icon-alert.svg + icon-alert.png (the Dock icon while a tab needs you:
eyes open wide in red). Design: canvas board "Logo · take 4", face A + D3.
Run after editing the grids; bundle.sh turns icon.svg into the .icns.
"""
import subprocess
from pathlib import Path

PALETTE = {
    "w": "#FFFFFF", "a": "#F7F9FC", "b": "#D4DAE2", "c": "#9AA4B2",
    "d": "#6E7886", "e": "#3A414B", "k": "#0B0C0E", "r": "#FF4A4F",
}

FACE = [
    "....cccccccc....",
    "...cbbbbbbbbc...",
    "..cbbbbwwbbbbc..",
    "..cddddddddddc..",
    "..cebbbbbbbbec..",
    "..cbeebbbbeebc..",
    "..cbkkkbbkkkbc..",
    "..ccbbbbbbbbcc..",
    "..ccbbbccbbbcc..",
    "..cdbbbccbbbdc..",
    "...ccbbbbbbcc...",
    "...cbbeeeebbc...",
    "....cebbbbec....",
    ".....cddddc.....",
    "......dddd......",
    "......eeee......",
]

# Needs you: the slits open into red 3x2 blocks.
ALERT = FACE.copy()
ALERT[6] = "..cbrrrbbrrrbc.."
ALERT[7] = "..ccrrrbbrrrcc.."


def face_rects(rows, cell, ox, oy):
    assert all(len(r) == 16 for r in rows) and len(rows) == 16
    assert all(r == r[::-1] for r in rows), "the face is mirror-symmetric"
    out = []
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch in PALETTE:
                out.append(f'<rect x="{ox + x * cell}" y="{oy + y * cell}" width="{cell}" height="{cell}" fill="{PALETTE[ch]}"/>')
    return "\n    ".join(out)


TILE = """<linearGradient id="tile" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1B1E23"/><stop offset="1" stop-color="#0B0C0E"/></linearGradient>"""
CHROME = """<linearGradient id="chrome" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F7F9FC"/><stop offset=".45" stop-color="#B7C0CC"/><stop offset=".58" stop-color="#7E8896"/><stop offset="1" stop-color="#D4DAE2"/></linearGradient>"""


def svg(rows):
    # Tile 884/1024 (70 px margin), matching the visual size of current macOS app icons.
    cell = 44
    origin = (1024 - 16 * cell) // 2
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>{TILE}</defs>
  <rect x="70" y="70" width="884" height="884" rx="198" fill="url(#tile)"/>
  <rect x="74" y="74" width="876" height="876" rx="194" fill="none" stroke="#2A2E35" stroke-width="8"/>
  <g shape-rendering="crispEdges">
    {face_rects(rows, cell, origin, origin)}
  </g>
</svg>
"""


def tile(body, defs=""):
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>{TILE}{defs}</defs>
  <rect x="70" y="70" width="884" height="884" rx="198" fill="url(#tile)"/>
  <rect x="74" y="74" width="876" height="876" rx="194" fill="none" stroke="#2A2E35" stroke-width="8"/>
  {body}
</svg>
"""


# One chrome band across the whole tile, so the chevron and the cursor share a horizon.
CHROME_TILE = """<linearGradient id="chrome" gradientUnits="userSpaceOnUse" x1="0" y1="330" x2="0" y2="700"><stop offset="0" stop-color="#F7F9FC"/><stop offset=".45" stop-color="#B7C0CC"/><stop offset=".58" stop-color="#7E8896"/><stop offset="1" stop-color="#D4DAE2"/></linearGradient>"""


def prompt_svg(alert=False):
    """Alt icon 1: a shell prompt (>_) in the wordmark's chrome. Needs you: the cursor goes red."""
    cursor = "#FF4A4F" if alert else "url(#chrome)"
    return tile(f"""<path d="M314 352 L490 512 L314 672" fill="none" stroke="url(#chrome)" stroke-width="84" stroke-linecap="square" stroke-linejoin="miter"/>
  <rect x="554" y="610" width="200" height="72" fill="{cursor}"/>""", CHROME_TILE)


def wordmark_svg(alert=False):
    """Alt icon 2: the chrome "T1K" wordmark. Needs you: the 1 goes red."""
    one = '<tspan fill="#FF4A4F">1</tspan>' if alert else "1"
    return tile(f"""<text x="512" y="620" text-anchor="middle" font-family="JetBrains Mono" font-weight="800" font-size="320" letter-spacing="6" fill="url(#chrome)">T{one}K</text>""",
                CHROME_TILE.replace('y1="330" x2="0" y2="700"', 'y1="400" x2="0" y2="620"'))


def lockup(width, height, cell, text_px, bg=True, tagline=""):
    """Face + chrome "T1K" wordmark side by side, centered (README header, social preview)."""
    face = 16 * cell
    gap = cell * 3
    text_w = int(text_px * 0.6 * 3 * 1.04)  # JetBrains Mono: 0.6 em per glyph, +4% tracking
    x0 = (width - (face + gap + text_w)) // 2
    y0 = (height - face) // 2
    # bg: True = full dark background; "round" = a dark rounded panel (reads on light pages too).
    back = {True: f'<rect width="{width}" height="{height}" fill="#0B0C0E"/>', "round": f'<rect width="{width}" height="{height}" rx="28" fill="#0B0C0E"/>'}.get(bg, "")
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
  <defs>{CHROME}</defs>
  {back}
  <g shape-rendering="crispEdges">
    {face_rects(FACE, cell, x0, y0)}
  </g>
  <text x="{x0 + face + gap}" y="{y0 + face / 2 + text_px * 0.36}" font-family="JetBrains Mono" font-weight="800" font-size="{text_px}" letter-spacing="{text_px * 0.04}" fill="url(#chrome)">T1K</text>
  {f'<text x="{width / 2}" y="{height - 70}" text-anchor="middle" font-family="Helvetica Neue, Helvetica, Arial" font-size="30" fill="#8C939C">{tagline}</text>' if tagline else ""}
</svg>
"""


def main():
    res = Path(__file__).resolve().parent.parent / "resources"
    (res / "icon.svg").write_text(svg(FACE))
    (res / "icon-alert.svg").write_text(svg(ALERT))
    subprocess.run(["rsvg-convert", "-w", "512", "-h", "512", str(res / "icon-alert.svg"), "-o", str(res / "icon-alert.png")], check=True)
    subprocess.run(["rsvg-convert", "-w", "256", "-h", "256", str(res / "icon.svg"), "-o", str(res / "icon-256.png")], check=True)
    # Alternative app icons (Settings › Appearance › App icon). The Dock shows them while T1K
    # runs; each has a needs-you variant.
    (res / "icons").mkdir(exist_ok=True)
    for name, make in {"prompt": prompt_svg, "wordmark": wordmark_svg}.items():
        for alert in (False, True):
            path = res / "icons" / f"{name}{'-alert' if alert else ''}.svg"
            path.write_text(make(alert))
            subprocess.run(["rsvg-convert", "-w", "512", "-h", "512", str(path), "-o", str(path.with_suffix(".png"))], check=True)
    # Brand images: README header (transparent), GitHub social preview (1280x640), and the
    # lockup the app shows in About / Welcome (transparent).
    (res / "brand").mkdir(exist_ok=True)
    for name, (w, h, cell, text, bg, tag) in {
        "readme-header": (1200, 300, 12, 150, "round", ""),
        "social-preview": (1280, 640, 18, 210, True, "The terminal for running many coding agents at once"),
        "lockup": (720, 200, 8, 104, False, ""),
    }.items():
        path = res / "brand" / f"{name}.svg"
        path.write_text(lockup(w, h, cell, text, bg, tag))
        subprocess.run(["rsvg-convert", "-w", str(w * 2 if name != "social-preview" else w), str(path), "-o", str(path.with_suffix(".png"))], check=True)
    print("wrote resources/icon.svg, icon-alert.*, icon-256.png, icons/*, brand/*")


if __name__ == "__main__":
    main()
