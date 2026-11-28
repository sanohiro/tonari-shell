"""History persistence, editor transaction and physical input boundary regressions."""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from bench_history import PROMPT, read_until, terminal


def run(shell, work, script):
    env = {"HOME": str(work), "LC_ALL": "C", "PATH": "/usr/bin:/bin"}
    result = subprocess.run([shell, "--noprofile", "--norc", "-c", script],
                            cwd=work, env=env, capture_output=True, timeout=8)
    assert result.returncode == 0, (script, result)
    return result.stdout


def completion_witnesses(tonari, bash, home):
    bindir = home / "commands"
    bindir.mkdir()
    for name in ("zzallowed", "zzshadow", "zzshape", "zzshave"):
        (bindir / name).touch(mode=0o755)
    (home / "zzblocked").touch(mode=0o600)
    (home / "zzfile").touch(mode=0o755)
    (home / "zzmiddlelong").touch()
    (home / "zzlink").symlink_to("zzfile")
    (home / "zzdir/subdir").mkdir(parents=True)
    for name in ("case-dir", "case_dir"):
        (home / name).mkdir()
    for name in ("Fold-first", "Fold-second", "case-word", "case_file", "ignore-old.bak", "ignore-only"):
        (home / name).touch()

    # Check source order as well as multiplicity, on an initial and a cached
    # query. Interactive picking still needs one row per distinct spelling.
    script = """PATH=./commands:./commands
zzshadow() { :; }; alias zzshadow=:
compgen -c zzsh; compgen -c zzsh
compgen -c ./zz; compgen -c ./zzblocked; echo "status=$?" """
    expected = run(bash, home, script)
    assert expected.count(b"zzshadow\n") == 8, expected
    assert b"./zzfile\n" in expected and b"./zzlink\n" in expected and b"./zzblocked\n" not in expected
    assert run(tonari, home, script) == expected

    # Bash 5.3 added -V. Earlier releases still provide the same reference rows
    # through ordinary compgen output.
    setup = script.split("compgen -c", 1)[0]
    rows = run(bash, home, setup + "compgen -c zzsh")
    store = setup + """compgen -V results -c zzsh && printf '%s\\n' "${results[@]}" """
    assert run(tonari, home, store) == rows

    rc = home / "completion-rc"
    rc.write_text("""PS1='tonari$ '
unset HISTFILE
PROBEAA=correct
bind 'set mark-directories off'
bind 'set completion-ignore-case on'
bind 'set completion-map-case on'
FIGNORE=.bak
snapshot() { printf '\\n<draft>%s</draft>\\n' "$READLINE_LINE"; READLINE_LINE=; READLINE_POINT=0; }
bind -x '"\\C-t":snapshot'
choose() { printf '%s\\n' "$1|$2|$3|$COMP_CWORD|$COMP_LINE|$COMP_POINT|${COMP_WORDS[*]}" >ctx; COMPREPLY=(selected); }
complete -F choose abc
remote_dir() { COMPREPLY=(remote/); }
complete -o filenames -F remote_dir remote_cmd
""")
    for shell in (bash, tonari):
        with terminal(shell, home, f"{bindir}:{bindir}", rc, prompt=PROMPT) as fd:
            def draft(line, expected, back=0):
                os.write(fd, line.encode() + b"\x02" * back + b"\t\x14")
                output = read_until(fd, b"</draft>")
                result = output.split(b"<draft>")[-1].split(b"</draft>")[0]
                assert result == expected.encode(), (shell, line, result, expected)
                read_until(fd, PROMPT, initial=output.split(b"</draft>", 1)[1])

            draft("./zzbloc", "./zzbloc")
            draft("./zzfil", "./zzfile ")
            draft("./zzlin", "./zzlink ")
            draft("A='two words' B+=x zzallo", "A='two words' B+=x zzallowed ")
            draft('A"B"=x zzallo', 'A"B"=x zzallo')
            draft(": zzmidTAIL", ": zzmiddlelongTAIL", 4)
            draft(': "zzmidTAIL"', ': "zzmiddlelongTAIL"', 5)
            draft(": 'zzmidTAIL'", ": 'zzmiddlelongTAIL'", 5)
            draft("A=x abc hiTAIL", "A=x abc selectedTAIL", 4)
            assert (home / "ctx").read_text() == "abc|hi|abc|1|abc hiTAIL|6|abc hiTAIL\n"
            draft(": zzdir/sub", ": zzdir/subdir")
            draft(": zzd/sub", ": zzdir/subdir" if shell == tonari else ": zzd/sub")
            draft(": fold-", ": Fold-")
            # Tonari keeps the typed separator when both spellings match.
            draft(": case-", ": case-" if shell == tonari else ": case")
            if shell == tonari:
                draft(": case-d", ": case-dir")
                draft(': "${PROBE}"', ': "${PROBEAA}"', 2)
            draft("remote_cmd rem", "remote_cmd remote/")
            draft(": igno", ": ignore-only ")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("tonari", type=Path)
    parser.add_argument("bash", type=Path)
    args = parser.parse_args()
    tonari, bash = args.tonari.resolve(), args.bash.resolve()
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        scripts = [
            "HISTCONTROL=ignoredups; history -s one; history -s one; history",
            "HISTCONTROL=erasedups; history -r seed; history",
            "history -s 'echo aa aa'; history -p '!!:Gs/a/x/' '!!:as/a/x/'",
            "history -r seed; history -c; echo new >>seed; history -n seed; history",
            "HISTFILE=seed; history -r seed; HISTFILESIZE=2; echo new >>seed; history -n seed; history",
            "bind 'set completion-ignore-case yes'; bind 'set completion-ignore-case'; bind 'TAB: complete'; echo $?",
        ]
        for script in scripts:
            outputs = []
            for shell in (bash, tonari):
                (home / "seed").write_text("one\ntwo\none\n")
                outputs.append(run(shell, home, script))
            assert outputs[0] == outputs[1], (script, outputs)

        seed = home / "seed"
        seed.write_text("imported\n")
        run(tonari, home, "history -s local; history -r seed; history -a journal; history -a journal")
        assert (home / "journal").read_text() == "local\n"

        seed.write_bytes(b"one\ntwo\nthree")
        run(tonari, home, "HISTFILE=seed; HISTFILESIZE=2")
        assert seed.read_bytes() == b"two\nthree"

        original = b"old\n\xff\nkept"
        seed.write_bytes(original)
        subprocess.run([tonari, "--noprofile", "--norc", "-i"], input=b": annotation\nexit 0\n",
                       cwd=home, capture_output=True, timeout=5,
                       env={"HOME": tmp, "HISTFILE": str(seed), "PATH": "/usr/bin:/bin"})
        contents = seed.read_bytes()
        assert contents.startswith(original + b"\n") and b": annotation\n" in contents

        # The owner may write an existing history file without being able to
        # import it. Preserve its earlier records on exit in that case too.
        seed.write_bytes(b"existing command\n")
        seed.chmod(0o200)
        try:
            if not os.access(seed, os.R_OK):
                subprocess.run([tonari, "--noprofile", "--norc", "-i"], input=b": added\nexit 0\n",
                               cwd=home, capture_output=True, timeout=5,
                               env={"HOME": tmp, "HISTFILE": str(seed), "PATH": "/usr/bin:/bin"})
        finally:
            seed.chmod(0o600)
        assert seed.read_bytes().startswith(b"existing command\n")

        # A program appended after a bulk-parsed group has the correct LINENO.
        growing = home / "growing.sh"
        initial = "{\n" + "# long physical line\n" * 1800 + "printf 'echo $LINENO\\n' >> \"$0\"\n}\n"
        for executable in (bash, tonari):
            growing.write_text(initial)
            result = subprocess.check_output([executable, growing], cwd=home)
            assert result == f"{initial.count(chr(10)) + 1}\n".encode(), result

        completion_witnesses(tonari, bash, home)

        bindir = home / "bin"
        bindir.mkdir()
        for name in ("test-command-a", "test-command-b"):
            file = bindir / name
            file.touch(mode=0o755)
        output = run(tonari, home, "PATH=./bin; compgen -c >/dev/null; compgen -c")
        assert all(name in output.splitlines() for name in (b"test-command-a", b"test-command-b"))

        (home / "rc").write_text("PS1='tonari$ '\nunset HISTFILE\n")
        with terminal(tonari, home, "/usr/bin:/bin", home / "rc", prompt=PROMPT) as fd:
            def enter(keys):
                os.write(fd, keys + b"\r")
                return read_until(fd, b"\x1b[?2004h")

            enter(b"history -s v=selected")
            os.write(fd, b"v=mydraft\x12selected")
            read_until(fd, b"v=selected")
            enter(b"\x01\x1f\x05")
            enter(b"printf %s \"$v\" >undo-result")
            assert (home / "undo-result").read_bytes() == b"mydraft"
            enter("v=A👩‍💻".encode() + b"\x7f")
            enter(b"printf %s \"$v\" >unicode-result")
            assert (home / "unicode-result").read_bytes() == b"A"
            enter(b"\x1b[200~alias xx='echo accepted'\nxx >alias-result\x1b[201~")
            assert (home / "alias-result").read_text() == "accepted\n"
            enter(b"v='one two three\x17\x17\x19'")
            enter(b'printf %s "$v" >kill-result')
            assert (home / "kill-result").read_bytes() == b"one two three"

            enter(b"trap ': >trapped' USR1; echo $$ >shell-pid")
            os.write(fd, b"v='before")
            read_until(fd, b"before")
            os.kill(int((home / "shell-pid").read_text()), signal.SIGUSR1)
            read_until(fd, b"\x1b[?2004h")
            assert (home / "trapped").is_file()
            enter(b"after'")
            enter(b'printf %s "$v" >handler-result')
            assert (home / "handler-result").read_bytes() == b"beforeafter"
            os.write(fd, b"\x1b[200~v='split")
            read_until(fd, b"split")
            os.kill(int((home / "shell-pid").read_text()), signal.SIGUSR1)
            read_until(fd, b"\x1b[?2004h")
            enter(b"packet'\x1b[201~")
            enter(b'printf %s "$v" >paste-handler-result')
            assert (home / "paste-handler-result").read_bytes() == b"splitpacket"
        print("History and interactive review witnesses passed")


if __name__ == "__main__":
    main()
