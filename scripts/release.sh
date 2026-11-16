#!/usr/bin/env bash
# Archives the apps and uploads them to App Store Connect (TestFlight).
#
#   scripts/release.sh          iPhone, then Mac
#   scripts/release.sh ios      iPhone only
#   scripts/release.sh macos    Mac only (Mac App Store build)
#
# Needs the app records in App Store Connect (bundle id net.staude.nasstats,
# iOS and macOS), see docs/release.md. Uploads with the account signed in to
# Xcode; an App Store Connect API key is optional:
#   ASC_KEY_ID and ASC_ISSUER_ID (both or none), the .p8 file under
#   ~/.appstoreconnect/private_keys or ASC_KEY_PATH
# Run scripts/bump-build.sh and commit before every upload; build numbers are
# never reused.
set -euo pipefail

cd "$(dirname "$0")/.."

spec="apps/NasStats/project.yml"
export_options="scripts/ExportOptions-AppStore.plist"

die() { echo "release: $*" >&2; exit 1; }
step() { echo "==> $*"; }

target="${1:-all}"
case "$target" in
    all) platforms=(ios macos) ;;
    ios | macos) platforms=("$target") ;;
    *) die "argument must be ios, macos or all" ;;
esac

command -v xcodegen >/dev/null || die "xcodegen not found (brew install xcodegen)"
if [[ -n "$(git status --porcelain)" ]]; then
    die "working tree not clean; commit first so the build matches a revision"
fi

# Optional API key: both values or none.
auth=()
if [[ -n "${ASC_KEY_ID:-}" || -n "${ASC_ISSUER_ID:-}" ]]; then
    [[ -n "${ASC_KEY_ID:-}" && -n "${ASC_ISSUER_ID:-}" ]] \
        || die "ASC_KEY_ID and ASC_ISSUER_ID must be set together"
    key="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8}"
    [[ -f "$key" ]] || die "API key file not found: $key"
    auth=(-authenticationKeyID "$ASC_KEY_ID" -authenticationKeyIssuerID "$ASC_ISSUER_ID"
          -authenticationKeyPath "$key")
fi

version=$(sed -n 's/^ *MARKETING_VERSION: "\(.*\)"$/\1/p' "$spec")
build=$(sed -n 's/^ *CURRENT_PROJECT_VERSION: "\(.*\)"$/\1/p' "$spec")

for platform in "${platforms[@]}"; do
    case "$platform" in
        ios) scheme="NasStats"; destination="generic/platform=iOS"; slice="iphoneos" ;;
        macos) scheme="NasStatsMac"; destination="generic/platform=macOS"; slice="macosx" ;;
    esac
    out="target/release-$platform"

    step "NAS Stats $version ($build) for $platform, $(git rev-parse --short HEAD)"
    rm -rf "$out"
    mkdir -p "$out"

    step "Rust core and Xcode project"
    scripts/build-ffi.sh release "$slice"
    (cd apps/NasStats && xcodegen generate --quiet)

    step "Archive"
    xcodebuild -project apps/NasStats/NasStats.xcodeproj -scheme "$scheme" \
        -configuration Release -destination "$destination" \
        -derivedDataPath "$out/derived" -archivePath "$out/NasStats.xcarchive" \
        -allowProvisioningUpdates ${auth[@]+"${auth[@]}"} archive -quiet

    step "Upload to App Store Connect"
    xcodebuild -exportArchive -archivePath "$out/NasStats.xcarchive" \
        -exportOptionsPlist "$export_options" -exportPath "$out/export" \
        -allowProvisioningUpdates ${auth[@]+"${auth[@]}"}

    step "Done: $platform build $build uploaded, processing takes a few minutes in App Store Connect"
done
