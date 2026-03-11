#!/bin/bash
set -euo pipefail

APP_NAME="Dirigent"
INSTALL_DIR="/Applications"
APP_PATH="${INSTALL_DIR}/${APP_NAME}.app"

echo "Building release..."
cargo build --release

echo "Bundling .app..."
./scripts/bundle_macos.sh --skip-build

BUNDLE_APP="target/release/bundle/${APP_NAME}.app"

# Remove old installation
if [ -d "$APP_PATH" ]; then
    echo "Removing old ${APP_PATH}..."
    rm -rf "$APP_PATH"
fi

# Copy to /Applications
echo "Installing to ${APP_PATH}..."
cp -R "$BUNDLE_APP" "$APP_PATH"

# Reset TCC so stale Apple Music prompts don't persist
BUNDLE_ID=$(defaults read "${APP_PATH}/Contents/Info.plist" CFBundleIdentifier 2>/dev/null || echo "")
if [ -n "$BUNDLE_ID" ]; then
    echo "Resetting TCC permissions for ${BUNDLE_ID}..."
    tccutil reset All "$BUNDLE_ID" 2>/dev/null || true
fi

echo ""
echo "Installed ${APP_NAME} to ${APP_PATH}"
echo "Size: $(du -sh "$APP_PATH" | cut -f1)"
echo ""
echo "Launch: open ${APP_PATH}"
