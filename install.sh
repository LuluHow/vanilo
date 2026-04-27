#!/bin/sh
set -e

# vanilo installer
# Usage: curl -fsSL https://raw.githubusercontent.com/LuluHow/vanilo/main/install.sh | sh

REPO="https://github.com/LuluHow/vanilo"
BIN_NAME="vanilo"
INSTALL_DIR="${VANILO_INSTALL_DIR:-/usr/local/bin}"

main() {
    need_cmd curl

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  target_os="linux" ;;
        Darwin) target_os="macos" ;;
        *)      err "unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)  target_arch="x86_64" ;;
        arm64|aarch64) target_arch="arm64" ;;
        *)             err "unsupported architecture: $arch" ;;
    esac

    artifact="${BIN_NAME}-${target_os}-${target_arch}"
    url="${REPO}/releases/download/latest/${artifact}"

    if curl --output /dev/null --silent --head --fail "$url"; then
        echo "downloading ${BIN_NAME} (${target_os}-${target_arch})..."
        tmp="$(mktemp -d)"
        curl -fsSL "$url" -o "$tmp/$BIN_NAME"
        chmod +x "$tmp/$BIN_NAME"
        install_bin "$tmp/$BIN_NAME"
        rm -rf "$tmp"
    else
        # Fallback: build from source
        echo "no pre-built binary for ${target_os}-${target_arch}, building from source..."
        need_cmd cargo
        cargo install --git "$REPO" --locked
        echo "installed via cargo to ~/.cargo/bin/${BIN_NAME}"
        return
    fi

    echo "installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
}

install_bin() {
    if [ -w "$INSTALL_DIR" ]; then
        mv "$1" "${INSTALL_DIR}/${BIN_NAME}"
    else
        echo "installing to ${INSTALL_DIR} (requires sudo)..."
        sudo mv "$1" "${INSTALL_DIR}/${BIN_NAME}"
    fi
    chmod +x "${INSTALL_DIR}/${BIN_NAME}"
}

need_cmd() {
    if ! command -v "$1" > /dev/null 2>&1; then
        err "need '$1' (command not found)"
    fi
}

err() {
    echo "error: $1" >&2
    exit 1
}

main
