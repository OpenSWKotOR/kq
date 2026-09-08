#!/bin/sh
# kq standalone installer (macOS / Linux).
#
#   curl -LsSf https://github.com/holowan-biolabs/kq/releases/latest/download/install.sh | sh
#   wget -qO- https://github.com/holowan-biolabs/kq/releases/latest/download/install.sh | sh
#
# Optional:
#   KQ_VERSION=v0.3.1     pin a release tag (default: latest)
#   KQ_INSTALL_DIR=DIR    where to put the binary (default: ~/.local/bin)
#   KQ_REPO=owner/name    GitHub repo (default: holowan-biolabs/kq)
set -eu

REPO="${KQ_REPO:-holowan-biolabs/kq}"
VERSION="${KQ_VERSION:-latest}"
BIN_DIR="${KQ_INSTALL_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"

say() { printf '%s\n' "$*"; }
err() { printf 'kq-install: %s\n' "$*" >&2; }

have() { command -v "$1" >/dev/null 2>&1; }

download() {
	url=$1
	dest=$2
	if have curl; then
		curl --proto '=https' --tlsv1.2 -fLSsS "$url" -o "$dest"
	elif have wget; then
		wget -qO "$dest" "$url"
	else
		err "need curl or wget"
		exit 1
	fi
}

# HEAD-ish: succeed only if the URL is there. curl -f fails on 404.
exists() {
	url=$1
	if have curl; then
		curl --proto '=https' --tlsv1.2 -fILsS "$url" >/dev/null 2>&1
	elif have wget; then
		wget -q --spider "$url"
	else
		return 1
	fi
}

asset_base() {
	if [ "$VERSION" = latest ]; then
		printf 'https://github.com/%s/releases/latest/download' "$REPO"
	else
		printf 'https://github.com/%s/releases/download/%s' "$REPO" "$VERSION"
	fi
}

detect_target() {
	os=$(uname -s)
	arch=$(uname -m)
	case "$os" in
	Linux)
		case "$arch" in
		x86_64 | amd64) printf '%s' "x86_64-unknown-linux-gnu" ;;
		aarch64 | arm64) printf '%s' "aarch64-unknown-linux-gnu" ;;
		*) printf '' ;;
		esac
		;;
	Darwin)
		case "$arch" in
		x86_64) printf '%s' "x86_64-apple-darwin" ;;
		arm64) printf '%s' "aarch64-apple-darwin" ;;
		*) printf '' ;;
		esac
		;;
	MINGW* | MSYS* | CYGWIN*)
		err "on Windows use:"
		err "  powershell -ExecutionPolicy ByPass -c \"irm https://github.com/${REPO}/releases/latest/download/install.ps1 | iex\""
		exit 1
		;;
	*)
		printf ''
		;;
	esac
}

verify_sha256() {
	file=$1
	sumfile=$2
	if have sha256sum; then
		got=$(sha256sum "$file" | awk '{ print $1 }')
	elif have shasum; then
		got=$(shasum -a 256 "$file" | awk '{ print $1 }')
	else
		err "no sha256sum/shasum; skipping checksum"
		return 0
	fi
	want=$(awk '{ print $1; exit }' "$sumfile")
	if [ -z "$want" ] || [ "$got" != "$want" ]; then
		err "checksum mismatch for $(basename "$file")"
		err "  wanted $want"
		err "  got    $got"
		return 1
	fi
}

install_binary() {
	target=$1
	base=$(asset_base)
	name="kq-${target}"
	url="${base}/${name}"
	sum_url="${url}.sha256"

	if ! exists "$url"; then
		return 1
	fi

	tmpdir=$(mktemp -d)
	trap 'rm -rf "$tmpdir"' EXIT
	say "downloading ${name} (${VERSION})"
	download "$url" "$tmpdir/kq"
	if exists "$sum_url"; then
		download "$sum_url" "$tmpdir/kq.sha256"
		verify_sha256 "$tmpdir/kq" "$tmpdir/kq.sha256"
	fi
	chmod 755 "$tmpdir/kq"
	mkdir -p "$BIN_DIR"
	# mv across filesystems can fail; cp+rm is the portable install.
	cp "$tmpdir/kq" "$BIN_DIR/kq"
	chmod 755 "$BIN_DIR/kq"
	rm -rf "$tmpdir"
	trap - EXIT
	return 0
}

install_from_cargo() {
	if ! have cargo; then
		return 1
	fi
	say "no prebuilt binary for this platform; building with cargo"
	if [ "$VERSION" = latest ]; then
		cargo install --git "https://github.com/${REPO}" --locked --force
	else
		cargo install --git "https://github.com/${REPO}" --tag "$VERSION" --locked --force
	fi
}

print_path_hint() {
	case ":$PATH:" in
	*":${BIN_DIR}:"*) ;;
	*)
		say ""
		say "kq is in ${BIN_DIR}, which is not on PATH. Add it:"
		say "  export PATH=\"${BIN_DIR}:\$PATH\""
		;;
	esac
}

main() {
	target=$(detect_target)
	if [ -n "$target" ] && install_binary "$target"; then
		say "installed $("$BIN_DIR/kq" --version) -> ${BIN_DIR}/kq"
		print_path_hint
		return 0
	fi
	if install_from_cargo; then
		if have kq; then
			say "installed $(kq --version)"
		fi
		return 0
	fi

	err "no prebuilt kq for $(uname -s)/$(uname -m), and cargo is not on PATH."
	err "install Rust (https://rustup.rs/), then re-run this script:"
	err "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
	exit 1
}

main
