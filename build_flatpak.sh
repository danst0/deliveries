#!/bin/bash
set -e

APP_ID="com.example.deliveries"
MANIFEST="${APP_ID}.yml"
BUILD_DIR="build-dir"
REPO_DIR="repo"

echo "Building Flatpak for ${APP_ID}..."

# Clean up previous builds
rm -rf "${BUILD_DIR}" "${REPO_DIR}"

# Build the flatpak
flatpak-builder --force-clean --share=network "${BUILD_DIR}" "${MANIFEST}"

# Install locally (optional, comment out if not wanted)
# flatpak-builder --user --install --force-clean "${BUILD_DIR}" "${MANIFEST}"

echo "Build complete. To run:"
echo "flatpak-builder --run ${BUILD_DIR} ${MANIFEST} deliveries_tracker"
