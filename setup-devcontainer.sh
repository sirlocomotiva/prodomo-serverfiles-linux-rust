#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
	printf 'Error: setup-devcontainer.sh must be run as root.\n' >&2
	exit 1
fi

PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export PATH
unset APT_CONFIG CARGO_HOME CARGO_TARGET_DIR RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER LD_PRELOAD LD_LIBRARY_PATH

cd /workspace
export DEBIAN_FRONTEND=noninteractive

need_build_essential=false
need_cargo=false
need_liblua=false
need_pkgconf=false
need_rust_clippy=false
need_rustc=false
need_rustfmt=false

rustc_is_supported() {
	command -v rustc >/dev/null 2>&1 || return 1

	local release
	release=$(rustc --version --verbose 2>/dev/null | while IFS=: read -r key value; do
		if [[ ${key} == release ]]; then
			value=${value#"${value%%[![:space:]]*}"}
			printf '%s\n' "${value}"
			break
		fi
	done)

	[[ ${release} =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
	/usr/bin/dpkg --compare-versions "${release}" ge 1.85.1
}

for command_name in cc c++ make; do
	if ! command -v "${command_name}" >/dev/null 2>&1; then
		need_build_essential=true
	fi
done

command -v cargo >/dev/null 2>&1 || need_cargo=true

if ! command -v pkg-config >/dev/null 2>&1; then
	need_pkgconf=true
	need_liblua=true
elif ! pkg-config --exists lua5.4 >/dev/null 2>&1; then
	need_liblua=true
fi

if ! command -v cargo-clippy >/dev/null 2>&1 || ! cargo-clippy --version >/dev/null 2>&1; then
	need_rust_clippy=true
fi

rustc_is_supported || need_rustc=true

if ! command -v rustfmt >/dev/null 2>&1 || ! command -v cargo-fmt >/dev/null 2>&1 || ! cargo-fmt --version >/dev/null 2>&1; then
	need_rustfmt=true
fi

apt_packages=()
${need_build_essential} && apt_packages+=(build-essential)
${need_cargo} && apt_packages+=(cargo)
${need_liblua} && apt_packages+=(liblua5.4-dev)
${need_pkgconf} && apt_packages+=(pkgconf)
${need_rust_clippy} && apt_packages+=(rust-clippy)
${need_rustc} && apt_packages+=(rustc)
${need_rustfmt} && apt_packages+=(rustfmt)

if ((${#apt_packages[@]} > 0)); then
	cleanup_apt() {
		/usr/bin/apt-get clean >/dev/null 2>&1 || true
		/usr/bin/rm -rf /var/lib/apt/lists/* || true
	}
	trap cleanup_apt EXIT

	printf 'Installing missing packages: %s\n' "${apt_packages[*]}"
	/usr/bin/apt-get update
	/usr/bin/apt-get install -y --no-install-recommends "${apt_packages[@]}"

	cleanup_apt
	trap - EXIT
else
	printf 'All required packages are already available.\n'
fi

if ! rustc_is_supported; then
	printf 'Error: rustc must be stable release 1.85.1 or newer.\n' >&2
	exit 1
fi

verify_command() {
	local command_name=$1
	local capability=$2
	if ! command -v "${command_name}" >/dev/null 2>&1; then
		printf 'Error: required capability is unavailable: %s.\n' "${capability}" >&2
		exit 1
	fi
}

verify_command cargo cargo
verify_command rustfmt rustfmt
verify_command cargo-fmt 'cargo fmt'
if ! cargo-fmt --version >/dev/null 2>&1; then
	printf 'Error: required capability is unavailable: cargo fmt.\n' >&2
	exit 1
fi
verify_command cargo-clippy 'cargo clippy'
if ! cargo-clippy --version >/dev/null 2>&1; then
	printf 'Error: required capability is unavailable: cargo clippy.\n' >&2
	exit 1
fi
verify_command cc 'C compiler (cc)'
verify_command c++ 'C++ compiler (c++)'
verify_command make make
verify_command pkg-config pkg-config
if ! pkg-config --exists lua5.4; then
	printf 'Error: pkg-config metadata for lua5.4 is unavailable.\n' >&2
	exit 1
fi

lua_version=$(pkg-config --modversion lua5.4)
if [[ ! ${lua_version} =~ ^5\.4([.]|$) ]]; then
	printf 'Error: lua5.4 pkg-config module reported unsupported version: %s.\n' "${lua_version}" >&2
	exit 1
fi
if ! pkg-config --cflags --libs lua5.4 >/dev/null; then
	printf 'Error: pkg-config could not resolve compiler and linker flags for lua5.4.\n' >&2
	exit 1
fi

printf 'Development tooling verification passed.\n'
