#!/usr/bin/env bash
set -e

VERSION="${VERSION:-latest}"
REPO="${REPO:-abdoumarkt/play}"

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)
    TARGET="x86_64-unknown-linux-gnu"
    ;;
  aarch64|arm64)
    TARGET="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "Error: Unsupported architecture: $ARCH"
    echo "Supported: x86_64, aarch64"
    exit 1
    ;;
esac

# Determine install location
if [ -w "/usr/local/bin" ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
fi

echo "Downloading play ${VERSION} for ${ARCH}..."

# Get download URL
if [ "$VERSION" = "latest" ]; then
  DOWNLOAD_URL=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep "browser_download_url.*play-${TARGET}.tar.gz" | cut -d '"' -f 4)
else
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/play-${TARGET}.tar.gz"
fi

if [ -z "$DOWNLOAD_URL" ]; then
  echo "Error: Could not find release for ${TARGET}"
  exit 1
fi

# Download and extract
TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"
curl -fsSL "$DOWNLOAD_URL" -o play.tar.gz
tar xzf play.tar.gz

# Install
echo "Installing to ${INSTALL_DIR}/play..."
mkdir -p "$INSTALL_DIR"
mv play "$INSTALL_DIR/play"
chmod +x "$INSTALL_DIR/play"

# Cleanup
cd -
rm -rf "$TMP_DIR"

# Setup completion
echo "Setting up bash completion..."
COMPLETION_DIR="$HOME/.local/share/bash-completion/completions"
mkdir -p "$COMPLETION_DIR"
"$INSTALL_DIR/play" --generate-completion bash > "$COMPLETION_DIR/play" 2>/dev/null || echo "  (Completion setup skipped)"

# Verify installation
echo "Verifying installation..."
if "$INSTALL_DIR/play" --version > /dev/null 2>&1; then
  echo ""
  echo "Done! Run 'play --help' to get started."
else
  echo ""
  echo "Warning: Installation verification failed"
  echo "Make sure ${INSTALL_DIR} is in your PATH"
fi
