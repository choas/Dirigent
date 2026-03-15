#!/bin/bash
set -euo pipefail

APP_NAME="Dirigent"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
BUNDLE_DIR="target/release/bundle"
APP_DIR="${BUNDLE_DIR}/${APP_NAME}.app"
DMG_PATH="${BUNDLE_DIR}/${APP_NAME}.dmg"
SKIP_BUILD=false

for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
    esac
done

# Build release binary unless --skip-build
if [ "$SKIP_BUILD" = false ]; then
    cargo build --release
fi

# Create bundle structure
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"

# Copy binary
cp "target/release/${APP_NAME}" "${APP_DIR}/Contents/MacOS/"

# Copy icon
cp "assets/Dirigent.icns" "${APP_DIR}/Contents/Resources/"

# Write PkgInfo
echo -n "APPL????" > "${APP_DIR}/Contents/PkgInfo"

# Write Info.plist (with version substitution)
sed "s/0\.1\.0/${VERSION}/g" assets/Info.plist > "${APP_DIR}/Contents/Info.plist"

# Code signing (if certificate is available)
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    # Import certificate from base64-encoded P12 if provided (CI)
    if [ -n "${P12_BASE64:-}" ]; then
        KEYCHAIN_PATH="$RUNNER_TEMP/app-signing.keychain-db"
        KEYCHAIN_PASSWORD="$(openssl rand -base64 32)"

        echo "$P12_BASE64" | base64 --decode > "$RUNNER_TEMP/certificate.p12"

        security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
        security default-keychain -s "$KEYCHAIN_PATH"
        security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
        security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
        security import "$RUNNER_TEMP/certificate.p12" -P "${P12_PASSWORD:-}" \
            -A -t cert -f pkcs12 -k "$KEYCHAIN_PATH"
        security set-key-partition-list -S apple-tool:,apple:,codesign: \
            -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
        # Preserve existing keychains in search list
        security list-keychains -d user -s "$KEYCHAIN_PATH" \
            $(security list-keychains -d user | tr -d '"' | tr '\n' ' ')

        rm "$RUNNER_TEMP/certificate.p12"

        echo "Available signing identities:"
        security find-identity -v -p codesigning "$KEYCHAIN_PATH"
    fi

    echo "Signing with identity: ${CODESIGN_IDENTITY}"

    # Sign the main binary first (inside-out signing)
    codesign --force --options runtime --timestamp \
        --entitlements assets/Dirigent.entitlements \
        --sign "$CODESIGN_IDENTITY" \
        "${APP_DIR}/Contents/MacOS/${APP_NAME}"

    # Then sign the overall app bundle
    codesign --force --options runtime --timestamp \
        --entitlements assets/Dirigent.entitlements \
        --sign "$CODESIGN_IDENTITY" \
        "${APP_DIR}"

    echo "Verifying signature..."
    codesign --verify --deep --strict --verbose=2 "${APP_DIR}"
    echo "Signature details for binary:"
    codesign --display --verbose=4 "${APP_DIR}/Contents/MacOS/${APP_NAME}"
    echo "Signature details for bundle:"
    codesign --display --verbose=4 "${APP_DIR}"
    spctl --assess --type execute --verbose=2 "${APP_DIR}" || true
fi

# Notarization (if Apple ID credentials are available)
if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_ID_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    echo "Submitting for notarization..."
    ZIP_PATH="${BUNDLE_DIR}/${APP_NAME}-notarize.zip"
    ditto -c -k --keepParent "${APP_DIR}" "$ZIP_PATH"

    SUBMIT_OUTPUT=$(xcrun notarytool submit "$ZIP_PATH" \
        --apple-id "$APPLE_ID" \
        --password "$APPLE_ID_PASSWORD" \
        --team-id "$APPLE_TEAM_ID" \
        --wait 2>&1) || true

    echo "$SUBMIT_OUTPUT"

    # Extract submission ID
    SUBMISSION_ID=$(echo "$SUBMIT_OUTPUT" | grep '  id:' | head -1 | awk '{print $2}')

    if echo "$SUBMIT_OUTPUT" | grep -q "status: Invalid"; then
        echo "Notarization failed! Fetching log for details..."
        xcrun notarytool log "$SUBMISSION_ID" \
            --apple-id "$APPLE_ID" \
            --password "$APPLE_ID_PASSWORD" \
            --team-id "$APPLE_TEAM_ID" \
            developer_log.json 2>&1 || true
        echo "--- Notarization Log ---"
        cat developer_log.json 2>/dev/null || echo "(no log available)"
        echo "--- End Notarization Log ---"
        rm -f developer_log.json
        rm "$ZIP_PATH"
        exit 1
    fi

    rm "$ZIP_PATH"

    echo "Stapling notarization ticket..."
    xcrun stapler staple "${APP_DIR}"
fi

# Create DMG
echo "Creating DMG..."
rm -f "$DMG_PATH"
hdiutil create -volname "$APP_NAME" \
    -srcfolder "${APP_DIR}" \
    -ov -format UDZO \
    "$DMG_PATH"

# Sign the DMG too if we have a signing identity
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --sign "$CODESIGN_IDENTITY" "$DMG_PATH"
fi

echo "Created ${APP_DIR}"
echo "Created ${DMG_PATH}"
echo "Size: $(du -sh "${APP_DIR}" | cut -f1)"
