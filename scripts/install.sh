#!/bin/sh
# Install the `jira` binary from GitHub Releases (Linux, macOS, Git Bash).
#
#   sh -c "$(curl -fsSL https://raw.githubusercontent.com/wuyan19/jira-rs/main/scripts/install.sh)"
#
# Environment overrides:
#   JIRA_VERSION         tag to install, e.g. v0.1.1 (default: latest release)
#   JIRA_DOWNLOAD_BASE   prefix in front of https://github.com for asset
#                        downloads, e.g. a GitHub accelerator
#                        (default: https://github.com)
#   JIRA_INSTALL_DIR     install directory (default: ~/.local/bin)
#
# Version lookup always goes to github.com directly: accelerators reliably
# proxy /releases/download/<tag>/<file> but not necessarily the
# /releases/latest redirect. Needs curl or wget, plus sha256sum, shasum or
# openssl (macOS ships the latter two).

set -eu

REPO="wuyan19/jira-rs"
GITHUB="https://github.com"

VERSION="${JIRA_VERSION:-}"
BASE="${JIRA_DOWNLOAD_BASE:-$GITHUB}"
INSTALL_DIR="${JIRA_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
fail() { printf 'jira install: %s\n' "$*" >&2; exit 1; }

[ -n "${HOME:-}" ] || [ -n "${JIRA_INSTALL_DIR:-}" ] || fail "HOME is not set; use JIRA_INSTALL_DIR"
BASE="${BASE%/}"
[ -n "$BASE" ] || fail "JIRA_DOWNLOAD_BASE is empty"

has() { command -v "$1" >/dev/null 2>&1; }

# download <url> <dest-file>   (never follows a version redirect for assets)
download() {
    if has curl; then
        curl -fsSL --connect-timeout 15 --retry 3 -o "$2" "$1"
    else
        wget -q -T 15 --tries=3 -O "$2" "$1"
    fi
}

# ---- platform ---------------------------------------------------------------

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    MINGW*|MSYS*|CYGWIN*) os=windows ;;
    *) fail "unsupported OS '$os'; build from source with: cargo install --git $GITHUB/$REPO" ;;
esac
case "$arch" in
    x86_64|amd64) arch=x86_64 ;;   # Linux says x86_64, Git Bash may say amd64
    aarch64|arm64) arch=aarch64 ;; # macOS says arm64
    *) fail "unsupported CPU architecture '$arch'" ;;
esac

case "$os-$arch" in
    linux-x86_64)   target=x86_64-unknown-linux-musl ;;
    linux-aarch64)  target=aarch64-unknown-linux-musl ;;
    darwin-x86_64)  target=x86_64-apple-darwin ;;
    darwin-aarch64) target=aarch64-apple-darwin ;;
    windows-x86_64) target=x86_64-pc-windows-msvc ;;
    *) fail "no prebuilt binary for $os-$arch; build from source with: cargo install --git $GITHUB/$REPO" ;;
esac
exe=""
[ "$os" = windows ] && exe=.exe

has curl || has wget || fail "need curl or wget"

# ---- version ----------------------------------------------------------------

# Latest tag from the redirect target of /releases/latest: one request,
# no API rate limit.
tag_from_redirect() {
    if has curl; then
        curl -fsSI --connect-timeout 15 --retry 2 "$GITHUB/$REPO/releases/latest" 2>/dev/null \
            | tr -d '\r' \
            | sed -n 's/^[Ll]ocation: .*\/releases\/tag\/\(v[^/? ][^ ]*\).*/\1/p' \
            | tail -n 1
    else
        wget -q -T 15 --spider --server-response "$GITHUB/$REPO/releases/latest" 2>&1 \
            | tr -d '\r' \
            | sed -n 's/^[[:space:]]*[Ll]ocation: .*\/releases\/tag\/\(v[^/? ][^ ]*\).*/\1/p' \
            | tail -n 1
    fi
}

tag_from_api() {
    if has curl; then
        curl -fsSL --connect-timeout 15 -H 'Accept: application/vnd.github+json' \
            "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null
    else
        wget -q -T 15 -O - --header='Accept: application/vnd.github+json' \
            "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null
    fi | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

if [ -n "$VERSION" ]; then
    case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;;
    esac
else
    VERSION="$(tag_from_redirect)"
    [ -n "$VERSION" ] || VERSION="$(tag_from_api)"
    [ -n "$VERSION" ] || fail "cannot determine the latest release (is github.com reachable?); set JIRA_VERSION, e.g. JIRA_VERSION=v0.1.1"
fi

asset="jira-${VERSION#v}-${target}${exe}"
root="$BASE/$REPO/releases/download/$VERSION"

# ---- download and verify ----------------------------------------------------

mkdir -p "$INSTALL_DIR"
tmp="$INSTALL_DIR/.jira.tmp.$$"
sum="$tmp.sha256"
trap 'rm -f "$tmp" "$sum"' EXIT

say "Installing jira $VERSION ($target) to $INSTALL_DIR"
download "$root/$asset" "$tmp" || fail "download failed: $root/$asset (try JIRA_DOWNLOAD_BASE=<GitHub accelerator prefix>https://github.com)"

if [ "${JIRA_SKIP_VERIFY:-}" = "1" ]; then
    say "WARNING: JIRA_SKIP_VERIFY=1, skipping checksum verification"
else
    download "$root/$asset.sha256" "$sum" || fail "download failed: $root/$asset.sha256"
    want=$(head -n 1 "$sum" | cut -d' ' -f1 | tr 'A-F' 'a-f')
    if has sha256sum; then
        got=$(sha256sum "$tmp" | cut -d' ' -f1)
    elif has shasum; then
        got=$(shasum -a 256 "$tmp" | cut -d' ' -f1)   # macOS
    elif has openssl; then
        got=$(openssl dgst -sha256 "$tmp" | sed 's/.*[ =]//')
    else
        fail "no sha256 tool found (sha256sum, shasum, openssl); set JIRA_SKIP_VERIFY=1 to skip"
    fi
    [ "$got" = "$want" ] || fail "checksum mismatch for $asset (got $got, want $want); the download may be truncated or tampered with, try another JIRA_DOWNLOAD_BASE"
fi

chmod 755 "$tmp"

# ---- install ----------------------------------------------------------------

dest="$INSTALL_DIR/jira$exe"
if [ "$os" = windows ] && [ -f "$dest" ]; then
    # Windows cannot overwrite a running .exe but does allow renaming it
    # (e.g. a `jira mcp` process owned by an MCP client).
    rm -f "$dest.old"
    mv -f "$dest" "$dest.old" 2>/dev/null || true
fi
if ! mv -f "$tmp" "$dest"; then
    fail "cannot write to $dest; use JIRA_INSTALL_DIR=<writable directory>"
fi
# Remains only while a still-running process maps the old image.
rm -f "$dest.old" 2>/dev/null || true
trap - EXIT

# Run it right away so a broken download surfaces here, not later.
"$dest" --version

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        say ""
        say "$INSTALL_DIR is not on your PATH. Add it (e.g. to ~/.bashrc or ~/.zshrc), then open a new shell:"
        say "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac
