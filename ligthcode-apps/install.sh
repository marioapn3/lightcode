#!/usr/bin/env bash
#
# LightCode installer
#
# Usage:
#   curl -fsSL https://<host>/install.sh | bash
#   ./install.sh            # from a clone of this repository
#
# Behavior:
#   1. If a prebuilt binary for this OS/arch is published in GitHub Releases,
#      download it (fast path — no Rust toolchain needed).
#   2. Otherwise fall back to `cargo install` (requires the Rust toolchain).
#
# Prebuilt asset naming convention: lightcode-<os>-<arch> with
#   os   = linux | darwin
#   arch = x86_64 | aarch64
# (the release pipeline must publish assets under those names).
#
# Env overrides:
#   LIGHTCODE_REPO        GitHub repo "owner/repo" (default: marioapn3/lightcode)
#   LIGHTCODE_VERSION     release tag, e.g. "v0.1.0" (default: latest release)
#   LIGHTCODE_BIN_DIR     install directory (default: ~/.cargo/bin or ~/.local/bin)
#   LIGHTCODE_FORCE_CARGO set to "1" to always build from source

set -euo pipefail

REPO="${LIGHTCODE_REPO:-marioapn3/lightcode}"
VERSION="${LIGHTCODE_VERSION:-latest}"
BIN="lightcode"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '%s\n' "$*"; }
die() { log "error: $*" >&2; exit 1; }

# --- platform detection -------------------------------------------------------
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m | tr '[:upper:]' '[:lower:]')"

case "$ARCH" in
  x86_64 | amd64)  ARCH="x86_64" ;;
  aarch64 | arm64) ARCH="aarch64" ;;
  *)               ARCH="" ;;
esac
case "$OS" in
  linux | darwin) ;;
  *) OS="" ;;
esac

# --- install directory --------------------------------------------------------
if [[ -n "${LIGHTCODE_BIN_DIR:-}" ]]; then
  BIN_DIR="$LIGHTCODE_BIN_DIR"
elif [[ -d "$HOME/.cargo/bin" && ":$PATH:" == *":$HOME/.cargo/bin:"* ]]; then
  BIN_DIR="$HOME/.cargo/bin"
elif [[ -d "$HOME/.local/bin" ]]; then
  BIN_DIR="$HOME/.local/bin"
elif [[ -d "$HOME/bin" ]]; then
  BIN_DIR="$HOME/bin"
else
  BIN_DIR="$HOME/.cargo/bin"
fi

# --- fast path: prebuilt binary ------------------------------------------------
install_prebuilt() {
  if ! command -v curl >/dev/null 2>&1; then
    return 1
  fi
  local url
  if [[ "$VERSION" == "latest" ]]; then
    url="https://github.com/$REPO/releases/latest/download/$BIN-$OS-$ARCH"
  else
    url="https://github.com/$REPO/releases/download/$VERSION/$BIN-$OS-$ARCH"
  fi
  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  if ! curl -fsSL --retry 3 --connect-timeout 10 "$url" -o "$tmp/$BIN" 2>/dev/null; then
    log "  no prebuilt binary at: $url"
    return 1
  fi
  chmod +x "$tmp/$BIN"
  if ! "$tmp/$BIN" --version >/dev/null 2>&1; then
    log "  downloaded binary failed a sanity check"
    return 1
  fi
  mkdir -p "$BIN_DIR"
  mv "$tmp/$BIN" "$BIN_DIR/$BIN"
  log "✓ installed $BIN ($VERSION, $OS/$ARCH) → $BIN_DIR/$BIN"
}

# --- slow path: cargo install ---------------------------------------------------
install_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    die "no prebuilt binary for this platform and 'cargo' was not found. Install Rust first: https://rustup.rs"
  fi
  local root="${BIN_DIR%/bin}"
  if [[ -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    log "building $BIN from source in $SCRIPT_DIR with cargo (a few minutes)..."
    cargo install --path "$SCRIPT_DIR" --locked --root "$root"
  else
    log "building $BIN from https://github.com/$REPO with cargo (a few minutes)..."
    cargo install --git "https://github.com/$REPO" --locked --root "$root"
  fi
  log "✓ installed $BIN → $BIN_DIR/$BIN"
}

# --- main -------------------------------------------------------------------------
if [[ "${LIGHTCODE_FORCE_CARGO:-}" == "1" ]]; then
  install_from_source
elif [[ -n "$OS" && -n "$ARCH" ]]; then
  if ! install_prebuilt; then
    install_from_source
  fi
else
  die "unsupported platform OS='$(uname -s)' ARCH='$(uname -m)'. Set LIGHTCODE_FORCE_CARGO=1 to build from source."
fi

if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
  log ""
  log "Add $BIN_DIR to your PATH, e.g.:"
  log "  export PATH=\"$BIN_DIR:\$PATH\"   # add to ~/.zshrc or ~/.bashrc"
fi
log ""
log "Next: run 'lightcode init' to set up your provider & API key."
