APP_NAME := Dirigent
APP_BUNDLE := $(APP_NAME).app
BINARY := target/release/$(APP_NAME)
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
DMG := $(APP_NAME)-$(VERSION).dmg
ENTITLEMENTS := assets/Dirigent.entitlements

# Code signing identity — set via environment or override on command line:
#   make sign IDENTITY="Developer ID Application: Your Name (TEAMID)"
IDENTITY ?= $(CODESIGN_IDENTITY)

# Notarization credentials — set via environment or override:
#   make notarize APPLE_ID=you@example.com TEAM_ID=TEAMID
APPLE_ID ?= $(NOTARIZE_APPLE_ID)
TEAM_ID ?= $(NOTARIZE_TEAM_ID)

.PHONY: build bundle sign dmg notarize clean

build:
	cargo build --release

bundle: build
	@rm -rf $(APP_BUNDLE)
	@mkdir -p $(APP_BUNDLE)/Contents/MacOS
	@mkdir -p $(APP_BUNDLE)/Contents/Resources
	@sed 's/0\.1\.0/$(VERSION)/g' assets/Info.plist > $(APP_BUNDLE)/Contents/Info.plist
	@cp assets/PkgInfo $(APP_BUNDLE)/Contents/
	@cp $(BINARY) $(APP_BUNDLE)/Contents/MacOS/
	@cp assets/Dirigent.icns $(APP_BUNDLE)/Contents/Resources/
	@echo "Created $(APP_BUNDLE)"

sign: bundle
	@test -n "$(IDENTITY)" || (echo "Error: set IDENTITY or CODESIGN_IDENTITY"; exit 1)
	codesign --force --deep --options runtime \
		--entitlements $(ENTITLEMENTS) \
		--sign "$(IDENTITY)" \
		"$(APP_BUNDLE)"
	codesign --verify --deep --strict "$(APP_BUNDLE)"
	@echo "Signed and verified $(APP_BUNDLE)"

dmg: sign
	@rm -f $(DMG)
	create-dmg \
		--volname "$(APP_NAME)" \
		--volicon "assets/Dirigent.icns" \
		--window-pos 200 120 \
		--window-size 600 400 \
		--icon-size 100 \
		--icon "$(APP_BUNDLE)" 150 190 \
		--app-drop-link 450 190 \
		--hide-extension "$(APP_BUNDLE)" \
		"$(DMG)" \
		"$(APP_BUNDLE)"
	@echo "Created $(DMG)"

notarize: dmg
	@test -n "$(APPLE_ID)" || (echo "Error: set APPLE_ID or NOTARIZE_APPLE_ID"; exit 1)
	@test -n "$(TEAM_ID)" || (echo "Error: set TEAM_ID or NOTARIZE_TEAM_ID"; exit 1)
	xcrun notarytool submit "$(DMG)" \
		--apple-id "$(APPLE_ID)" \
		--team-id "$(TEAM_ID)" \
		--password "@keychain:notarytool-password" \
		--wait
	xcrun stapler staple "$(DMG)"
	@echo "Notarized and stapled $(DMG)"

clean:
	cargo clean
	rm -rf $(APP_BUNDLE)
	rm -f $(APP_NAME)-*.dmg
