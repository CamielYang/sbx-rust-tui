# Release builds for Windows, Linux and macOS.
#
# `just release` leaves one archive per platform in ./dist:
#   sbx-rust-tui-v0.1.0-windows.gz  -> sbx-rust-tui.exe
#   sbx-rust-tui-v0.1.0-linux.gz    -> sbx-rust-tui
#   sbx-rust-tui-v0.1.0-darwin.gz   -> sbx-rust-tui
#
# Run `just setup` once to install the Rust targets, cargo-zigbuild and zig.
# zig is used only to cross-link the macOS binary.

bin := `sed -n 's/^name = "\(.*\)"/\1/p' Cargo.toml | head -n 1`
version := `sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1`
out := "dist"
windows_target := "x86_64-pc-windows-gnu"
linux_target := "x86_64-unknown-linux-gnu"
darwin_target := "x86_64-apple-darwin"

zig_version := "0.14.1"
zig_dir := justfile_directory() / ".tools" / "zig"
export PATH := zig_dir + ":" + env_var("PATH")

cargo := "cargo build"
cargo_zig := "cargo zigbuild"

# Build every platform into ./dist (default recipe).
release: clean (build-platform windows_target "windows" ".exe" cargo) (build-platform linux_target "linux" "" cargo) (build-platform darwin_target "darwin" "" cargo_zig)

# Install everything the release build needs.
setup: install-zig
    rustup target add {{ windows_target }} {{ linux_target }} {{ darwin_target }}
    command -v cargo-zigbuild >/dev/null 2>&1 || cargo install cargo-zigbuild --locked

# Download zig locally unless it is already on PATH.
[private]
install-zig:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v zig >/dev/null 2>&1; then exit 0; fi
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64)  triple="x86_64-linux" ;;
        Darwin-x86_64) triple="x86_64-macos" ;;
        Darwin-arm64)  triple="aarch64-macos" ;;
        *)
            echo "No zig package for $(uname -s)-$(uname -m); install zig manually" >&2
            exit 1
            ;;
    esac
    mkdir -p "{{ zig_dir }}"
    curl -fsSL "https://ziglang.org/download/{{ zig_version }}/zig-${triple}-{{ zig_version }}.tar.xz" \
        | tar -xJ -C "{{ zig_dir }}" --strip-components=1

# Build one target, then gzip only the executable under its plain project name.
[private]
build-platform target platform ext build_cmd:
    #!/usr/bin/env bash
    set -euo pipefail
    {{ build_cmd }} --release --target {{ target }}
    stage="$(mktemp -d)"
    trap 'rm -rf "$stage"' EXIT
    cp "target/{{ target }}/release/{{ bin }}{{ ext }}" "$stage/{{ bin }}{{ ext }}"
    mkdir -p {{ out }}
    gzip -c "$stage/{{ bin }}{{ ext }}" > "{{ out }}/{{ bin }}-v{{ version }}-{{ platform }}.gz"

build-windows: (build-platform windows_target "windows" ".exe" cargo)
build-linux: (build-platform linux_target "linux" "" cargo)
build-darwin: (build-platform darwin_target "darwin" "" cargo_zig)

# Remove the output folder so it never contains stale files.
clean:
    rm -rf {{ out }}
