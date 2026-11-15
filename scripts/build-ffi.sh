#!/usr/bin/env bash
# Builds stats-ffi for the apps: static libraries wrapped as
# StatsFFI.xcframework plus the generated Swift bindings, both in
# apps/NasStats/Generated (not checked in).
#
# Usage: scripts/build-ffi.sh [debug|release] [macosx|iphoneos|iphonesimulator|all]
#
# Profile defaults to release. Platform defaults to $PLATFORM_NAME (set by
# Xcode build phases) or macosx. Only the requested slice is built; slices
# built earlier with the same profile are kept in the framework, so the Mac
# and iOS targets can share it.
set -euo pipefail

cd "$(dirname "$0")/.."

profile="${1:-release}"
platform="${2:-${PLATFORM_NAME:-macosx}}"

# Xcode build phases run with a minimal PATH.
export PATH="$HOME/.cargo/bin:$PATH"

triple_for() {
    case "$1" in
        macosx) echo aarch64-apple-darwin ;;
        iphoneos) echo aarch64-apple-ios ;;
        iphonesimulator) echo aarch64-apple-ios-sim ;;
        *) echo "unknown platform: $1" >&2; exit 1 ;;
    esac
}

if [[ "$platform" == "all" ]]; then
    platforms=(macosx iphoneos iphonesimulator)
else
    platforms=("$platform")
fi

# Check up front: inside $(...) the exit of triple_for would not stop the script.
for p in "${platforms[@]}"; do
    triple_for "$p" >/dev/null
done

cargo_profile=()
if [[ "$profile" == "release" ]]; then
    cargo_profile=(--release)
fi

for p in "${platforms[@]}"; do
    # Xcode exports SDKROOT and deployment targets for its own platform. They
    # must not leak into cargo unchanged: host proc-macros would be built
    # against the iOS SDK or with a deployment target the host loader
    # rejects. Each slice gets the SDK of its own platform instead, because
    # C code in dependencies (ring) needs a sysroot and the compiler shim
    # does not find one on its own inside an Xcode build phase. cc-rs
    # ignores an SDKROOT that does not match the target, so host builds in
    # the iOS slices fall back to the default SDK.
    sdk="$(xcrun --sdk "$p" --show-sdk-path)"
    ios_target=()
    case "$p" in
        iphoneos | iphonesimulator) ios_target=(IPHONEOS_DEPLOYMENT_TARGET=27.0) ;;
    esac
    # ${a[@]+"${a[@]}"}: empty arrays count as unset under `set -u` in the
    # bash 3.2 that Xcode build phases run.
    env -u MACOSX_DEPLOYMENT_TARGET -u IPHONEOS_DEPLOYMENT_TARGET \
        SDKROOT="$sdk" \
        ${ios_target[@]+"${ios_target[@]}"} \
        cargo build -p stats-ffi --target "$(triple_for "$p")" ${cargo_profile[@]+"${cargo_profile[@]}"}
done

work="target/ffi"
out="apps/NasStats/Generated"
rm -rf "$work"
mkdir -p "$work/swift" "$work/headers" "$out"

# Bindings come from the slice just built; all slices share one API.
bindings_lib="target/$(triple_for "${platforms[0]}")/$profile/libstats_ffi.a"
env -u SDKROOT -u MACOSX_DEPLOYMENT_TARGET -u IPHONEOS_DEPLOYMENT_TARGET \
    cargo run -q -p uniffi-bindgen -- generate \
    --library "$bindings_lib" \
    --language swift \
    --out-dir "$work/swift"

cp "$work/swift/stats_ffiFFI.h" "$work/headers/"
cp "$work/swift/stats_ffiFFI.modulemap" "$work/headers/module.modulemap"

libraries=()
stamp_input=()
for p in macosx iphoneos iphonesimulator; do
    lib="target/$(triple_for "$p")/$profile/libstats_ffi.a"
    if [[ -f "$lib" ]]; then
        libraries+=(-library "$lib" -headers "$work/headers")
        stamp_input+=("$lib")
    fi
done

# Leave the framework alone when nothing changed: Xcode may already be
# reading it for the app target.
stamp="$(shasum "${stamp_input[@]}" "$work/headers/stats_ffiFFI.h" | shasum | cut -d' ' -f1)"
if [[ -d "$out/StatsFFI.xcframework" && "$(cat "$out/.ffi-stamp" 2>/dev/null)" == "$stamp" ]] \
    && cmp -s "$work/swift/stats_ffi.swift" "$out/stats_ffi.swift"; then
    echo "stats-ffi ($profile, ${platforms[*]}) unchanged"
    exit 0
fi

# Build next to the old framework and swap, so Xcode never finds the path
# missing, and only touch the Swift file when it changed.
xcodebuild -create-xcframework "${libraries[@]}" \
    -output "$work/StatsFFI.xcframework" >/dev/null
rm -rf "$out/StatsFFI.xcframework.old"
if [[ -d "$out/StatsFFI.xcframework" ]]; then
    mv "$out/StatsFFI.xcframework" "$out/StatsFFI.xcframework.old"
fi
mv "$work/StatsFFI.xcframework" "$out/StatsFFI.xcframework"
rm -rf "$out/StatsFFI.xcframework.old"
if ! cmp -s "$work/swift/stats_ffi.swift" "$out/stats_ffi.swift"; then
    cp "$work/swift/stats_ffi.swift" "$out/"
fi
echo "$stamp" > "$out/.ffi-stamp"

echo "stats-ffi ($profile, ${platforms[*]}) -> $out"
