"""Time installed Bash integrations through a real PTY in disposable projects."""
import argparse
from collections import defaultdict
import fcntl
import os
from pathlib import Path
import pty
import shutil
import shlex
import signal
import struct
import subprocess
import tempfile
import termios
import time

from bench_history import read_until
from bench_starship import compare

PROMPT = b"hook-ready> "
ACCEPT = b"\x1b[?2004l"
TOOLS = ("nvm", "direnv", "git", "gh", "fzf", "fzf-files", "mise", "vscode")
VSCODE_ENV = dict(VSCODE_INJECTION="1", TERM_PROGRAM="vscode", VSCODE_STABLE="1")


def sample(shell, root, rc, env, tool, versions, saved):
    env = dict(env, SHELL=str(shell))
    if tool == "fzf" and saved:
        shutil.copyfile(root / "seed", root / ".bash_history")
        shutil.copyfile(root / "journal", root / ".local/state/tonari/history")
    read, write = os.pipe()
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.close(read)
            os.chdir(root)
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 110, 0, 0))
            os.write(write, struct.pack("Q", time.monotonic_ns()))
            os.execve(shell, [str(shell), "--noprofile", "--init-file", str(rc), "-i"], env)
        finally:
            os._exit(126)
    os.close(write)
    result = defaultdict(list)

    def record(name, start):
        result[name].append((time.monotonic_ns() - start) / 1e6)

    def command(line, expected=None):
        os.write(fd, line + b"\r")
        data = read_until(fd, ACCEPT)
        data = read_until(fd, PROMPT, data.partition(ACCEPT)[2])
        if expected and expected not in data:
            raise ValueError(f"{line!r}: expected {expected!r}, got {data[-1200:]!r}")
        return data

    def timed(name, line, expected=None):
        start = time.monotonic_ns()
        data = command(line, expected)
        record(name, start)
        return data

    try:
        with os.fdopen(read, "rb") as clock:
            shell_start, = struct.unpack("Q", clock.read(8))
        first = read_until(fd, PROMPT)
        record("first prompt", shell_start)
        if tool in ("vscode", "combined"):
            if not all(k in first for k in (b"633;P;HasRichCommandDetection=True", b"633;A\x07")):
                raise ValueError(f"missing VS Code prompt markers: {first!r}")
            for _ in range(3):
                frame = timed("false to next prompt", b"false", b"633;D;1\x07")
                for field in (b"633;E;false;", b"633;C\x07", b"633;A\x07"):
                    if field not in frame:
                        raise ValueError(f"missing {field!r}: {frame!r}")
            command(b'printf "<injected-%s>\\n" "$HOOK_BENCH_VSCODE"', b"<injected-1>\r\n")
            if tool == "combined":
                for _ in range(3):
                    start = time.monotonic_ns()
                    os.write(fd, b":")
                    read_until(fd, b":")
                    record("key to echo", start)
                    command(b"")
                timed("enter with both hooks", b"cd project")
                command(b'printf "<%s|%s|%s|%s>\\n" "$CHECK_DIRENV" "$CHECK_MISE" '
                        b'"$COMBINED_LOADED" "$(node --version)"',
                        b"<ok|ok|yes|" + versions[0].encode() + b">\r\n")
                timed("leave with both hooks", b"cd ..")
                command(b'printf "<%s|%s>\\n" "$CHECK_DIRENV" "$CHECK_MISE"', b"<|>\r\n")
        elif tool in ("direnv", "mise"):
            for directory, value in ((b"project", b"active"), (b"..", b"unset")):
                timed("enter" if directory == b"project" else "leave", b"cd " + directory)
                command(b'printf "<%s>\\n" "$' b'{HOOK_BENCH_ENV-unset}"', b"<" + value + b">\r\n")
            for _ in range(3):
                timed("unchanged prompt", b":")
        elif tool in ("git", "gh"):
            prefix, ending = ((b"git switch topic-uni", b"que ") if tool == "git"
                              else (b"gh iss", b"ue "))
            for i in range(4):
                os.write(fd, prefix)
                read_until(fd, prefix)
                start = time.monotonic_ns()
                os.write(fd, b"\t")
                read_until(fd, ending)
                record("first Tab" if i == 0 else "repeat Tab", start)
                command(b"\x15:")
        elif tool == "nvm":
            command(b"node --version", versions[0].encode() + b"\r\n")
            record("shell start through first node --version and next prompt", shell_start)
            command(b'printf "<%s>\\n" "$(command -v node)"',
                    b"<" + os.fsencode(root / "nvm/versions/node" / versions[0] / "bin/node") + b">\r\n")
            for version in (versions[1], versions[0]):
                timed("switch installed Node", b"nvm use --silent " + version.encode())
                command(b'printf "<%s>\\n" "$(node --version)"', b"<" + version.encode() + b">\r\n")
        elif tool == "fzf":
            if not saved:
                timed("history import", b'history -r "$HOME/seed"')
            start = time.monotonic_ns()
            os.write(fd, b"\x12")
            read_until(fd, b"HISTORY_PICK")
            record("Ctrl-R to visible match", start)
            start = time.monotonic_ns()
            os.write(fd, b"\r")
            closed = read_until(fd, b"\x1b[?1049l")
            read_until(fd, b"printf HISTORY_PICK", closed.partition(b"\x1b[?1049l")[2])
            record("select to editable line", start)
            command(b"-ok", b"HISTORY_PICK-ok")
        elif tool == "fzf-files":
            prefix = b"printf '<%s>\\n' "
            os.write(fd, prefix)
            read_until(fd, prefix)
            start = time.monotonic_ns()
            os.write(fd, b"\x14")
            read_until(fd, b"WIDGET_FILE")
            record("Ctrl-T to visible match", start)
            start = time.monotonic_ns()
            os.write(fd, b"\r")
            closed = read_until(fd, b"\x1b[?1049l")
            read_until(fd, b"WIDGET_FILE", closed.partition(b"\x1b[?1049l")[2])
            record("select to editable argument", start)
            command(b"", b"<unique WIDGET_FILE>\r\n")
        return result
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(fd)
        os.waitpid(pid, 0)


