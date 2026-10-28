#!/usr/bin/env bash
# The Apple gate: builds the XCFramework, generates the project and runs
# `xcodebuild test` (the app's unit tests, the XCUITest smoke and the
# GentboxCore package tests), signed ad hoc with no team whatever
# Signing.local.xcconfig says, so it runs the same here and on CI.
#
# Ad hoc rather than CODE_SIGNING_ALLOWED=NO: the UI smoke launches the app
# and the test runner, which need a signature. The ad hoc entitlements open
# ~/Library/Application Support/Gentbox, where the smoke keeps its state in
# UITests/ (emptied at each launch), apart from your real inbox.
#
# DerivedData goes to apps/apple/build (gitignored). Extra arguments go to
# xcodebuild, e.g. `-only-testing:GentboxUITests`.
#
# Usage: apps/apple/scripts/test.sh [xcodebuild arguments…]
set -euo pipefail

apple_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$apple_dir/scripts/build-xcframework.sh"
cd "$apple_dir"
xcodegen generate --quiet
xcodebuild test \
	-project Gentbox.xcodeproj -scheme Gentbox \
	-destination 'platform=macOS' -derivedDataPath build \
	CODE_SIGN_IDENTITY=- DEVELOPMENT_TEAM= \
	GENTBOX_ENTITLEMENTS=Signing/AdHoc.entitlements \
	GENTBOX_BUNDLE_IDENTIFIER=com.eiluviann.gentbox GENTBOX_APP_GROUP= \
	"$@"
