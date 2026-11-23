#!/bin/sh
# Build the distribution executable. --host-toolchain uses the installed Rust std.
# Remaining arguments go to Cargo.
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

distribution=yes
if [ "${1-}" = --host-toolchain ]; then
    distribution=no
    shift
fi
host=$(rustc -vV | sed -n 's/^host: //p')
case "$host:$distribution" in
    aarch64-apple-darwin:yes|aarch64-*-linux-*:yes|x86_64-*-linux-*:yes)
        # Cargo requires nightly to compile std with the application's release profile.
        toolchain=nightly-2026-11-20
        case "$host" in
            *-apple-darwin) target=$host ;;
            *) target=${host%%-*}-unknown-linux-musl ;;
        esac
        install="rustup toolchain install $toolchain --profile minimal --component rust-src --target $target"
        compiler=$(rustup which --toolchain "$toolchain" rustc 2>/dev/null) || {
            echo "For a distribution build, run: $install" >&2
            echo "To use the installed compiler and prebuilt std, pass --host-toolchain." >&2
            exit 1
        }
        export PATH="${compiler%/*}:$PATH" RUSTUP_TOOLCHAIN="$toolchain"
        host=$(rustc -vV | sed -n 's/^host: //p')
        if [ ! -f "$(rustc --print sysroot)/lib/rustlib/src/rust/library/std/Cargo.toml" ] ||
            [ ! -d "$(rustc --print target-libdir --target "$target")" ]; then
            echo "Install the distribution build components: $install" >&2
            exit 1
        fi
        ;;
esac
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
        if [ "$distribution" = yes ]; then
            # Smaller std routines let the shell use the faster -Os profile.
            tuning="-Cforce-frame-pointers=yes $tuning"
            level=s
        fi
        ;;
    *)
        printf '%s\n' "No measured release profile for $host" >&2
        exit 1
        ;;
esac

if [ "$distribution" = yes ]; then
    # Shipped binaries are stripped. Keep panic locations and frame capture
    # without embedding an object-file reader and DWARF symbolizer.
    std_features=backtrace,backtrace-trace-only
    case "$host" in
        x86_64-*-linux-*) std_features=optimize_for_size,$std_features ;;
        aarch64-*) level=2 ;;
    esac
    set -- -Zbuild-std=std,panic_abort "-Zbuild-std-features=$std_features" "$@"
fi
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$tuning"
export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-$level}"
case "$host" in
    *-apple-darwin)
        binary="${CARGO_TARGET_DIR:-target}/release/tonari"
        if [ "$distribution" = yes ]; then
            set -- --target "$host" "$@"
            binary="${CARGO_TARGET_DIR:-target}/$host/release/tonari"
        fi
        cargo build --release --locked "$@"
        if [ "$(lipo -archs "$binary")" != arm64 ] ||
            [ "$(otool -L "$binary" | awk 'NR > 1 {print $1}')" != /usr/lib/libSystem.B.dylib ]; then
            echo "Expected a macOS arm64 executable linked only to libSystem: $binary" >&2
            exit 1
        fi
        if [ "$distribution" = yes ] && otool -l "$binary" | grep -Eq 'segname +__DWARF$'; then
            echo "Distribution executable contains DWARF debug information: $binary" >&2
            exit 1
        fi
        codesign --verify --strict "$binary"
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
        case "$target" in
            x86_64-*) machine='Advanced Micro Devices X86-64' ;;
            aarch64-*) machine='AArch64' ;;
        esac
        headers=$(LC_ALL=C readelf -hSlWd "$binary")
        if ! printf '%s\n' "$headers" | grep -Eq "Machine: +$machine$" ||
            ! printf '%s\n' "$headers" | grep -Eq 'FLAGS_1.*PIE' ||
            printf '%s\n' "$headers" | grep -Eq 'INTERP|\(NEEDED\)'; then
            echo "Expected a $machine static PIE with no shared libraries: $binary" >&2
            exit 1
        fi
        if [ "$distribution" = yes ] &&
            printf '%s\n' "$headers" | grep -Eq '\] +(\.symtab |\.z?debug_)'; then
            echo "Distribution executable contains symbols or debug sections: $binary" >&2
            exit 1
        fi
        ;;
esac
printf '%s\n' "$binary"
