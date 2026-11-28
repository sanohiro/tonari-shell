"""Real terminal checks for scrolling, Unicode filters, completion scopes and input ownership."""
import argparse
import fcntl
import struct
import termios
from pathlib import Path
import runpy
import tempfile


observer = runpy.run_path(str(Path(__file__).resolve().parents[1] / "tools/ci/whole-editor.py"))
terminal, rc_file = observer["terminal"], observer["rc_file"]
PROMPT = observer["PROMPT"]


def draft(shell, home, packet):
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe(packet + b"\x14", lambda: any("SNAP-DONE" in s for s in d.screen.display))
        return (home / "draft").read_text()


def substitutions(shell, home):
    for source, want in ((": <(zzc", ": <(zzcmd "), (": >(zzc", ": >(zzcmd "),
                         (": $(zzc", ": $(zzcmd "), (': "<(zzc', ': "<(zzc'),
                         (": '<(zzc", ": '<(zzc")):
        got = draft(shell, home, source.encode() + b"\t")
        assert got == want, (source, got, want)


def completion_fields(shell, home):
    with (home / "rc").open("a") as rc:
        rc.write("context() { printf '<%s>' \"${COMP_WORDS[@]}\" > words; COMPREPLY=(yes); }; "
                 "complete -F context abc\n")
    line = "abc >(zzcmd a | zzcmd) h"
    assert draft(shell, home, line.encode() + b"\t") == line[:-1] + "yes "
    assert (home / "words").read_text() == "<abc><>(><zzcmd><a><|><zzcmd)><h>"


def wide_edits(shell, home):
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe("A🙂B".encode() + b"\x02\x02", lambda: d.adjacent(PROMPT + "A", "🙂B"))
        d.observe(b"\x04" + "雪".encode(),
                  lambda: d.screen.cursor.x == 7 and d.adjacent(PROMPT + "A雪", "B"))


def word_movement(shell, home):
    with (home / "rc").open("a") as rc:
        rc.write("bind '\"\\C-xb\":backward-word'\nbind '\"\\C-xf\":forward-word'\n")
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe("前 one two".encode() + b"\x18b", lambda: d.adjacent(PROMPT + "前 one ", "two"))
        d.observe(b"\x18f", lambda: d.adjacent(PROMPT + "前 one two"))


def cache_rename(shell, home):
    bindir = home / "bin"
    bindir.mkdir(exist_ok=True)
    old = bindir / "zzcachealpha"
    new = bindir / "zzcachebeta"
    new.unlink(missing_ok=True)
    old.touch(mode=0o755)
    with (home / "rc").open("a") as rc:
        rc.write('PATH="$HOME/bin:$PATH"\n')
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe(b"zzcachea\t", lambda: d.adjacent(PROMPT + "zzcachealpha "))
        d.observe(b"\x15", lambda: d.adjacent(PROMPT))
        old.rename(new)
        d.observe(b"zzcacheb\t", lambda: d.adjacent(PROMPT + "zzcachebeta "))


def split_pane(shell, home):
    observer["sample"](shell, home, "split-home")


def resize(shell, home):
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        value = "HEAD" + "abcdefghijklmnop" * 20 + "EDITDONE"
        d.observe(value.encode(), lambda: d.edit_complete(value, ""))
        fcntl.ioctl(d.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 6, 36, 0, 0))
        d.screen.resize(lines=6, columns=36)
        d.observe(b"\x01", lambda: d.adjacent(PROMPT, "HEADabcdefghij"))
        fcntl.ioctl(d.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        d.screen.resize(lines=24, columns=80)
        d.observe(b"\x05", lambda: d.edit_complete(value, ""))


def erase_grapheme(shell, home):
    kana = "か\u3099".encode()
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe(b"\x12" + kana, lambda: any("search" in line for line in d.lines))
        d.observe(b"\x7f", lambda: any("search" in line and "`':" in line for line in d.lines))


def picker(shell, home):
    kana = "か\u3099".encode()
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe(b"pick \t\t" + kana, lambda: d.screen.cursor.x == 10
                  and "filter>" in d.lines[0] and "(no matches)" in d.lines[1])
        d.observe(b"\x7f", lambda: d.adjacent("filter> ") and "> apple" in d.lines[1])
        d.observe(b"\x07", lambda: d.adjacent(PROMPT + "pick "))

    words = " ".join(f"group-{i:03d}" for i in range(320))
    with (home / "rc").open("a") as rc:
        rc.write(f"complete -W '{words} group-Zed' delayed\n")
    assert draft(shell, home, b"delayed \t\tZed\r") == "delayed group-Zed "
    assert draft(shell, home, b"delayed \t\tMissing\x07") == "delayed group-"


def enter_ownership(shell, home):
    # Put Enter just after both first-frame and long-stream boundaries. The
    # following line belongs to the external process reading the real terminal.
    cmd = b"/bin/sh -c 'IFS= read -r v; printf %s \"$v\" >child' #"
    for n in (257, 258, 4354):
        (home / "child").unlink(missing_ok=True)
        with terminal(shell, home) as d:
            d.observe(b"", lambda: d.adjacent(PROMPT))
            packet = cmd.ljust(n, b"x") + b"\r" + "子の入力\n".encode()
            d.observe(packet, lambda: (home / "child").exists() and d.adjacent(PROMPT))
            assert (home / "child").read_text() == "子の入力", n


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("bash", type=Path)
    args = parser.parse_args()
    partial = observer["Display"](-1, 24, 80)
    partial.stream.feed(b"ED> EDITDONETAIL\x1b[4D")
    assert not partial.edit_complete("abcdef" * 500 + "EDITDONE", "TAIL")
    partial.stream.feed(b"\r\x1b[2KED> abcEDITDONETAIL\x1b[4D")
    assert partial.edit_complete("abcEDITDONE", "TAIL")
    checks = (substitutions, completion_fields, wide_edits, word_movement, cache_rename,
              split_pane, resize, erase_grapheme, enter_ownership)
    for shell, candidate in ((args.bash, False), (args.candidate, True)):
        for check in (*checks, picker) if candidate else checks:
            with tempfile.TemporaryDirectory() as root:
                home = Path(root)
                rc_file(home, "zzcmd() { :; }\ncomplete -W 'apple banana' pick\n")
                try:
                    check(str(shell.resolve()), home)
                except (TimeoutError, AssertionError) as exc:
                    if candidate or check not in (split_pane, resize):
                        raise
                    # Readline versions differ when the draft exceeds the screen.
                    # Keep failures in the native report and check the other operations.
                    print(f"{shell} {check.__name__} reference difference: {exc}", flush=True)
                else:
                    print(f"{shell} {check.__name__} passed", flush=True)


if __name__ == "__main__":
    main()
