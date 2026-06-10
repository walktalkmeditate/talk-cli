#!/bin/sh
# talk installer (macOS / Linux). Downloads the latest release binary, verifies its
# checksum, and installs it to ~/.local/bin. talk fetches its speech models on first
# run (`talk download models`, ~330 MB) — the binary itself ships without them.
set -eu

REPO="walktalkmeditate/talk-cli"
BIN="talk"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Darwin) os_id="apple-darwin" ;;
    Linux) os_id="unknown-linux-gnu" ;;
    *) echo "talk: unsupported OS '$os' — on Windows use install.ps1" >&2; exit 1 ;;
esac
case "$arch" in
    x86_64 | amd64) arch_id="x86_64" ;;
    arm64 | aarch64) arch_id="aarch64" ;;
    *) echo "talk: unsupported architecture '$arch'" >&2; exit 1 ;;
esac
target="${arch_id}-${os_id}"

api="https://api.github.com/repos/${REPO}/releases/latest"
# Quote the auth header as a single argument; an unquoted variable would
# word-split "Bearer <token>" and leak the token into curl's arguments.
if [ -n "${GITHUB_TOKEN:-}" ]; then
    tag="$(curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" "$api" | grep '"tag_name"' | head -1 | cut -d '"' -f4)"
else
    tag="$(curl -fsSL "$api" | grep '"tag_name"' | head -1 | cut -d '"' -f4)"
fi
if [ -z "$tag" ]; then
    echo "talk: could not find the latest release (GitHub rate limit? set GITHUB_TOKEN)" >&2
    exit 1
fi

base="https://github.com/${REPO}/releases/download/${tag}"
archive="${BIN}-${target}.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "${base}/${archive}" -o "${tmp}/${archive}"
curl -fsSL "${base}/checksums.txt" -o "${tmp}/checksums.txt"

expected="$(grep " ${archive}\$" "${tmp}/checksums.txt" | awk '{print $1}')"
actual="$(sha256 "${tmp}/${archive}")"
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    echo "talk: checksum verification failed — aborting" >&2
    exit 1
fi

tar -xzf "${tmp}/${archive}" -C "$tmp"
dest="${HOME}/.local/bin"
mkdir -p "$dest"
install -m 0755 "${tmp}/${BIN}-${target}/${BIN}" "${dest}/${BIN}"

echo "Installed ${BIN} ${tag} to ${dest}/${BIN}"
echo "Run 'talk download models' once (~330 MB) before your first reflection."
case ":$PATH:" in
    *":$dest:"*) ;;
    *) echo "Add ${dest} to your PATH to run 'talk'." ;;
esac
