#!/bin/sh
set -e

VERSION="0.1.0"
BASE_URL="https://devx.machines.works/releases"
INSTALL_DIR="$HOME/.devx/bin"

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
URL="${BASE_URL}/v${VERSION}/${ARTIFACT}.tar.gz"

echo "devx v${VERSION} (${OS}/${ARCH})"
echo "downloading ${URL}..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "${TMPDIR}/${ARTIFACT}.tar.gz"
tar -xzf "${TMPDIR}/${ARTIFACT}.tar.gz" -C "$TMPDIR"

# Install to ~/.devx/bin (no sudo needed)
mkdir -p "$INSTALL_DIR"
mv "${TMPDIR}/devx" "${INSTALL_DIR}/devx"
chmod +x "${INSTALL_DIR}/devx"

# Add to PATH if not already there
add_to_path() {
  local rc="$1"
  if [ -f "$rc" ]; then
    if ! grep -q '\.devx/bin' "$rc" 2>/dev/null; then
      echo '' >> "$rc"
      echo '# devx' >> "$rc"
      echo 'export PATH="$HOME/.devx/bin:$PATH"' >> "$rc"
    fi
  fi
}

case "$SHELL" in
  */zsh)  add_to_path "$HOME/.zshrc" ;;
  */bash) add_to_path "$HOME/.bashrc" ;;
  *)      add_to_path "$HOME/.profile" ;;
esac

echo ""
echo "devx v${VERSION} installed to ${INSTALL_DIR}/devx"
echo ""
echo "Next steps:"
echo "  1. Restart your shell or run: export PATH=\"\$HOME/.devx/bin:\$PATH\""
echo "  2. cd into your project and run: devx init"
echo "  3. Start your stack: devx up"
