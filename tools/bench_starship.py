"""Measure the upstream Starship Bash integration in interactive pseudo-terminals."""
import argparse
from collections import defaultdict
import fcntl
import math
import os
from pathlib import Path
import pty
import shlex
import signal
import statistics
import struct
import subprocess
import tempfile
import termios
import time

from bench_history import read_until


PROMPT = "❯".encode() + b"\x1b[0m "
ACCEPTED = b"\x1b[?2004l"


def next_prompt(fd):
    accepted = read_until(fd, ACCEPTED)
    return read_until(fd, PROMPT, accepted.partition(ACCEPTED)[2])


def trial(shell, cwd, rc, env, verify, first_only=False):
    clock_in, clock_out = os.pipe()
    argv = [str(shell), "--noprofile", "--rcfile", str(rc), "-i"]
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.close(clock_in)
            os.chdir(cwd)
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
            os.write(clock_out, struct.pack("Q", time.monotonic_ns()))
            os.execve(shell, argv, env)
        finally:
            os._exit(127)
    os.close(clock_out)
    result = defaultdict(list)
    try:
        with os.fdopen(clock_in, "rb") as stamp:
            start, = struct.unpack("Q", stamp.read(8))
        first = read_until(fd, PROMPT)
        result["first prompt (ms)"].append((time.monotonic_ns() - start) / 1e6)
        if cwd.name == "project" and b"prompt-bench" not in first:
            raise ValueError(f"missing Git branch in prompt: {first!r}")
        if first_only:
            return result
        for _ in range(3):
            start = time.monotonic_ns()
            os.write(fd, b":")
            echo = read_until(fd, b":")
            result["key echo (ms)"].append((time.monotonic_ns() - start) / 1e6)
            result["key echo (bytes)"].append(len(echo))
            start = time.monotonic_ns()
            os.write(fd, b"\r")
            next_prompt(fd)
            result["next prompt (ms)"].append((time.monotonic_ns() - start) / 1e6)
        if verify:
            os.write(fd, b"false\r")
            if b"\x1b[1;31m" + PROMPT not in next_prompt(fd):
                raise ValueError("missing failure prompt")
            os.write(fd, b"false | true\r")
            if b"\x1b[1;32m" + PROMPT not in next_prompt(fd):
                raise ValueError("missing success prompt")
            os.write(fd, b"""printf '<status=%s pipe=%s>\\n' "$STARSHIP_CMD_STATUS" "${STARSHIP_PIPE_STATUS[*]}"\r""")
            if b"<status=0 pipe=1 0>\r\n" not in next_prompt(fd):
                raise ValueError("Starship did not receive pipeline status")
            os.write(fd, b"/bin/sleep 0.1\r")
            next_prompt(fd)
            os.write(fd, b"((STARSHIP_DURATION >= 80)) && printf '<duration-%s>\\n' ok\r")
            if b"<duration-ok>\r\n" not in next_prompt(fd):
                raise ValueError("missing elapsed command time")
            os.write(fd, b"sample_jobs() { seen_jobs=$NUM_JOBS; }; starship_precmd_user_func=sample_jobs; /bin/sleep 3 & p=$!\r")
            next_prompt(fd)
            os.write(fd, b"""printf '<jobs=%s>\\n' "$seen_jobs"; kill "$p"; wait "$p"; :\r""")
            if b"<jobs=1>\r\n" not in next_prompt(fd):
                raise ValueError("live background job missing from prompt hook")
        return result
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.close(fd)
        os.waitpid(pid, 0)


def compare(shells, runs, sample):
    values = {sh: defaultdict(list) for sh in shells}
    for i in range(runs + 2):
        start = i % len(shells)
        for sh in shells[start:] + shells[:start]:
            for key, samples in sample(sh, i == 0).items():
                if i >= 2:
                    values[sh][key].extend(samples)
    for key in values[shells[0]]:
        row = []
        for sh in shells:
            v = sorted(values[sh][key])
            row.append(f"{statistics.median(v):.3f}/{v[math.ceil(.95 * len(v)) - 1]:.3f}")
        print(key, *row, sep=" | ", flush=True)


