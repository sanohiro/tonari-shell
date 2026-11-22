"""Exercise fzf's installed Bash Ctrl-T widget in Tonari's Emacs and vi maps."""
import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from terminal import Terminal


def require(terminal, marker):
    output = terminal.read(marker, 8)
    if marker not in output:
        raise AssertionError(f"missing {marker!r} in {output[-500:]!r}")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", type=Path)
    parser.add_argument("--fzf", default="fzf")
    args = parser.parse_args()
    executable = shutil.which(args.fzf)
    if executable is None:
        parser.error("fzf with the --bash integration is required")
    shell = args.shell.resolve()
    script = subprocess.check_output([executable, "--bash"])
    version = subprocess.check_output([executable, "--version"], text=True).strip()
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        name = "unique-cat 雪.txt"
        (root / name).touch()
        (root / "keys.bash").write_bytes(script)
        env = dict(PATH=os.pathsep.join([str(Path(executable).parent), os.defpath]),
                   SHELL=str(shell), LC_ALL="en_US.UTF-8", FZF_CTRL_T_OPTS="--filter=unique-cat")
        terminal = Terminal(shell, root, True, env)
        try:
            require(terminal, b"READY>")
            terminal.send(b'. ./keys.bash 2>load-errors; printf "LOAD%s\\n" ED\r')
            require(terminal, b"LOADED\r\n")
            errors = (root / "load-errors").read_text()
            if errors:
                raise AssertionError(errors)
            for mode in ("emacs", "vi"):
                terminal.send(f'set -o {mode}; printf "MODE%s\\n" SET\r'.encode())
                ready = require(terminal, b"MODESET\r\n").partition(b"MODESET\r\n")[2]
                if b"\x1b[?2004h" not in ready:
                    require(terminal, b"\x1b[?2004h")
                terminal.send(b"printf '<%s>\\n' \x14")
                require(terminal, b"unique-cat")
                terminal.send(b' >picked; printf "PICK%s\\n" ED\r')
                require(terminal, b"PICKED\r\n")
                actual = (root / "picked").read_text()
                if actual != f"<{name}>\n":
                    raise AssertionError(f"{mode} passed {actual!r} to printf")
                print(f"fzf {version}, {mode}: {actual.strip()}")
        finally:
            terminal.close()


if __name__ == "__main__":
    main()
