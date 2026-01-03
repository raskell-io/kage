#!/bin/sh
# Kage installer script
# Usage: curl -fsSL https://kage.raskell.io/install.sh | sh
#
# Environment variables:
#   KAGE_INSTALL_DIR - Installation directory (default: ~/.local/bin or /usr/local/bin)
#   KAGE_VERSION     - Specific version to install (default: latest)

set -e

REPO="raskell-io/kage"
BINARY_NAME="kage"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

info() {
    printf "${BLUE}${BOLD}info${NC}: %s\n" "$1"
}

success() {
    printf "${GREEN}${BOLD}success${NC}: %s\n" "$1"
}

warn() {
    printf "${YELLOW}${BOLD}warning${NC}: %s\n" "$1"
}

error() {
    printf "${RED}${BOLD}error${NC}: %s\n" "$1" >&2
    exit 1
}

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "darwin" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64) echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        armv7l) echo "armv7" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Get the latest version from GitHub
get_latest_version() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

# Download file
download() {
    url="$1"
    output="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$output"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$output"
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

# Determine install directory
get_install_dir() {
    if [ -n "$KAGE_INSTALL_DIR" ]; then
        echo "$KAGE_INSTALL_DIR"
    elif [ -w "/usr/local/bin" ]; then
        echo "/usr/local/bin"
    elif [ -d "$HOME/.local/bin" ]; then
        echo "$HOME/.local/bin"
    else
        mkdir -p "$HOME/.local/bin"
        echo "$HOME/.local/bin"
    fi
}

# Check if directory is in PATH
check_path() {
    dir="$1"
    case ":$PATH:" in
        *":$dir:"*) return 0 ;;
        *) return 1 ;;
    esac
}

# Main installation
main() {
    OS=$(detect_os)
    ARCH=$(detect_arch)

    info "Detected OS: $OS, Architecture: $ARCH"

    # Get version
    if [ -n "$KAGE_VERSION" ]; then
        VERSION="$KAGE_VERSION"
    else
        info "Fetching latest version..."
        VERSION=$(get_latest_version)
        if [ -z "$VERSION" ]; then
            error "Could not determine latest version. Set KAGE_VERSION manually or check your internet connection."
        fi
    fi

    info "Installing Kage $VERSION"

    # Construct download URL
    # Binary naming convention: kage-{version}-{os}-{arch}.tar.gz
    # Strip 'v' prefix if present for the filename
    VERSION_NUM="${VERSION#v}"

    if [ "$OS" = "windows" ]; then
        ARCHIVE_NAME="kage-${VERSION_NUM}-${OS}-${ARCH}.zip"
        BINARY_NAME="kage.exe"
    else
        ARCHIVE_NAME="kage-${VERSION_NUM}-${OS}-${ARCH}.tar.gz"
    fi

    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE_NAME}"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    info "Downloading from $DOWNLOAD_URL"
    download "$DOWNLOAD_URL" "$TMP_DIR/$ARCHIVE_NAME"

    # Extract
    info "Extracting..."
    cd "$TMP_DIR"
    if [ "$OS" = "windows" ]; then
        unzip -q "$ARCHIVE_NAME"
    else
        tar -xzf "$ARCHIVE_NAME"
    fi

    # Find the binary
    if [ -f "$TMP_DIR/$BINARY_NAME" ]; then
        BINARY_PATH="$TMP_DIR/$BINARY_NAME"
    elif [ -f "$TMP_DIR/kage-${VERSION_NUM}-${OS}-${ARCH}/$BINARY_NAME" ]; then
        BINARY_PATH="$TMP_DIR/kage-${VERSION_NUM}-${OS}-${ARCH}/$BINARY_NAME"
    else
        # Try to find it
        BINARY_PATH=$(find "$TMP_DIR" -name "$BINARY_NAME" -type f | head -n 1)
        if [ -z "$BINARY_PATH" ]; then
            error "Could not find $BINARY_NAME in archive"
        fi
    fi

    # Install
    INSTALL_DIR=$(get_install_dir)
    info "Installing to $INSTALL_DIR"

    if [ ! -w "$INSTALL_DIR" ]; then
        warn "Need elevated permissions to install to $INSTALL_DIR"
        sudo mkdir -p "$INSTALL_DIR"
        sudo cp "$BINARY_PATH" "$INSTALL_DIR/$BINARY_NAME"
        sudo chmod +x "$INSTALL_DIR/$BINARY_NAME"
    else
        mkdir -p "$INSTALL_DIR"
        cp "$BINARY_PATH" "$INSTALL_DIR/$BINARY_NAME"
        chmod +x "$INSTALL_DIR/$BINARY_NAME"
    fi

    success "Kage $VERSION installed to $INSTALL_DIR/$BINARY_NAME"

    # Check if in PATH
    if ! check_path "$INSTALL_DIR"; then
        warn "$INSTALL_DIR is not in your PATH"
        echo ""
        echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
    fi

    # Verify installation
    if command -v kage >/dev/null 2>&1; then
        echo ""
        info "Verifying installation..."
        kage --version
    else
        echo ""
        info "Run 'kage --version' to verify the installation"
    fi

    echo ""
    success "Installation complete! Run 'kage --help' to get started."
}

main "$@"
