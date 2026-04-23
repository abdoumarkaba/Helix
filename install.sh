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

# Install main binary
echo "Installing to ${INSTALL_DIR}/play..."
mkdir -p "$INSTALL_DIR"
mv play "$INSTALL_DIR/play"
chmod +x "$INSTALL_DIR/play"

# Install play-helper if present in the archive
if [ -f "play-helper" ]; then
  echo "Installing play-helper to ${INSTALL_DIR}/play-helper..."
  mv play-helper "$INSTALL_DIR/play-helper"
  chmod +x "$INSTALL_DIR/play-helper"
fi

# Install polkit policy if present (requires sudo for system-wide install)
if [ -f "com.github.abdoumarkt.play.policy" ]; then
  POLICY_DIR="/usr/share/polkit-1/actions"
  if [ -w "$POLICY_DIR" ] || [ "$EUID" -eq 0 ]; then
    echo "Installing polkit policy to ${POLICY_DIR}..."
    mv com.github.abdoumarkt.play.policy "$POLICY_DIR/"
  else
    echo "Installing polkit policy (requires sudo)..."
    sudo mv com.github.abdoumarkt.play.policy "$POLICY_DIR/"
  fi
fi

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
  echo ""
  echo "Note: System tweaks require polkit authentication."
  echo "      You will be prompted for your password when needed."
else
  echo ""
  echo "Warning: Installation verification failed"
  echo "Make sure ${INSTALL_DIR} is in your PATH"
fi
