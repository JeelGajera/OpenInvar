#!/usr/bin/env bash
set -e

# OpenInvar Installation Script

GITHUB_REPO="JeelGajera/OpenInvar"
INSTALL_DIR="${HOME}/.local/bin"

if [ "$EUID" -eq 0 ]; then
    INSTALL_DIR="/usr/local/bin"
fi

# Colors and formatting
BOLD="$(tput bold 2>/dev/null || printf '')"
GREY="$(tput setaf 0 2>/dev/null || printf '')"
RED="$(tput setaf 1 2>/dev/null || printf '')"
GREEN="$(tput setaf 2 2>/dev/null || printf '')"
YELLOW="$(tput setaf 3 2>/dev/null || printf '')"
BLUE="$(tput setaf 4 2>/dev/null || printf '')"
MAGENTA="$(tput setaf 5 2>/dev/null || printf '')"
CYAN="$(tput setaf 6 2>/dev/null || printf '')"
RESET="$(tput sgr0 2>/dev/null || printf '')"

info() { printf "${BLUE}info${RESET} %s\n" "$1"; }
success() { printf "${GREEN}success${RESET} %s\n" "$1"; }
error() { printf "${RED}error${RESET} %s\n" "$1"; exit 1; }
warn() { printf "${YELLOW}warn${RESET} %s\n" "$1"; }
step() { printf "${BOLD}${CYAN}»${RESET} ${BOLD}%s${RESET}\n" "$1"; }

printf "${BLUE}${BOLD}"
cat << "EOF"
   ____                   ____
  / __ \____  ___  ____  /  _/___ _   ______ ______
 / / / / __ \/ _ \/ __ \ / // __ \ | / / __ `/ ___/
/ /_/ / /_/ /  __/ / / // // / / / |/ / /_/ / /
\____/ .___/\___/_/ /_/___/_/ /_/|___/\__,_/_/
    /_/
EOF
printf "${RESET}\n"

step "Initializing installation..."

# 1. Detect OS
OS="$(uname -s)"
case "${OS}" in
    Linux*)     PLATFORM="unknown-linux-gnu";;
    Darwin*)    PLATFORM="apple-darwin";;
    *)          error "Unsupported OS '${OS}'";;
esac

# 2. Detect Architecture
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64*)    ARCH="x86_64";;
    arm64*|aarch64*) ARCH="aarch64";;
    *)          error "Unsupported architecture '${ARCH}'";;
esac

if [ "$OS" = "Darwin" ] && [ "$ARCH" = "x86_64" ]; then
    warn "Pre-built binaries are not available for Intel Macs (x86_64 Darwin)."
    info "Install from source with Cargo:"
    info "  cargo install openinvar-cli --git https://github.com/JeelGajera/OpenInvar"
    exit 0
fi

TARGET="${ARCH}-${PLATFORM}"
ASSET_NAME="openinvar-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/${ASSET_NAME}"

info "Detected Platform: ${BOLD}${OS} (${ARCH})${RESET}"
info "Target: ${BOLD}${TARGET}${RESET}"

# 3. Check for existing installation
if command -v openinvar &> /dev/null; then
    EXISTING_PATH=$(command -v openinvar)
    OLD_VERSION=$(openinvar --version 2>/dev/null | awk '{print $NF}' || echo "unknown")
    step "Updating existing installation..."
    info "Found OpenInvar ${BOLD}${OLD_VERSION}${RESET} at ${BOLD}${EXISTING_PATH}${RESET}"
    INSTALL_TARGET="${EXISTING_PATH}"
else
    step "Preparing new installation..."
    INSTALL_TARGET="${INSTALL_DIR}/openinvar"
fi

# 4. Create temp directory
TMP_DIR=$(mktemp -d -t openinvar-install-XXXXXXXXXX)
trap 'rm -rf "${TMP_DIR}"' EXIT

# 5. Download
step "Downloading latest release..."
info "URL: ${GREY}${DOWNLOAD_URL}${RESET}"

if command -v curl &> /dev/null; then
    curl -fsSL "${DOWNLOAD_URL}" -o "${TMP_DIR}/${ASSET_NAME}"
elif command -v wget &> /dev/null; then
    wget -qO "${TMP_DIR}/${ASSET_NAME}" "${DOWNLOAD_URL}"
else
    error "Neither curl nor wget is installed."
fi
success "Download complete."

# 6. Extract
step "Extracting binary..."
tar -xzf "${TMP_DIR}/${ASSET_NAME}" -C "${TMP_DIR}"
chmod +x "${TMP_DIR}/openinvar"
success "Extraction complete."

# 7. Install
step "Finalizing installation..."
mkdir -p "$(dirname "${INSTALL_TARGET}")"

# Move binary (handle permission issues)
if [ -w "$(dirname "${INSTALL_TARGET}")" ]; then
    mv "${TMP_DIR}/openinvar" "${INSTALL_TARGET}"
else
    warn "Root privileges required to install to $(dirname "${INSTALL_TARGET}")"
    sudo mv "${TMP_DIR}/openinvar" "${INSTALL_TARGET}"
fi

# 8. Verification
NEW_VERSION=$("${INSTALL_TARGET}" --version | awk '{print $NF}')
success "OpenInvar ${BOLD}${NEW_VERSION}${RESET} has been installed!"

# 9. PATH Check
if [[ ":$PATH:" != *":$(dirname "${INSTALL_TARGET}"):"* ]]; then
    printf "\n"
    warn "${BOLD}Installation directory is not in your PATH!${RESET}"
    info "To use OpenInvar from anywhere, add this to your shell profile (.bashrc, .zshrc, etc.):"
    printf "\n  ${MAGENTA}export PATH=\"$(dirname "${INSTALL_TARGET}"):\$PATH\"${RESET}\n\n"
else
    printf "\n"
    # No trailing \n: `success` prints through %s, which does not interpret
    # escapes, so one here reaches the terminal as the two characters.
    success "${BOLD}OpenInvar is ready!${RESET} Run '${BOLD}openinvar --help${RESET}' to get started."
fi

printf "${BLUE}==========================================${RESET}\n"
