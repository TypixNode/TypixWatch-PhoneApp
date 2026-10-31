#!/bin/sh
# Builds core/ for iOS and generates the Swift bindings.  Run this before opening the
# Xcode project, and again whenever core/ changes.
#   ios/Frameworks/TypixCore.xcframework   static library, device and simulator
#   ios/Generated/typix_core.swift         the UniFFI bindings
#
# Pass "sim" for the simulator slices only.  Needs rustup targets aarch64-apple-ios,
# aarch64-apple-ios-sim and x86_64-apple-ios.
set -e
export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CORE=$ROOT/core
OUT=$ROOT/ios/Generated
XC=$ROOT/ios/Frameworks/TypixCore.xcframework
TMP=$CORE/target/ios-headers
export IPHONEOS_DEPLOYMENT_TARGET=16.0

cd "$CORE"
SIM="aarch64-apple-ios-sim x86_64-apple-ios"
for t in $SIM; do cargo build --release --lib --target "$t"; done
[ "$1" = sim ] || cargo build --release --lib --target aarch64-apple-ios

# Metadata for library mode comes from any build of this crate.
cargo build --release --lib
rm -rf "$OUT" "$TMP" "$XC"
mkdir -p "$OUT" "$TMP" "$(dirname "$XC")"
cargo run --quiet --features bindgen --bin uniffi-bindgen -- generate \
	--library target/release/libtypix_core.dylib --language swift --out-dir "$TMP/gen"
cp "$TMP"/gen/*.swift "$OUT/"
cp "$TMP"/gen/*.h "$TMP/"
cat "$TMP"/gen/*.modulemap >"$TMP/module.modulemap"
rm -rf "$TMP/gen"

# The brand's Manrope, if this machine has it.  Theme.swift falls back to the system
# font, so the app builds without it; the fonts stay out of the repository.
FONTS=$HOME/w1a-build/brand/medeo/assets/fonts-manrope/manrope
if [ -d "$FONTS" ]; then
	cp "$FONTS/manrope-medium-5.otf" "$OUT/Manrope-Medium.otf"
	cp "$FONTS/manrope-semibold-7.otf" "$OUT/Manrope-SemiBold.otf"
fi

mkdir -p target/ios-sim
lipo -create -output target/ios-sim/libtypix_core.a \
	target/aarch64-apple-ios-sim/release/libtypix_core.a \
	target/x86_64-apple-ios/release/libtypix_core.a
set -- -library target/ios-sim/libtypix_core.a -headers "$TMP"
if [ -f target/aarch64-apple-ios/release/libtypix_core.a ]; then
	set -- "$@" -library target/aarch64-apple-ios/release/libtypix_core.a -headers "$TMP"
fi
xcodebuild -create-xcframework "$@" -output "$XC"
ls -d "$XC" "$OUT"/*
