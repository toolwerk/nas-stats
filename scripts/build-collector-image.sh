#!/usr/bin/env bash
# Builds the Docker image of the collector (#46) and saves it as a file
# that Container Station imports: target/collector/nas-collector-<version>-<arch>.tar,
# tagged nas-collector:<version> and nas-collector:latest.
#
# Usage: scripts/build-collector-image.sh [amd64|arm64]
#
# Architecture defaults to amd64 (most QNAP with Intel or AMD); arm64 for
# QNAP with an ARM processor. Needs Docker with buildx; another
# architecture than the one of this Mac is built in emulation and takes a
# while.
set -euo pipefail

cd "$(dirname "$0")/.."

arch="${1:-amd64}"
case "$arch" in
    amd64 | arm64) ;;
    *) echo "unknown architecture: $arch (amd64 or arm64)" >&2; exit 2 ;;
esac

if ! command -v docker >/dev/null 2>&1 || ! docker buildx version >/dev/null 2>&1; then
    echo "Docker with buildx is needed: https://docs.docker.com/get-docker/" >&2
    exit 1
fi

version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
if [[ -z "$version" ]]; then
    echo "no version in Cargo.toml" >&2
    exit 1
fi

out="target/collector/nas-collector-$version-$arch.tar"
mkdir -p target/collector
docker buildx build \
    --platform "linux/$arch" \
    --file docker/collector/Dockerfile \
    --tag "nas-collector:$version" \
    --tag "nas-collector:latest" \
    --output "type=docker,dest=$out" \
    .
echo "$out"
