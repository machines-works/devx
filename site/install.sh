#!/bin/sh
set -e

REPO="machines-works/devx"
INSTALL_DIR="/usr/local/bin"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Darwin) OS="darwin" ;;
  Linux)  OS="linux" ;;
  *)
    echo "error: unsupported OS: $OS" >&2
    exit 1
    ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) ARCH="arm64" ;;
  x86_64|amd64)  ARCH="x86_64" ;;
  *)
    echo "error: unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

ARTIFACT="devx-${OS}-${ARCH}"

# Get latest release tag
TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)

if [ -z "$TAG" ]; then
  echo "error: could not determine latest release" >&2
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/${TAG}/${ARTIFACT}.tar.gz"

echo "devx ${TAG} (${OS}/${ARCH})"
echo "downloading ${URL}..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "${TMPDIR}/${ARTIFACT}.tar.gz"
tar -xzf "${TMPDIR}/${ARTIFACT}.tar.gz" -C "$TMPDIR"

# Install — try /usr/local/bin, fall back to ~/.local/bin
if [ -w "$INSTALL_DIR" ]; then
  mv "${TMPDIR}/devx" "${INSTALL_DIR}/devx"
else
  echo "installing to ${INSTALL_DIR} (requires sudo)..."
  sudo mv "${TMPDIR}/devx" "${INSTALL_DIR}/devx"
fi

chmod +x "${INSTALL_DIR}/devx"

echo ""
echo "devx installed to ${INSTALL_DIR}/devx"
echo "run 'devx up' in any project with a devx.toml"
