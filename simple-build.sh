#!/usr/bin/env bash
set -euo pipefail

# Simple cross-platform-ish Linux release builder for goose-cli
# Produces: target/$TARGET/release/goose-$TARGET.tar.bz2
# Contents: goose, temporal-service, temporal (if download enabled)

ARCH="${1:-${ARCH:-x86_64}}"   # x86_64 | aarch64
NO_TEMPORAL_DOWNLOAD="${NO_TEMPORAL_DOWNLOAD:-0}" # set to 1 to skip temporal CLI download
TEMPORAL_VERSION="${TEMPORAL_VERSION:-1.3.0}"

case "$ARCH" in
  x86_64)  GOARCH=amd64 ; TARGET="x86_64-unknown-linux-gnu" ;;
  aarch64) GOARCH=arm64 ; TARGET="aarch64-unknown-linux-gnu" ;;
  *) echo "Unsupported ARCH: $ARCH (use x86_64 or aarch64)" ; exit 1 ;;
esac

echo "🔨 Building goose CLI for $TARGET (release)"

# Activate hermit toolchain if present
if [ -f "./bin/activate-hermit" ]; then
  # shellcheck disable=SC1091
  source ./bin/activate-hermit
fi

# Ensure rust target (for cargo direct builds)
if command -v rustup >/dev/null 2>&1; then
  rustup target add "$TARGET" >/dev/null 2>&1 || true
fi

# Choose builder: cross if available, else cargo
BUILD_DIR="target/$TARGET/release"
if command -v cross >/dev/null 2>&1; then
  echo "Using cross to build"
  cross build --release --target "$TARGET" -p goose-cli -vv
else
  echo "cross not found; building with cargo for host."
  cargo build --release -p goose-cli
  # If host target differs, copy into expected dir
  mkdir -p "$BUILD_DIR"
  if [ -f "target/release/goose" ]; then
    cp target/release/goose "$BUILD_DIR/goose"
  fi
fi

# Verify goose binary exists in $BUILD_DIR
if [ ! -f "$BUILD_DIR/goose" ]; then
  # cross output should be here; if not, try copying from target/$TARGET
  if [ -f "target/$TARGET/release/goose" ]; then
    mkdir -p "$BUILD_DIR"
    cp "target/$TARGET/release/goose" "$BUILD_DIR/goose"
  fi
fi
if [ ! -f "$BUILD_DIR/goose" ]; then
  echo "❌ goose binary not found in $BUILD_DIR"
  exit 1
fi

# Build temporal-service (Go)
echo "🔧 Building temporal-service for GOOS=linux GOARCH=$GOARCH"
pushd temporal-service >/dev/null
dos2unix ./build.sh
GOOS=linux GOARCH="$GOARCH" ./build.sh
popd >/dev/null
mv -f "temporal-service/temporal-service" "$BUILD_DIR/temporal-service" 2>/dev/null || true

# Download temporal CLI (optional)
if [ "$NO_TEMPORAL_DOWNLOAD" != "1" ]; then
  echo "⬇️  Downloading temporal CLI v$TEMPORAL_VERSION for linux/$GOARCH"
  case "$GOARCH" in
    amd64) TEMPORAL_ARCH=amd64 ;;
    arm64) TEMPORAL_ARCH=arm64 ;;
  esac
  TMP_TAR="temporal_cli_${TEMPORAL_VERSION}_linux_${TEMPORAL_ARCH}.tar.gz"
  if command -v curl >/dev/null 2>&1; then
    if curl -fsSL "https://github.com/temporalio/cli/releases/download/v${TEMPORAL_VERSION}/${TMP_TAR}" -o "$TMP_TAR"; then
      tar -xzf "$TMP_TAR"
      chmod +x temporal || true
      mv -f temporal "$BUILD_DIR/temporal" 2>/dev/null || true
      rm -f "$TMP_TAR"
    else
      echo "⚠️  Failed to download temporal CLI; continuing without it."
    fi
  else
    echo "⚠️  curl not found; skipping temporal CLI download."
  fi
else
  echo "⏭️  Skipping temporal CLI download (NO_TEMPORAL_DOWNLOAD=1)"
fi

# Package
PKG_DIR="$BUILD_DIR/goose-package"
mkdir -p "$PKG_DIR"
cp -f "$BUILD_DIR/goose" "$PKG_DIR/" 2>/dev/null || true
[ -f "$BUILD_DIR/temporal-service" ] && cp -f "$BUILD_DIR/temporal-service" "$PKG_DIR/"
[ -f "$BUILD_DIR/temporal" ] && cp -f "$BUILD_DIR/temporal" "$PKG_DIR/"

ARTIFACT="$BUILD_DIR/goose-$TARGET.tar.bz2"
echo "📦 Packaging artifact: $ARTIFACT"
tar -cjf "$ARTIFACT" -C "$PKG_DIR" .

echo "✅ Done"
echo "Artifact: $ARTIFACT"
