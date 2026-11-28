"""Real terminal checks for scrolling, Unicode filters, completion scopes and input ownership."""
import argparse
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


def check(shell, home, candidate):
    rc_file(home, "zzcmd() { :; }\ncomplete -W 'apple banana' pick\n")
    for source, want in ((": <(zzc", ": <(zzcmd "), (": >(zzc", ": >(zzcmd "),
                         (": $(zzc", ": $(zzcmd "), (': "<(zzc', ': "<(zzc'),
                         (": '<(zzc", ": '<(zzc")):
        got = draft(shell, home, source.encode() + b"\t")
        assert got == want, (source, got, want)

    with (home / "rc").open("a") as rc:
        rc.write("context() { printf '<%s>' \"${COMP_WORDS[@]}\" > words; COMPREPLY=(yes); }; "
                 "complete -F context abc\n")
    line = "abc >(zzcmd a | zzcmd) h"
    assert draft(shell, home, line.encode() + b"\t") == line[:-1] + "yes "
    assert (home / "words").read_text() == "<abc><>(><zzcmd><a><|><zzcmd)><h>"

    observer["sample"](shell, home, "split-home")
    kana = "か\u3099".encode()
    with terminal(shell, home) as d:
        d.observe(b"", lambda: d.adjacent(PROMPT))
        d.observe(b"\x12" + kana, lambda: any("search" in line for line in d.screen.display))
        d.observe(b"\x7f", lambda: any("search" in line and "`':" in line for line in d.screen.display))
    if candidate:
        with terminal(shell, home) as d:
            d.observe(b"", lambda: d.adjacent(PROMPT))
            d.observe(b"pick \t\t" + kana, lambda: d.screen.cursor.x == 10 and "filter>" in d.screen.display[0] and "(no matches)" in d.screen.display[1])
            d.observe(b"\x7f", lambda: d.adjacent("filter> ") and "> apple" in d.screen.display[1])
            d.observe(b"\x07", lambda: d.adjacent(PROMPT + "pick "))

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
    with tempfile.TemporaryDirectory() as root:
        for shell, candidate in ((args.bash, False), (args.candidate, True)):
            check(str(shell.resolve()), Path(root), candidate)
            print(f"{shell} passed", flush=True)


if __name__ == "__main__":
    main()
