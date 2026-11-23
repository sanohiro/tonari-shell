#!/bin/sh
# Exercise the system account database and login PTY inside a disposable distro image.
set -eu
if [ "$#" -ne 2 ]; then
    echo "usage: sh tests/compat/login_account.sh LINUX_EXECUTABLE DISTRO_IMAGE" >&2
    exit 2
fi
binary=$(realpath "$1")
test -x "$binary"
docker run --rm -i --init --user root -v "$binary:/artifact:ro" --entrypoint /bin/sh "$2" -se <<'GUEST'
install -D -m 755 /artifact /opt/tonari/libexec/tonari/bash
printf '%s\n' /opt/tonari/libexec/tonari/bash >> /etc/shells
useradd -m -s /opt/tonari/libexec/tonari/bash shellfixture
home=/home/shellfixture
printf '%s\n' 'export TONARI_PROFILE_SEEN=profile' >> "$home/.profile"
printf '%s\n' 'printf "%s\n" logout > "$HOME/logout-observed"' >> "$home/.bash_logout"
status=0
printf '%s\n' 'printf "%s\n" "$SHELL" "$0" "$TONARI_PROFILE_SEEN" "$HISTCONTROL" > "$HOME/login-observed"' \
    'exit 19' | su --pty --login shellfixture >/tmp/session-log 2>&1 || status=$?
test "$status" -eq 19
printf '%s\n' /opt/tonari/libexec/tonari/bash -bash profile ignoreboth > /tmp/expected
diff -u /tmp/expected "$home/login-observed"
test "$(cat "$home/logout-observed")" = logout
printf '%s\n' 'Account login, profile, .bashrc, logout and exit status passed'
GUEST
