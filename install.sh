#!/bin/sh
# Installs the latest KivaDB release on Linux or macOS.
#
#   curl -sSL https://raw.githubusercontent.com/fomadev/kiva-db/main/install.sh | bash
#
# Options, as environment variables:
#   KIVADB_INSTALL_DIR   where to put the executable (default: /usr/local/bin)
#   KIVADB_VERSION       a version to install instead of the latest, e.g. 1.1.0
#
# Copyright (c) 2026 Fordima Inc. Licensed under the Fordima Public License (FPL) v1.0.

set -eu

REPO="fomadev/kiva-db"
INSTALL_DIR="${KIVADB_INSTALL_DIR:-/usr/local/bin}"

say() { printf '%s\n' "$*"; }
fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }

# --- Which archive for this system ---------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Linux/x86_64 | Linux/amd64) asset="kivadb_linux_x64.tar.gz" ;;
  Darwin/arm64 | Darwin/aarch64) asset="kivadb_mac_arm64.tar.gz" ;;
  *)
    fail "no ready-made executable for $os/$arch. Build KivaDB from the source code: https://github.com/$REPO/blob/main/DOCUMENTATION.md#13-build-from-the-source-code"
    ;;
esac

if [ -n "${KIVADB_VERSION:-}" ]; then
  base="https://github.com/$REPO/releases/download/v${KIVADB_VERSION#v}"
else
  base="https://github.com/$REPO/releases/latest/download"
fi

command -v curl >/dev/null 2>&1 || fail "curl is required."
command -v tar >/dev/null 2>&1 || fail "tar is required."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

# --- Download and verify --------------------------------------------------
say "Downloading $asset..."
curl -fsSL "$base/$asset" -o "$tmp/$asset" || fail "cannot download $base/$asset (check the Internet connection and the version)."

if curl -fsSL "$base/SHA256SUMS.txt" -o "$tmp/SHA256SUMS.txt" 2>/dev/null; then
  expected="$(grep " \*\{0,1\}$asset\$" "$tmp/SHA256SUMS.txt" | awk '{print $1}')"
  [ -n "$expected" ] || fail "SHA256SUMS.txt has no entry for $asset."
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')"
  fi
  [ "$actual" = "$expected" ] || fail "checksum mismatch for $asset: the download was damaged or altered."
  say "Checksum verified (SHA-256)."
else
  say "Warning: no SHA256SUMS.txt in this release, the download cannot be verified."
fi

tar -xzf "$tmp/$asset" -C "$tmp" kivadb || fail "the archive does not contain kivadb."
chmod 755 "$tmp/kivadb"

# --- Install --------------------------------------------------------------
if [ -d "$INSTALL_DIR" ] && [ -w "$INSTALL_DIR" ]; then
  sudo=""
else
  command -v sudo >/dev/null 2>&1 || fail "$INSTALL_DIR is not writable. Run as root, or set KIVADB_INSTALL_DIR to a folder you own."
  sudo="sudo"
  say "Installing into $INSTALL_DIR needs administrator rights (sudo)."
fi
$sudo mkdir -p "$INSTALL_DIR"
$sudo install -m 755 "$tmp/kivadb" "$INSTALL_DIR/kivadb"

# --- Check ----------------------------------------------------------------
version="$("$INSTALL_DIR/kivadb" --version 2>/dev/null)" || fail "kivadb was installed in $INSTALL_DIR but does not run on this system."
say "$version installed in $INSTALL_DIR/kivadb"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) say "Start it with: kivadb start" ;;
  *) say "Add $INSTALL_DIR to your PATH, then start it with: kivadb start" ;;
esac
say "Later, update it with: kivadb update"
