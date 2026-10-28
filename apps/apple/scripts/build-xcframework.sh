#!/usr/bin/env bash
# Builds the Gentbox core as `GentboxCoreFFI.xcframework` for macOS and
# generates the Swift bindings into the `GentboxCore` package.
#
# Two builds:
#   release  arm64 + x86_64, optimised. The default on CI (`CI` set) and with
#            `--release`: what ships and what the app runs day to day.
#   debug    arm64 only, unoptimised. The local default: it compiles in a
#            fraction of the time and is enough for `xcodebuild test`.
#
# A build is skipped when the core sources, the compiler, the build settings
# from the environment and the build kind match the last one in this worktree
# (`--force` builds anyway).
#
# Outputs (all gitignored):
#   apps/apple/GentboxCore/GentboxCoreFFI.xcframework
#   apps/apple/GentboxCore/Sources/GentboxCore/Generated/GentboxCore.swift
#   apps/apple/GentboxCore/.xcframework-stamp
#
# The bindings generator is `crates/gentbox-uniffi-bindgen`, built from the
# same pinned `uniffi` version `gentbox-ffi` links against. Cargo builds into
# `core/target` of this worktree: never point CARGO_TARGET_DIR at a directory
# several worktrees share (Cargo would mix their build products).
#
# Usage: apps/apple/scripts/build-xcframework.sh [--release | --debug] [--force]
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
core_dir="$repo_root/core"
package_dir="$repo_root/apps/apple/GentboxCore"
generated_dir="$package_dir/Sources/GentboxCore/Generated"
xcframework="$package_dir/GentboxCoreFFI.xcframework"
stamp_file="$package_dir/.xcframework-stamp"
module_name="GentboxCoreFFI"
library="libgentbox_ffi.a"

if [[ -z "${CI:-}" ]]; then
	unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR
fi
target_dir="${CARGO_TARGET_DIR:-$core_dir/target}"

profile=debug
if [[ -n "${CI:-}" ]]; then
	profile=release
fi
force=false
for argument in "$@"; do
	case "$argument" in
		--release) profile=release ;;
		--debug) profile=debug ;;
		--force) force=true ;;
		*)
			echo "usage: apps/apple/scripts/build-xcframework.sh [--release | --debug] [--force]" >&2
			exit 64
			;;
	esac
done

if [[ "$profile" == release ]]; then
	targets=(aarch64-apple-darwin x86_64-apple-darwin)
	cargo_profile_flag=(--release)
else
	targets=(aarch64-apple-darwin)
	cargo_profile_flag=()
fi

# C dependencies (the bundled SQLite) build for the app's deployment target,
# not the host SDK's, so the app links without version warnings.
export MACOSX_DEPLOYMENT_TARGET=15.0

# Everything the XCFramework is built from, hashed by content: the build kind,
# the compiler, the environment Cargo reads at compile time, tracked and
# untracked files under core/, the toolchain pin and this script.
function source_fingerprint() {
	{
		echo "$profile ${targets[*]}"
		(cd "$core_dir" && rustc -vV) || true
		env | grep -E '^(GENTBOX_|RUSTFLAGS=|CARGO_ENCODED_RUSTFLAGS=|RUSTC_WRAPPER=|RUSTUP_TOOLCHAIN=|CARGO_PROFILE_|CARGO_BUILD_)' | sort || true
		# A tracked file deleted in the working tree has nothing to hash.
		(cd "$repo_root" && git ls-files -z --cached --others --exclude-standard -- \
			core rust-toolchain.toml apps/apple/scripts/build-xcframework.sh docs/schema |
			xargs -0 shasum -a 256 2>/dev/null) || true
	} | shasum -a 256 | cut -d ' ' -f 1
}

fingerprint="$(source_fingerprint)"
if [[ "$force" == false && -d "$xcframework" && -d "$generated_dir" && -f "$stamp_file" &&
	"$(cat "$stamp_file")" == "$fingerprint" ]]; then
	echo "XCFramework is up to date ($profile, ${targets[*]}); --force rebuilds it"
	exit 0
fi

# Two builds in one worktree write the same library: hold a lock from the
# build until the library is copied out.
lock_dir="$target_dir/xcframework.lock"
mkdir -p "$target_dir"
until mkdir "$lock_dir" 2>/dev/null; do
	lock_owner="$(cat "$lock_dir/pid" 2>/dev/null || true)"
	if [[ -n "$lock_owner" ]] && ! kill -0 "$lock_owner" 2>/dev/null; then
		rm -rf "$lock_dir"
		continue
	fi
	echo "==> waiting for another XCFramework build in this worktree (pid ${lock_owner:-unknown})"
	sleep 2
done
echo $$ >"$lock_dir/pid"

build_dir="$(mktemp -d "${TMPDIR:-/tmp}/gentbox-xcframework.XXXXXX")"
trap 'rm -rf "$lock_dir" "$build_dir"' EXIT
rm -rf "$xcframework" "$generated_dir" "$stamp_file"
mkdir -p "$build_dir/headers" "$generated_dir"

slices=()
for target in "${targets[@]}"; do
	rustup target add "$target" >/dev/null 2>&1 || true
	echo "==> cargo build -p gentbox-ffi --target $target ($profile)"
	# `${a[@]+...}`: an empty array is "unbound" under `set -u` in macOS bash 3.2.
	(cd "$core_dir" && cargo build -p gentbox-ffi --lib --target "$target" ${cargo_profile_flag[@]+"${cargo_profile_flag[@]}"})
	cp "$target_dir/$target/$profile/$library" "$build_dir/$target-$library"
	slices+=("$build_dir/$target-$library")
done

combined="$build_dir/$library"
echo "==> lipo -create -> $combined"
lipo -create "${slices[@]}" -output "$combined"

echo "==> uniffi-bindgen-swift (from crates/gentbox-uniffi-bindgen)"
(cd "$core_dir" && cargo run --quiet -p gentbox-uniffi-bindgen --bin uniffi-bindgen-swift -- \
	--swift-sources --module-name "$module_name" "$combined" "$generated_dir")
(cd "$core_dir" && cargo run --quiet -p gentbox-uniffi-bindgen --bin uniffi-bindgen-swift -- \
	--headers --modulemap --module-name "$module_name" --modulemap-filename module.modulemap \
	"$combined" "$build_dir/headers")

echo "==> xcodebuild -create-xcframework -> $xcframework"
xcodebuild -create-xcframework \
	-library "$combined" \
	-headers "$build_dir/headers" \
	-output "$xcframework" >/dev/null

echo "$fingerprint" >"$stamp_file"
echo "Built $xcframework ($profile, ${targets[*]})"
echo "Generated $(ls "$generated_dir")"
