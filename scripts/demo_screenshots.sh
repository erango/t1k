#!/bin/sh
# Remakes docs/screenshots from demo mode (T1K_DEMO), plus the social sizes.
#
# Runs the debug build against an isolated HOME (target/demo), in the background so it doesn't
# take focus, and captures each window by id. Needs ImageMagick (`magick`) for the social sizes.
# Usage: scripts/demo_screenshots.sh   (after `cargo build`)
set -eu
cd "$(dirname "$0")/.."
ROOT=$PWD
OUT=$ROOT/docs/screenshots
SOCIAL=$OUT/social
# Short: the agent socket path has a ~104-byte limit.
DEMO_HOME=$ROOT/target/demo
mkdir -p "$OUT" "$SOCIAL" "$DEMO_HOME"
swiftc -O -o "$ROOT/target/window_id" scripts/window_id.swift 2>/dev/null || true

# shot <file> <T1K_DEMO scene> [VAR=value ...]
shot() {
    name=$1 scene=$2
    shift 2
    env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin HOME="$DEMO_HOME" USER="$USER" SHELL=/bin/zsh LANG=en_US.UTF-8 \
        T1K_DEMO="$scene" T1K_DEBUG_BACKGROUND=1 "$@" ./target/debug/t1k >/dev/null 2>&1 &
    pid=$!
    sleep "${WAIT:-4}"
    id=$("$ROOT/target/window_id" "$pid")
    screencapture -x -o -l "$id" "$OUT/$name.png"
    kill "$pid"
    wait "$pid" 2>/dev/null || true
    echo "$name.png"
}

shot hero 1
shot needs-you needs
shot switcher 1 T1K_DEBUG_OVERLAY=switcher T1K_DEBUG_QUERY=pay
shot overview 1 T1K_DEBUG_OVERLAY=overview
shot outputs outputs
shot setup 1 T1K_DEBUG_OVERLAY=setup-3
shot themes 1 T1K_DEBUG_OVERLAY=themes
shot hero-light 1 "T1K_THEME=Mercury Light"

# Social sizes: the window scaled onto a dark backdrop with a margin.
fit() { # fit <source> <width> <height> <margin> <out>
    magick -size "$2x$3" gradient:'#1A1C20-#0B0C0E' \
        \( "$OUT/$1.png" -resize "$(($2 - 2 * $4))x$(($3 - 2 * $4))" \) \
        -gravity center -composite "$SOCIAL/$5"
}
n=1
for s in hero needs-you switcher overview outputs; do
    fit "$s" 1270 760 40 "producthunt-$n-$s-1270x760.png"
    n=$((n + 1))
done
fit hero 1200 630 36 "kofi-hero-1200x630.png"
fit hero-light 1200 630 36 "kofi-hero-light-1200x630.png"
# Full bleed, bigger text: the window's top, cut to 1200:630.
magick "$OUT/hero.png" -resize 1200x -gravity north -crop 1200x630+0+0 +repage "$SOCIAL/kofi-hero-crop-1200x630.png"
cp "$OUT/hero.png" "$SOCIAL/slack-hero.png"
echo "done: $OUT"
