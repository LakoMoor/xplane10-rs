#!/usr/bin/env bash
# make_app.sh [version] — build target/dist/xplane10-rs.app (Apple Silicon) and a zip of it.
# The app bundles its FreeType and libpng so it runs without Homebrew; X-Plane content is never
# included — the app looks for a folder "X-Plane 10" beside itself, then ~/X-Plane 10, or uses
# XP_INSTALL.
set -euo pipefail
cd "$(dirname "$0")/.."
VER="${1:-0.0.0}"
cargo build --release -p xp
APP=target/dist/xplane10-rs.app
rm -rf target/dist && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp target/release/xp "$APP/Contents/MacOS/xplane10-rs"
cp packaging/macos/xplane10-rs.icns "$APP/Contents/Resources/"
FT=$(otool -L target/release/xp | awk '/libfreetype/{print $1}')
PNG=$(otool -L "$FT" | awk '/libpng/{print $1}' | head -1)
cp "$FT" "$APP/Contents/Frameworks/libfreetype.6.dylib"
cp "$PNG" "$APP/Contents/Frameworks/libpng16.16.dylib"
chmod u+w "$APP"/Contents/Frameworks/*.dylib
install_name_tool -change "$FT" @executable_path/../Frameworks/libfreetype.6.dylib "$APP/Contents/MacOS/xplane10-rs"
install_name_tool -id @executable_path/../Frameworks/libfreetype.6.dylib "$APP/Contents/Frameworks/libfreetype.6.dylib"
install_name_tool -change "$PNG" @executable_path/../Frameworks/libpng16.16.dylib "$APP/Contents/Frameworks/libfreetype.6.dylib"
install_name_tool -id @executable_path/../Frameworks/libpng16.16.dylib "$APP/Contents/Frameworks/libpng16.16.dylib"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>xplane10-rs</string>
  <key>CFBundleDisplayName</key><string>xplane10-rs</string>
  <key>CFBundleIdentifier</key><string>io.github.xplane10-rs</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleExecutable</key><string>xplane10-rs</string>
  <key>CFBundleIconFile</key><string>xplane10-rs</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# ad-hoc signature so Apple Silicon will run the modified binaries
codesign --force --deep --sign - "$APP" >/dev/null 2>&1
(cd target/dist && ditto -c -k --keepParent xplane10-rs.app "xplane10-rs-$VER-macos-arm64.zip")
echo "built $APP and target/dist/xplane10-rs-$VER-macos-arm64.zip"
