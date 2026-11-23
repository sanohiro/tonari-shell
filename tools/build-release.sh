#!/bin/sh
# Build the single executable for this host. Extra arguments go to Cargo.
set -eu
cd "$(dirname "$0")/.."

# musl 1.2.4 added packed relocations to its static PIE startup code.
# Unknown or older archives can use ordinary relocations.
supports_relr() {
    ar p "$1" version.lo 2>/dev/null | strings | awk -F. '
        /^[0-9]+\.[0-9]+\.[0-9]+($|[-+_])/ {
            ok = ($1 > 1 || ($1 == 1 && ($2 > 2 || ($2 == 2 && $3 + 0 >= 4))))
            exit
        }
        END { exit !ok }'
}

host=$(rustc -vV | sed -n 's/^host: //p')
level=s
case "$host" in
    aarch64-apple-darwin)
        tuning='-C link-arg=-Wl,-dead_strip_dylibs -C target-cpu=native -C llvm-args=--enable-machine-outliner=always -C llvm-args=--outliner-benefit-threshold=384'
        ;;
    aarch64-*-linux-gnu|aarch64-*-linux-musl)
        tuning='-C target-cpu=native'
        ;;
    x86_64-*-linux-gnu|x86_64-*-linux-musl)
        tuning='-C target-cpu=x86-64-v3'
        level=z
        ;;
    *)
        printf '%s\n' "No measured release profile for $host" >&2
        exit 1
        ;;
esac

export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$tuning"
export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-$level}"
case "$host" in
    *-apple-darwin)
        cargo build --release --locked "$@"
        binary="${CARGO_TARGET_DIR:-target}/release/tonari"
        ;;
    *-linux-*)
        target=$host
        case "$host" in
            *-gnu) target="${host%%-*}-unknown-linux-musl" ;;
        esac
        libdir=$(rustc --print target-libdir --target "$target")
        if [ ! -d "$libdir" ]; then
            echo "Install the Rust target first: rustup target add $target" >&2
            exit 1
        fi
        export RUSTFLAGS="$RUSTFLAGS -C target-feature=+crt-static -C force-unwind-tables=no"
        set -- --release --locked --target "$target" "$@" --
        crt=$libdir/self-contained
        if [ -f "$crt/libc.a" ]; then
            # Rustup supplies both the linker and the target's musl runtime.
            lld=$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld
            set -- "$@" -C "linker=$lld" -C link-arg=--icf=all
            if supports_relr "$crt/libc.a"; then
                set -- "$@" -C link-arg=-z -C link-arg=pack-relative-relocs
            fi
            case "$target" in
                aarch64-*)
                    # This rustc target defaults to crt1.o (fixed address).
                    # Use musl's self-relocating startup for a static PIE.
                    set -- "$@" -C link-self-contained=no -L "native=$crt" \
                        -C link-arg=-pie -C link-arg=--no-dynamic-linker \
                        -C "link-arg=$crt/rcrt1.o" -C "link-arg=$crt/crti.o" \
                        -C "link-arg=$crt/crtbeginS.o" -C "link-arg=$crt/crtendS.o" -C "link-arg=$crt/crtn.o"
                    ;;
                *) set -- "$@" -C link-self-contained=yes ;;
            esac
        else
            # Alpine's native toolchain supplies the musl objects through cc.
            if [ "$target" != "$host" ]; then
                echo "Missing musl runtime in $crt for $target" >&2
                exit 1
            fi
            flags=-Wl,--icf=all
            if supports_relr "$(cc -print-file-name=libc.a)"; then
                flags=$flags,-z,pack-relative-relocs
            fi
            set -- "$@" -C link-arg=-fuse-ld=lld -C "link-arg=$flags"
        fi
        cargo rustc "$@"
        binary="${CARGO_TARGET_DIR:-target}/$target/release/tonari"
        if command -v objcopy >/dev/null 2>&1; then
            objcopy --remove-section=.comment "$binary"
        fi
        headers=$(LC_ALL=C readelf -lWd "$binary")
        if ! printf '%s\n' "$headers" | grep -Eq 'FLAGS_1.*PIE' ||
            printf '%s\n' "$headers" | grep -Eq 'INTERP|\(NEEDED\)'; then
            echo "Expected a static PIE with no shared libraries: $binary" >&2
            exit 1
        fi
        ;;
esac
printf '%s\n' "$binary"
