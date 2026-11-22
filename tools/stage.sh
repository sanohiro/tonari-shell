#!/bin/sh
# Assemble the install layout under the requested prefix.
set -eu
if [ "$#" -ne 2 ]; then
    echo "usage: $0 EXECUTABLE PREFIX" >&2
    exit 2
fi
binary=$1
prefix=$2
if [ ! -f "$binary" ] || [ ! -x "$binary" ] || [ -z "$prefix" ]; then
    echo "expected an executable file and a non-empty prefix" >&2
    exit 2
fi
bin=$prefix/bin
lib=$prefix/libexec/tonari
link=../libexec/tonari/bash
mkdir -p "$bin" "$lib"
if [ -e "$bin/tonari" ] || [ -L "$bin/tonari" ]; then
    if [ ! -L "$bin/tonari" ] || [ "$(readlink "$bin/tonari")" != "$link" ]; then
        echo "another entry occupies $bin/tonari" >&2
        exit 1
    fi
fi
if [ -d "$lib/bash" ]; then
    echo "expected a file at $lib/bash" >&2
    exit 1
fi
# Rename a completed executable so existing sessions keep their original inode.
tmp=$(mktemp "$lib/.bash.XXXXXXXX")
trap 'rm -f "$tmp"' EXIT
trap 'exit 1' HUP INT TERM
cp "$binary" "$tmp"
chmod 755 "$tmp"
mv -f "$tmp" "$lib/bash"
if [ ! -L "$bin/tonari" ]; then
    ln -s "$link" "$bin/tonari"
fi