def fixture(tool, root, args):
    project = root / "project"
    if tool == "combined":
        rc, env, versions = fixture("nvm", root, args)
        env.update(VSCODE_ENV, MISE_OFFLINE="1", MISE_TRUSTED_CONFIG_PATHS=str(project))
        (project / ".envrc").write_text("export CHECK_DIRENV=ok\n")
        (project / "mise.toml").write_text('[env]\nCHECK_MISE="ok"\n')
        subprocess.run(["direnv", "allow", str(project)], env=env, cwd=root, check=True)
        (root / "gh.bash").write_bytes(subprocess.check_output(["gh", "completion", "--shell", "bash"], env=env))
        with rc.open("a") as script:
            for path in (args.bash_completion, args.git_completion, root / "gh.bash"):
                script.write(". " + shlex.quote(str(path.resolve())) + "\n")
            script.write('eval "$(fzf --bash)"\neval "$(direnv hook bash)"\neval "$(mise activate bash)"\n'
                         'export HOOK_BENCH_VSCODE=1\n'
                         'declare -F nvm __fzf_history__ __git_complete >/dev/null && '
                         'complete -p gh >/dev/null && export COMBINED_LOADED=yes\n')
        return args.vscode_script.resolve(), env, versions
    project.mkdir()
    env = dict(PATH=os.environ["PATH"], HOME=str(root), TERM="xterm-256color", LC_ALL="C",
               PS1=PROMPT.decode(), INPUTRC="/dev/null", HISTFILE="/dev/null",
               GH_CONFIG_DIR=str(root / "gh-config"), GIT_CONFIG_NOSYSTEM="1")
    rc = root / ".bashrc"
    versions = []
    if tool in ("git", "gh"):
        script = args.git_completion if tool == "git" else root / "gh.bash"
        if tool == "gh":
            script.write_bytes(subprocess.check_output(["gh", "completion", "--shell", "bash"], env=env))
            init = ". " + shlex.quote(str(args.bash_completion.resolve())) + "\n"
        else:
            init = ""
            def git(*argv, **kw):
                subprocess.run(["git", *argv], check=True, cwd=root, env=env, **kw)
            git("init", "-q")
            git("-c", "user.name=fixture", "-c", "user.email=test@example.invalid", "commit",
                "--allow-empty", "--no-gpg-sign", "-qm", "fixture")
            oid = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, env=env, text=True).strip()
            refs = [f"create refs/heads/bench-{i:04} {oid}\n" for i in range(1000)]
            git("update-ref", "--stdin", input="".join(refs), text=True)
            git("pack-refs", "--all")
            git("branch", "topic-unique")
        rc.write_text(init + ". " + shlex.quote(str(script.resolve())) + "\n")
    elif tool == "fzf-files":
        for i in range(1024):
            (project / f"file-{i:04}").touch()
        (project / "unique WIDGET_FILE").touch()
        env.update(FZF_CTRL_T_OPTS="--no-height --query=unique")
        rc.write_text('cd project\neval "$(fzf --bash)"\n')
    elif tool == "fzf":
        (root / "seed").write_text("".join(f"echo entry-{i}\n" for i in range(args.entries))
                                   + "printf HISTORY_PICK\n")
        env.update(FZF_CTRL_R_OPTS="--no-height", HISTSIZE=str(args.entries + 100))
        if args.saved_history:
            env.pop("HISTFILE")
            (root / ".local/state/tonari").mkdir(parents=True)
            (root / "journal").write_text("".join(
                "1\t100\t0\t/tmp\t" + line for line in (root / "seed").read_text().splitlines(True)))
        rc.write_text('eval "$(fzf --bash)"\n')
    elif tool == "nvm":
        nvm = root / "nvm"
        (nvm / "alias").mkdir(parents=True)
        (nvm / "versions/node").mkdir(parents=True)
        for node in args.nodes:
            node = node.resolve()
            version = subprocess.check_output([node / "bin/node", "--version"], text=True).strip()
            (nvm / "versions/node" / version).symlink_to(node, target_is_directory=True)
            versions.append(version)
        (nvm / "alias/default").write_text(versions[0] + "\n")
        env.update(NVM_DIR=str(nvm))
        rc.write_text(". " + shlex.quote(str(args.nvm_script.resolve())) + "\n")
    elif tool == "vscode":
        rc.write_text("export HOOK_BENCH_VSCODE=1\n")
        rc = args.vscode_script.resolve()
        env.update(VSCODE_ENV)
    elif tool == "mise":
        (project / "mise.toml").write_text('[env]\nHOOK_BENCH_ENV = "active"\n')
        env.update(MISE_TRUSTED_CONFIG_PATHS=str(project), MISE_OFFLINE="1")
        rc.write_text('eval "$(mise activate bash)"\n')
    elif tool == "direnv":
        (project / ".envrc").write_text('export HOOK_BENCH_ENV=active\n')
        subprocess.run(["direnv", "allow", str(project)], env=env, cwd=root, check=True)
        rc.write_text('eval "$(direnv hook bash)"\n')
    bashrc = root / ".bashrc"
    bashrc.write_text("PS1=" + shlex.quote(PROMPT.decode()) + "\n" + bashrc.read_text())
    return rc, env, versions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", type=Path, nargs="+")
    parser.add_argument("--case", action="append", choices=(*TOOLS, "combined"))
    parser.add_argument("--git-completion", type=Path)
    parser.add_argument("--bash-completion", type=Path)
    parser.add_argument("--vscode-script", type=Path)
    parser.add_argument("--nvm-script", type=Path)
    parser.add_argument("--nodes", type=Path, nargs=2, help="two installed Node prefixes")
    parser.add_argument("--entries", type=int, default=100000)
    parser.add_argument("--saved-history", action="store_true", help="load previous-session histories for fzf")
    parser.add_argument("--runs", type=int, default=15)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as temp:
        workspace = Path(temp)
        shells = []
        for i, original in enumerate(args.shells):
            shell = workspace / f"shell-{i}" / "bash"
            shell.parent.mkdir()
            shutil.copy2(original, shell)
            shells.append(shell)
        for tool in args.case or TOOLS:
            root = workspace / tool
            root.mkdir()
            rc, env, versions = fixture(tool, root, args)
            print(tool, *(str(s) for s in args.shells), sep=" | ", flush=True)
            compare(shells, args.runs, lambda shell, _: sample(shell, root, rc, env, tool, versions, args.saved_history))


if __name__ == "__main__":
    main()
