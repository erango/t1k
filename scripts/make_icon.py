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


def svg(rows):
    assert all(len(r) == 16 for r in rows) and len(rows) == 16
    assert all(r == r[::-1] for r in rows), "the face is mirror-symmetric"
    cell, origin = 40, 192  # 640 px face centered on the 1024 canvas
    rects = []
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch in PALETTE:
                rects.append(
                    f'<rect x="{origin + x * cell}" y="{origin + y * cell}" width="{cell}" height="{cell}" fill="{PALETTE[ch]}"/>'
                )
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#1B1E23"/><stop offset="1" stop-color="#0B0C0E"/>
    </linearGradient>
  </defs>
  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#tile)"/>
  <rect x="104" y="104" width="816" height="816" rx="181" fill="none" stroke="#2A2E35" stroke-width="8"/>
  <g shape-rendering="crispEdges">
    {chr(10).join('    ' + r for r in rects).strip()}
  </g>
</svg>
"""


def main():
    res = Path(__file__).resolve().parent.parent / "resources"
    (res / "icon.svg").write_text(svg(FACE))
    (res / "icon-alert.svg").write_text(svg(ALERT))
    subprocess.run(["rsvg-convert", "-w", "512", "-h", "512", str(res / "icon-alert.svg"), "-o", str(res / "icon-alert.png")], check=True)
    print("wrote resources/icon.svg, icon-alert.svg, icon-alert.png")


if __name__ == "__main__":
    main()
