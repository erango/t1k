#!/bin/sh
# Build T1K.app in target/release. Usage: scripts/bundle.sh [--install]
#   --install  copy to /Applications and link the CLI to /usr/local/bin/t1k (or ~/.local/bin).
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/release/T1K.app

cargo build --release -p t1k-app

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/t1k "$APP/Contents/MacOS/t1k"

# Icon: SVG -> iconset -> icns.
ICONSET=target/release/T1K.iconset
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  rsvg-convert -w $s -h $s resources/icon.svg -o "$ICONSET/icon_${s}x${s}.png"
  rsvg-convert -w $((s*2)) -h $((s*2)) resources/icon.svg -o "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/T1K.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>T1K</string>
  <key>CFBundleDisplayName</key><string>T1K</string>
  <key>CFBundleIdentifier</key><string>com.erango.t1k</string>
  <key>CFBundleExecutable</key><string>t1k</string>
  <key>CFBundleIconFile</key><string>T1K</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Signing. T1K_SIGN_IDENTITY names a "Developer ID Application: ..." identity in the
# keychain; when unset, the first Developer ID identity found is used. Distribution
# signing needs the hardened runtime and a secure timestamp (both required for
# notarization, see scripts/notarize.sh). T1K_SIGN_IDENTITY=- forces an ad-hoc signature.
IDENTITY=${T1K_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}
if [ -n "$IDENTITY" ] && [ "$IDENTITY" != "-" ]; then
  # Nested code (Sparkle) is signed inside-out before the app itself.
  if [ -d "$APP/Contents/Frameworks/Sparkle.framework" ]; then
    SP="$APP/Contents/Frameworks/Sparkle.framework/Versions/B"
    for c in "$SP/XPCServices/Installer.xpc" "$SP/XPCServices/Downloader.xpc" "$SP/Autoupdate" "$SP/Updater.app"; do
      [ -e "$c" ] && codesign --force --options runtime --timestamp --sign "$IDENTITY" "$c"
    done
    codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP/Contents/Frameworks/Sparkle.framework"
  fi
  codesign --force --options runtime --timestamp \
    --entitlements scripts/T1K.entitlements --sign "$IDENTITY" "$APP"
  codesign --verify --strict --verbose=1 "$APP"
  echo "Signed with $IDENTITY"
else
  codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
  echo "Ad-hoc signed"
fi
echo "Built $APP"

if [ "${1:-}" = "--install" ]; then
  rm -rf /Applications/T1K.app
  cp -R "$APP" /Applications/T1K.app
  BIN=/Applications/T1K.app/Contents/MacOS/t1k
  if [ -w /usr/local/bin ]; then LINK=/usr/local/bin/t1k; else mkdir -p "$HOME/.local/bin"; LINK="$HOME/.local/bin/t1k"; fi
  ln -sf "$BIN" "$LINK"
  echo "Installed /Applications/T1K.app; CLI linked at $LINK"
fi
