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

# Icon. With Xcode, compile resources/T1K.icon (Icon Composer) into Assets.car + T1K.icns:
# macOS 26 draws only compiled icons at full size and shrinks a bare .icns onto a platter.
# Without Xcode, fall back to an .icns from the SVG.
if xcrun --find actool >/dev/null 2>&1; then
  ICONOUT="$PWD/target/release/icon-out"
  rm -rf "$ICONOUT" && mkdir -p "$ICONOUT"
  # actool resolves relative paths oddly; give it absolute ones.
  xcrun actool "$PWD/resources/T1K.icon" --compile "$ICONOUT" --app-icon T1K --enable-on-demand-resources NO \
    --development-region en --target-device mac --platform macosx --minimum-deployment-target 13.0 \
    --output-partial-info-plist "$ICONOUT/partial.plist" >/dev/null
  cp "$ICONOUT/Assets.car" "$ICONOUT/T1K.icns" "$APP/Contents/Resources/"
else
  ICONSET=target/release/T1K.iconset
  rm -rf "$ICONSET" && mkdir -p "$ICONSET"
  for s in 16 32 128 256 512; do
    rsvg-convert -w $s -h $s resources/icon.svg -o "$ICONSET/icon_${s}x${s}.png"
    rsvg-convert -w $((s*2)) -h $((s*2)) resources/icon.svg -o "$ICONSET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/T1K.icns"
fi

# Sparkle (auto-update), pinned and cached in target/sparkle. T1K loads it at runtime and
# draws its own update UI (crates/t1k-app/src/updater.rs).
SPARKLE_VERSION=2.10.1
SPARKLE=target/sparkle/$SPARKLE_VERSION
if [ ! -d "$SPARKLE/Sparkle.framework" ]; then
  mkdir -p "$SPARKLE"
  curl -fsSL "https://github.com/sparkle-project/Sparkle/releases/download/$SPARKLE_VERSION/Sparkle-$SPARKLE_VERSION.tar.xz" | tar xJ -C "$SPARKLE"
fi
mkdir -p "$APP/Contents/Frameworks"
ditto "$SPARKLE/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"
# Public half of the EdDSA key that signs updates. The private half is in the login
# keychain (Sparkle's generate_keys --account t1k); scripts/release.sh signs with it.
SPARKLE_PUBLIC_KEY="W1hctyZqC/ipZHQ0ZZ2YltMU7aJ2BqUaZsu/GkHB368="
FEED_URL="https://raw.githubusercontent.com/erango/t1k/main/appcast.xml"

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
  <key>CFBundleIconName</key><string>T1K</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>SUFeedURL</key><string>$FEED_URL</string>
  <key>SUPublicEDKey</key><string>$SPARKLE_PUBLIC_KEY</string>
  <key>SUEnableAutomaticChecks</key><true/>
  <key>SUScheduledCheckInterval</key><integer>86400</integer>
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
