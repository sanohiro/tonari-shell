#!/bin/sh
# Build the single executable for this host. Extra arguments go to Cargo.
set -eu
cd "$(dirname "$0")/.."
host=$(rustc -vV | sed -n 's/^host: //p')
level=s
case "$host" in
    aarch64-apple-darwin)
        tuning='-C target-cpu=native -C llvm-args=--enable-machine-outliner=always -C llvm-args=--outliner-benefit-threshold=384'
        ;;
    aarch64-*-linux-gnu)
        tuning='-C target-cpu=native -C force-unwind-tables=no'
        ;;
    x86_64-*-linux-gnu)
        tuning='-C target-cpu=x86-64-v3 -C force-unwind-tables=no'
        level=z
        ;;
    aarch64-*-linux-musl)
        tuning='-C target-cpu=native -C target-feature=+crt-static -C force-unwind-tables=no'
        ;;
    *)
        printf '%s\n' "No measured release profile for $host" >&2
        exit 1
        ;;
esac

export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$tuning"
export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-$level}"
binary="${CARGO_TARGET_DIR:-target}/release/tonari"
case "$host" in
    *-apple-darwin)
        cargo build --release --locked "$@"
        ;;
    *-linux-*)
        # Rust's GNU target also ships a linker under gcc-ld.
        export PATH="$(rustc --print sysroot)/lib/rustlib/$host/bin/gcc-ld:$PATH"
        linker='-Wl,-z,pack-relative-relocs,--icf=all'
        case "$host" in
            *-musl)
                linker='-Wl,-z,pack-relative-relocs,-z,max-page-size=65536,--icf=all'
                set -- --target "$host" "$@"
                binary="${CARGO_TARGET_DIR:-target}/$host/release/tonari"
                ;;
        esac
        cargo rustc --release --locked "$@" -- -C link-arg=-fuse-ld=lld -C "link-arg=$linker"
        if command -v objcopy >/dev/null 2>&1; then
            objcopy --remove-section=.comment "$binary"
        fi
        ;;
esac
printf '%s\n' "$binary"
