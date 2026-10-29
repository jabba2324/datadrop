#!/bin/bash
# Build DataDrop.app and a .dmg around it, from the release binaries.
#   packaging/macos/package.sh <version> <target> <out-dir>
# Signed with a Developer ID and notarised when these are set (repository secrets):
#   APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD, APPLE_SIGNING_IDENTITY,
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD (an app-specific password).
# Otherwise it's signed ad hoc (it then opens with right-click → Open the first time).
set -euo pipefail
version="$1"; target="$2"; out="$3"
here="$(cd "$(dirname "$0")" && pwd)"
bin="target/$target/release"
work="$(mktemp -d)"
app="$work/DataDrop.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/datadrop-gui" "$bin/datadrop" "$app/Contents/MacOS/"
sed "s/__VERSION__/${version#v}/g" "$here/Info.plist" > "$app/Contents/Info.plist"

# The icon, from the 1024 px PNG.
iconset="$work/DataDrop.iconset"
mkdir "$iconset"
for s in 16 32 128 256 512; do
  sips -z $s $s "$here/../icon.png" --out "$iconset/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2)); sips -z $d $d "$here/../icon.png" --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/DataDrop.icns"

signed=""
if [ -n "${APPLE_CERTIFICATE:-}" ] && [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  keychain="$work/signing.keychain-db"
  pw="$(uuidgen)"
  security create-keychain -p "$pw" "$keychain"
  security set-keychain-settings -lut 3600 "$keychain"
  security unlock-keychain -p "$pw" "$keychain"
  echo "$APPLE_CERTIFICATE" | base64 --decode > "$work/cert.p12"
  security import "$work/cert.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple: -s -k "$pw" "$keychain" >/dev/null
  # Added to the keychains searched (the existing ones, one argument each).
  # shellcheck disable=SC2046
  security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
  for exe in "$app/Contents/MacOS/datadrop" "$app/Contents/MacOS/datadrop-gui"; do
    codesign --force --options runtime --timestamp --entitlements "$here/entitlements.plist" \
      --sign "$APPLE_SIGNING_IDENTITY" "$exe"
  done
  codesign --force --options runtime --timestamp --entitlements "$here/entitlements.plist" \
    --sign "$APPLE_SIGNING_IDENTITY" "$app"
  signed=yes
else
  echo "::warning::No Apple signing identity: DataDrop.app is signed ad hoc, not notarised"
  codesign --force --deep --sign - "$app"
fi

# The disk image: the app and a link to Applications.
stage="$work/dmg"
mkdir "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$out/DataDrop-$version-$target.dmg"
hdiutil create -volname "DataDrop" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null

if [ -n "$signed" ]; then
  codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$dmg"
  if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
    xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$dmg"
  else
    echo "::warning::Signed but not notarised: APPLE_ID, APPLE_TEAM_ID or APPLE_APP_PASSWORD is missing"
  fi
  security delete-keychain "$keychain"
fi
echo "Made $dmg"