def replay_starship(root, starship, shell, cwd, rc, env):
    """Keep the normal hook and process boundaries, replaying a successful prompt."""
    directory = root / "replay"
    directory.mkdir()
    exe = directory / "starship"
    env_replay = {**env, "PATH": f"{directory}:/usr/bin:/bin"}
    # Starship resolves its embedded executable via PATH. Generate the hook for
    # the final replay path with the genuine binary before installing the stub.
    exe.symlink_to(starship)
    fixture = {}
    for name, args in (("stub", []), ("hook", ["--print-full-init"])):
        fixture[name] = subprocess.check_output([exe, "init", "bash", *args], env=env_replay)
        if os.fsencode(exe) not in fixture[name]:
            raise ValueError("replay hook did not use the test executable")
    exe.unlink()

    # Capture the literal PS1/PS2 after the real Bash hook has run in a PTY.
    capture_rc = root / "capture-rc"
    capture_rc.write_text(rc.read_text() + """PROMPT_COMMAND='starship_precmd
printf %s "$PS1" > "$HOME/ps1"
printf %s "$PS2" > "$HOME/ps2"'\n""")
    trial(shell, cwd, capture_rc, env, False)
    for name in ("ps1", "ps2"):
        fixture[name] = (Path(env["HOME"]) / name).read_bytes()
    data = "".join(f"static const char {k}[] = {{{','.join(map(str, v))},0}};\n"
                   for k, v in fixture.items())
    src = directory / "replay.c"
    src.write_text("#include <stdio.h>\n#include <string.h>\n#include <time.h>\n" + data + """
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    if (strcmp(argv[1], "init") == 0) return fputs(argc > 3 ? hook : stub, stdout) < 0;
    if (strcmp(argv[1], "time") == 0) {
        struct timespec ts;
        if (clock_gettime(CLOCK_REALTIME, &ts) < 0) return 1;
        printf("%lld\\n", (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
        return 0;
    }
    if (strcmp(argv[1], "prompt") != 0) return 2;
    return fputs(argc > 2 && strcmp(argv[2], "--continuation") == 0 ? ps2 : ps1, stdout) < 0;
}
""")
    subprocess.run(["cc", "-O2", src, "-o", exe], check=True)
    trial(shell, cwd, capture_rc, env_replay, False)
    for name in ("ps1", "ps2"):
        if (Path(env["HOME"]) / name).read_bytes() != fixture[name]:
            raise ValueError(f"replayed {name} differs from Starship")
    return env_replay


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("shells", type=Path, nargs="+")
    ap.add_argument("--starship", type=Path, required=True)
    ap.add_argument("--runs", type=int, default=21)
    ap.add_argument("--long-prompts-only", action="store_true", help="measure Git prompts with 256 or 1024 additional visible ASCII characters")
    args = ap.parse_args()
    shells = [sh.resolve() for sh in args.shells]
    starship = args.starship.resolve()
    print(subprocess.check_output([starship, "--version"], text=True).splitlines()[0])
    print("p50/p95", *shells, sep=" | ")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        home, plain, project = (root / name for name in ("home", "plain", "project"))
        for d in (home, plain, project):
            d.mkdir()
        # Keep other Starship installations and user shell, Git and prompt settings out.
        (root / "starship").symlink_to(starship)
        env = dict(PATH=f"{root}:/usr/bin:/bin", HOME=str(home), INPUTRC="/dev/null",
                   TERM="xterm-256color", LANG="en_US.UTF-8", HISTFILE="/dev/null",
                   GIT_CONFIG_NOSYSTEM="1", GIT_CEILING_DIRECTORIES=str(root))
        subprocess.run(["git", "init", "-qb", "prompt-bench"], cwd=project, env=env, check=True)
        for i in range(200):
            (project / f"file-{i}").touch()
        subprocess.run(["git", "add", "."], cwd=project, env=env, check=True)
        subprocess.run(["git", "-c", "user.name=bench", "-c", "user.email=bench@localhost",
                        "commit", "-qm", "fixture"], cwd=project, env=env, check=True)
        rc = home / "rc"
        rc.write_text('eval "$(starship init bash)"\n')
        if args.long_prompts_only:
            config = home / "prompt.toml"
            env["STARSHIP_CONFIG"] = str(config)
            for size in (256, 1024):
                fmt = "${directory}${git_branch}" + "x" * size + "$character"
                config.write_text(f"format = '{fmt}'\n")
                print(f"project, {size} additional characters", flush=True)
                compare(shells, args.runs, lambda sh, verify: trial(sh, project, rc, env, verify))
            return
        for cwd in (plain, project):
            print(cwd.name, flush=True)
            compare(shells, args.runs, lambda sh, verify: trial(sh, cwd, rc, env, verify))

        replay_env = replay_starship(root, starship, shells[-1], project, rc, env)
        print("project, external output replay (successful colon commands)", flush=True)
        compare(shells, args.runs, lambda sh, _: trial(sh, project, rc, replay_env, False))

        # Compare startup using the same upstream code generated ahead of time.
        init = home / "starship-init.bash"
        init.write_bytes(subprocess.check_output([starship, "init", "bash", "--print-full-init"], env=env))
        rc.write_text('. "$HOME/starship-init.bash"\n')
        print("project, saved init", flush=True)
        compare(shells, args.runs, lambda sh, verify: trial(sh, project, rc, env, verify, first_only=not verify))

        # Isolate shell work which is otherwise hidden by the renderer and Git.
        marker_prompt = (r"\[" + "\x1b[1;36m" + r"\]" + "開発 🌸 " + r"\[" + "\x1b[0m" + r"\]") * 4 + "\n❯ "
        # Take the full multiline precmd definition verbatim from the installed hook.
        # The final top-level brace is unindented in the pinned upstream script.
        hook = init.read_text()
        start = hook.index("starship_precmd() {\n")
        end = hook.index("\n}\n", start) + len("\n}")
        setup = 'hook=' + shlex.quote(hook[start:end])
        cases = [
            ("100 evals of the upstream precmd definition", setup,
             'eval "$hook" || exit; declare -F starship_precmd >/dev/null || exit 1', 100, ""),
            ("200 empty jobs captures", "", "s=$(jobs -p)", 200, ""),
            ("200 external array captures", 'args=("a b" "" "*")', 's=$(/usr/bin/printf %s "${args[@]}")', 200, "a b*"),
            ("100 captures of a wrapper function", "f() { /usr/bin/printf ok; }", "s=$(f)", 100, "ok"),
            ("24 newlines-only expansions, 256 bytes", "p=$'" + "abc🌸x" * 32 + "\\n'", r"s=${p//[!$'\n']}", 24, "\n"),
            ("2000 styled prompt expansions", f"p='{marker_prompt}'", 's=${p@P}', 2000,
             marker_prompt.replace(r"\[", "").replace(r"\]", "")),
        ]
        for label, setup, body, count, expected in cases:
            script = f'{setup}\nfor ((i=0; i<{count}; i++)); do {body}; done\nprintf %s "$s"'

            def batch(sh, verify):
                start = time.monotonic_ns()
                result = subprocess.check_output([sh, "--noprofile", "--norc", "-c", script], env=env, cwd=plain)
                elapsed = (time.monotonic_ns() - start) / 1e6
                if result != expected.encode():
                    raise ValueError(f"{label}: unexpected result {result!r}")
                return {label + " (ms)": [elapsed]}

            compare(shells, args.runs, batch)


if __name__ == "__main__":
    main()
