#!/usr/bin/env bash
# Builds the distributable: Popcorn.app with libmpv inside, and a DMG made from it.
# Tauri's own DMG step is skipped because it would run before libmpv is bundled.
#
# Usage: scripts/package.sh [extra "cargo tauri build" arguments]
# Env:   CODESIGN_IDENTITY  optional Developer ID identity (default: ad-hoc)
# Needs: Rust, tauri-cli v2, and the build tools of scripts/build-libmpv.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPTS="$ROOT/scripts"
TARGET=aarch64-apple-darwin
BUNDLE="${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}/$TARGET/release/bundle"
APP="$BUNDLE/macos/Popcorn.app"

export MACOSX_DEPLOYMENT_TARGET=13.0

# Does nothing if the library is already built from the current script.
"$SCRIPTS/build-libmpv.sh"

(cd "$ROOT" && cargo tauri build --target "$TARGET" --bundles app "$@")

"$SCRIPTS/bundle-dylibs.sh" "$APP"

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
DMG="$BUNDLE/dmg/Popcorn_${VERSION}_aarch64.dmg"
mkdir -p "$BUNDLE/dmg"
rm -f "$DMG"

# Check the app before spending time on the disk image.
"$SCRIPTS/check-size.sh" "$APP"

# hdiutil sometimes fails with "Resource busy" (exit 16) on CI machines, while
# something else still holds the disk tools; trying again a little later works.
retry_hdiutil() {
  local attempt
  for attempt in 1 2 3 4 5; do
    hdiutil "$@" && return 0
    local status=$?
    [[ "$attempt" == 5 ]] && return "$status"
    echo "warning: hdiutil $1 failed (exit $status), retrying in $((attempt * 5))s" >&2
    sleep $((attempt * 5))
  done
}

WORK="$(mktemp -d "${TMPDIR:-/tmp}/popcorn-dmg.XXXXXX")"
STAGING="$WORK/staging"
RW_DMG="$WORK/layout.dmg"
cleanup() {
  [[ -n "${DEVICE:-}" ]] && hdiutil detach "$DEVICE" -quiet -force 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$STAGING/.background"
ditto "$APP" "$STAGING/Popcorn.app"
ln -s /Applications "$STAGING/Applications"
# One TIFF with both resolutions, so Retina screens get the sharp one.
tiffutil -cathidpicheck "$SCRIPTS/dmg/background.png" "$SCRIPTS/dmg/background@2x.png" \
  -out "$STAGING/.background/background.tiff" >/dev/null

# The window's content: 660x420, matching scripts/dmg/background.svg, with
# the icons on either side of its arrow. Finder's bounds include the title bar.
WINDOW_W=660
WINDOW_H=420
TITLE_BAR=28
ICON_Y=212
ICON_APP_X=150
ICON_APPS_X=510

# Finder lays the window out in a writable image, which is then compressed.
# Room to spare for the .DS_Store Finder writes.
SIZE_MB=$(( $(du -sm "$STAGING" | cut -f1) + 20 ))
# Finder finds disks by name, and the window's background is recorded
# against it, so the image must already be called Popcorn and be the only
# one. A mounted copy of the DMG being replaced is ejected; anything else
# named Popcorn means building without the styled window.
LAYOUT=1
if [[ -d /Volumes/Popcorn ]]; then
  stale="$(hdiutil info | awk -v dmg="$DMG" '
    /^image-path/ { sub(/^image-path *: /, ""); image = $0 }
    /\/Volumes\/Popcorn$/ && image == dmg { print $1 }')"
  if [[ -n "$stale" ]]; then
    hdiutil detach "$stale" -quiet || hdiutil detach "$stale" -quiet -force
  else
    echo "warning: another disk named Popcorn is mounted; eject it for the styled window" >&2
    LAYOUT=0
  fi
fi
retry_hdiutil create -volname Popcorn -srcfolder "$STAGING" -fs HFS+ -format UDRW \
  -size "${SIZE_MB}m" -ov "$RW_DMG" >/dev/null
# The device, not the path, identifies it: renaming changes the path.
DEVICE="$(retry_hdiutil attach "$RW_DMG" -readwrite -noverify -noautoopen | awk '/Apple_HFS/ { print $1 }')"

MOUNT=/Volumes/Popcorn
# Without Finder (some CI machines) the DMG still works, just unstyled.
if [[ "$LAYOUT" == 1 ]] && ! osascript - <<APPLESCRIPT
on run argv
  tell application "Finder"
    tell disk "Popcorn"
      open
      delay 1
      set current view of container window to icon view
      set toolbar visible of container window to false
      set statusbar visible of container window to false
      set the bounds of container window to {200, 120, 200 + $WINDOW_W, 120 + $WINDOW_H + $TITLE_BAR}
      set opts to the icon view options of container window
      set arrangement of opts to not arranged
      set icon size of opts to 96
      -- Black labels vanish beside the icons; the background names them.
      set text size of opts to 12
      set label position of opts to right
      set background picture of opts to file ".background:background.tiff"
      set position of item "Popcorn.app" of container window to {$ICON_APP_X, $ICON_Y}
      set position of item "Applications" of container window to {$ICON_APPS_X, $ICON_Y}
      close
      open
      delay 1
      update without registering applications
      close
    end tell
  end tell
end run
APPLESCRIPT
then
  echo "warning: Finder could not lay out the DMG window; it will open unstyled" >&2
fi
# Finder writes .DS_Store lazily; wait until it is on disk.
for _ in {1..10}; do
  [[ "$LAYOUT" == 1 ]] || break
  [[ -f "$MOUNT/.DS_Store" ]] && break
  sleep 1
done
chflags hidden "$MOUNT/.background"
sync
hdiutil detach "$DEVICE" -quiet || hdiutil detach "$DEVICE" -quiet -force
DEVICE=

# ULMO (LZMA) is much smaller than UDZO and is readable since macOS 10.15.
retry_hdiutil convert "$RW_DMG" -format ULMO -ov -o "$DMG" >/dev/null
retry_hdiutil verify "$DMG"

if [[ -n "${CODESIGN_IDENTITY:-}" && "$CODESIGN_IDENTITY" != "-" ]]; then
  codesign --force --sign "$CODESIGN_IDENTITY" --timestamp "$DMG"
fi

echo
"$SCRIPTS/check-size.sh" "$APP" "$DMG"
