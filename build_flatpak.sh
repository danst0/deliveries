#!/usr/bin/env bash
set -euo pipefail

# Simple helper to build, optionally install, and optionally run the Flatpak.
# Usage examples:
#   ./build_flatpak.sh              # build and export repo
#   INSTALL=1 ./build_flatpak.sh    # build + install to --user
#   RUN=1 ./build_flatpak.sh        # build + run after build (no install)
#   INSTALL=1 RUN=1 ./build_flatpak.sh

APP_ID="${APP_ID:-me.dumke.deliveries}"
MANIFEST="${MANIFEST:-${APP_ID}.yml}"
BUILD_DIR="${BUILD_DIR:-build-dir}"
REPO_DIR="${REPO_DIR:-flatpak-repo}"
CMD="${CMD:-deliveries_tracker}"
INSTALL="${INSTALL:-0}"
RUN_APP="${RUN:-0}"

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"

need() {
	command -v "$1" >/dev/null 2>&1 || { echo "Missing dependency: $1" >&2; exit 1; }
}

need flatpak-builder
need flatpak

echo "Building Flatpak for ${APP_ID} using ${MANIFEST}..."

# Clean up previous builds
rm -rf "${BUILD_DIR}" "${REPO_DIR}"

# Build and export to a local repo for distribution
flatpak-builder \
	--force-clean \
	--repo "${REPO_DIR}" \
	"${BUILD_DIR}" "${MANIFEST}"

if [[ "${INSTALL}" == "1" ]]; then
	echo "Installing ${APP_ID} to --user..."
	flatpak install --user --reinstall "${REPO_DIR}" "${APP_ID}"
fi

if [[ "${RUN_APP}" == "1" ]]; then
	echo "Running ${CMD} from build dir..."
	flatpak-builder --run "${BUILD_DIR}" "${MANIFEST}" "${CMD}"
fi


python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json

echo "Done. Repo available at: ${REPO_DIR}"
echo "To run without installing: flatpak-builder --run ${BUILD_DIR} ${MANIFEST} ${CMD}"
echo "To install later: flatpak install --user --reinstall ${REPO_DIR} ${APP_ID}"
