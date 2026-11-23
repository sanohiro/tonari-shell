"""Check Git branch completion and direnv's prompt hook in a disposable interactive shell."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

from fzf_keys import require
from terminal import Terminal


def git_fixture(root, env):
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=root, env=env, text=True).strip()
    git("init", "-q")
    git("-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "--allow-empty", "--no-gpg-sign", "-qm", "fixture")
    return git


def run(shell, completion, kind):
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        env = dict(PATH=os.environ.get("PATH", os.defpath), SHELL=str(shell), LC_ALL="en_US.UTF-8")
        child = dict(env, HOME=temp, GIT_CONFIG_NOSYSTEM="1")
        if kind == "git":
            git = git_fixture(root, child)
            env["GIT_COMPLETION_FIXTURE"] = str(completion)
            setup = b'. "$GIT_COMPLETION_FIXTURE"'
        else:
            project = root / "project"
            project.mkdir()
            (project / ".envrc").write_text("export TONARI_DIRENV_SAMPLE='雪 x'\n")
            subprocess.run(["direnv", "allow", str(project)], cwd=root, env=child, check=True)
            setup = b'eval "$(direnv hook bash)"'
        terminal = Terminal(shell, root, True, env)
        try:
            require(terminal, b"READY>")
            terminal.send(setup + b' 2>load-errors; printf "SET%s\\n" UP\r')
            require(terminal, b"SETUP\r\n")
            errors = (root / "load-errors").read_text()
            if errors:
                raise AssertionError(errors)
            if kind == "git":
                git("branch", "tonari-topic-new")
                terminal.send(b"git switch tonari-topic-n\t")
                require(terminal, b"git switch tonari-topic-new ")
                terminal.send(b'\rprintf "DO%s\\n" NE\r')
                require(terminal, b"DONE\r\n")
                if git("branch", "--show-current") != "tonari-topic-new":
                    raise AssertionError("the completed branch was not checked out")
                print("Git completed and checked out a branch created after startup")
            else:
                terminal.send(b"cd project\r")
                require(terminal, b"READY>")
                terminal.send(b'printf "ACTIVE=<%s>\\n" "$TONARI_DIRENV_SAMPLE"\r')
                require(terminal, "ACTIVE=<雪 x>\r\n".encode())
                terminal.send(b"cd ..\r")
                require(terminal, b"READY>")
                terminal.send(b'printf "LEFT=<%s>\\n" "${TONARI_DIRENV_SAMPLE-unset}"\r')
                require(terminal, b"LEFT=<unset>\r\n")
                print("direnv applied and removed the temporary project environment")
        finally:
            terminal.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shell", type=Path)
    parser.add_argument("--git-completion", type=Path, help="installed Git Bash completion script")
    parser.add_argument("--direnv", action="store_true", help="use direnv from PATH")
    args = parser.parse_args()
    if not args.git_completion and not args.direnv:
        parser.error("select --git-completion or --direnv")
    if args.git_completion:
        run(args.shell.resolve(), args.git_completion.resolve(), "git")
    if args.direnv:
        run(args.shell.resolve(), None, "direnv")


if __name__ == "__main__":
    main()
