#!/usr/bin/env bash
# Cut a T1K release that Sparkle can install over older copies.
#
#   scripts/release.sh 0.2.0            stable: tag v0.2.0, GitHub release, appcast item
#   scripts/release.sh 0.3.0-beta.1 --beta
#   scripts/release.sh 0.2.0 --local http://localhost:8765
#                                       build, sign and write appcast.xml + zip into
#                                       target/updates/ only (for testing the update flow)
#
# Steps: set the version, build and sign T1K.app (bundle.sh), notarize and staple, zip it,
# sign the zip with the EdDSA key (Sparkle's sign_update, key in the login keychain under
# account "t1k"), add an <item> to appcast.xml, then publish the zip as a GitHub release and
# push appcast.xml to main (the feed the app reads).
#
# Release notes: docs/releases/<version>.md, one "- " bullet per line. They show in the
# update dialog under "What's new".
#
# Needs: T1K_NOTARY_PROFILE (see scripts/notarize.sh) unless --local, and gh logged in.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env.local ] && . ./.env.local

VERSION="${1:?usage: release.sh <version> [--beta] [--local <base url>]}"
shift
CHANNEL=""
LOCAL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --beta) CHANNEL=beta ;;
    --local) LOCAL="${2:?--local needs a base url}"; shift ;;
    *) echo "unknown option $1" >&2; exit 1 ;;
  esac
  shift
done

REPO=erango/t1k
SPARKLE=target/sparkle/2.10.1
OUT=target/updates
mkdir -p "$OUT"

# 1. Version. Sparkle compares CFBundleVersion, which bundle.sh takes from Cargo.toml.
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# 2-3. Build, sign, notarize.
./scripts/bundle.sh
if [ -z "$LOCAL" ]; then
  ./scripts/notarize.sh target/release/T1K.app
fi

# Debug symbols to Sentry, so crash reports from this build name the code.
if [ -n "${SENTRY_AUTH_TOKEN:-}" ] && command -v sentry-cli >/dev/null; then
  SENTRY_ORG=tv-tracker SENTRY_PROJECT=t1k sentry-cli debug-files upload --wait target/release/t1k.dSYM
else
  echo "==> skipped dSYM upload (needs SENTRY_AUTH_TOKEN in .env.local and sentry-cli)"
fi

# 4. Zip (ditto keeps symlinks and signatures intact) and sign for Sparkle.
ZIP="$OUT/T1K-$VERSION.zip"
rm -f "$ZIP"
ditto -c -k --sequesterRsrc --keepParent target/release/T1K.app "$ZIP"
SIG_ATTRS="$("$SPARKLE/bin/sign_update" --account t1k "$ZIP")"

# 5. Appcast item.
if [ -n "$LOCAL" ]; then
  URL="$LOCAL/T1K-$VERSION.zip"
  APPCAST="$OUT/appcast.xml"
  [ -f "$APPCAST" ] || cp appcast.xml "$APPCAST" 2>/dev/null || true
else
  URL="https://github.com/$REPO/releases/download/v$VERSION/T1K-$VERSION.zip"
  APPCAST=appcast.xml
fi
NOTES="docs/releases/$VERSION.md"
python3 - "$APPCAST" "$VERSION" "$URL" "$SIG_ATTRS" "$CHANNEL" "$NOTES" "https://github.com/$REPO/releases/tag/v$VERSION" <<'PY'
import html, os, sys, email.utils
appcast, version, url, sig, channel, notes_path, notes_url = sys.argv[1:]
notes = ""
if os.path.exists(notes_path):
    items = [l.strip()[2:].strip() for l in open(notes_path) if l.strip().startswith("- ")]
    notes = "<ul>" + "".join(f"<li>{html.escape(i)}</li>" for i in items) + "</ul>"
item = f"""    <item>
      <title>T1K {version}</title>
      <pubDate>{email.utils.formatdate(localtime=True)}</pubDate>
      <sparkle:version>{version}</sparkle:version>
      <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>13.0</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>{notes_url}</sparkle:fullReleaseNotesLink>
{f"      <sparkle:channel>{channel}</sparkle:channel>" + chr(10) if channel else ""}      <description><![CDATA[{notes}]]></description>
      <enclosure url="{url}" type="application/octet-stream" {sig} />
    </item>
"""
head = """<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>T1K</title>
"""
tail = "  </channel>\n</rss>\n"
text = open(appcast).read() if os.path.exists(appcast) else head + tail
# Newest first, right after the channel title.
marker = "    <title>T1K</title>\n"
text = text.replace(marker, marker + item, 1)
open(appcast, "w").write(text)
PY
echo "==> appcast: $APPCAST"

if [ -n "$LOCAL" ]; then
  git checkout -- Cargo.toml Cargo.lock 2>/dev/null || true
  echo "==> local release in $OUT (serve it: python3 -m http.server -d $OUT 8765)"
  exit 0
fi

# 6. Publish: the zip as a GitHub release, then the feed.
FLAGS=()
[ -n "$CHANNEL" ] && FLAGS+=(--prerelease)
NOTES_ARG=(--generate-notes)
[ -f "$NOTES" ] && NOTES_ARG=(--notes-file "$NOTES")
git add Cargo.toml Cargo.lock
git commit -m "release: $VERSION" || true
git tag "v$VERSION"
git push origin HEAD "v$VERSION"
gh release create "v$VERSION" "$ZIP" --repo "$REPO" --title "T1K $VERSION" "${NOTES_ARG[@]}" "${FLAGS[@]}"
git add appcast.xml
git commit -m "release: appcast for $VERSION"
git push origin HEAD
echo "==> released $VERSION"
