"""Compare complete script launches and sourcing the same files after shell startup."""

import argparse
from pathlib import Path
import select
import subprocess
import tempfile
import time

from bench import report


# The last command prints a result that the driver checks on every run.
CASES = {
    "arithmetic": ("""\
sum=0
for ((i=1; i<=10000; i++)); do ((sum += i)); done
echo "$sum"
""", "50005000"),
    "functions": ("""\
add() { local n=$1; ((sum += n)); }
sum=0
for ((i=1; i<=4000; i++)); do add "$i"; done
echo "$sum"
""", "8002000"),
    "strings": ("""\
for ((i=0; i<2000; i++)); do
    path="/var/log/worker-${i}.log"
    base=${path##*/}
    stem=${base%.log}
done
echo "$stem"
""", "worker-1999"),
    "read-lines": ("""\
count=0
while IFS= read -r row; do
    ((count++))
    last=$row
done < rows
echo "$count $last"
""", "2000 record 1999"),
    "substitutions": ("""\
for ((i=0; i<64; i++)); do value=$(printf 'v%s' "$i"); done
echo "$value"
""", "v63"),
    "external": ("""\
for ((i=0; i<16; i++)); do /usr/bin/true || exit 1; done
echo "$i"
""", "16"),
}


def sample(binary, script, expected, env, started):
    argv = [binary, "--noprofile", "--norc"]
    if started:
        argv.extend(["-c", 'printf "ready\\n"; IFS= read -r gate && . "$1"', "bench", script])
    else:
        argv.append(script)
    start = time.perf_counter_ns()
    with subprocess.Popen(argv, env=env, cwd=script.parent, stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
        try:
            if started:
                if not select.select([child.stdout], [], [], 10)[0] or child.stdout.readline() != b"ready\n":
                    raise RuntimeError(f"{binary} did not reach the startup marker")
                start = time.perf_counter_ns()
            out, err = child.communicate(b"\n" if started else None)
            elapsed = (time.perf_counter_ns() - start) / 1e6
        finally:
            if child.poll() is None:
                child.kill()
    if (child.returncode, out, err) != (0, expected, b""):
        raise RuntimeError(f"{binary} {script.name}: status={child.returncode}, stdout={out!r}, stderr={err!r}")
    return elapsed


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("tonari", type=Path)
    cli.add_argument("--bash", type=Path, default=Path("/bin/bash"))
    cli.add_argument("--previous", type=Path, help="include a previous Tonari build")
    cli.add_argument("--runs", type=int, default=30)
    cli.add_argument("--case", choices=CASES, action="append")
    args = cli.parse_args()
    if args.runs < 1:
        cli.error("runs must be positive")
    shells = [("Tonari", args.tonari.resolve()), ("Bash", args.bash.resolve())]
    if args.previous:
        shells.append(("Previous", args.previous.resolve()))
    with tempfile.TemporaryDirectory() as temp:
        home = Path(temp)
        env = {"HOME": temp, "PATH": "/usr/bin:/bin", "LC_ALL": "C"}
        (home / "rows").write_text("".join(f"record {i}\n" for i in range(2000)))
        for case in args.case or CASES:
            source, expected = CASES[case]
            script = home / f"{case}.sh"
            script.write_text(source)
            for started in (False, True):
                samples = {name: [] for name, _ in shells}
                for i in range(5 + args.runs):
                    offset = i % len(shells)
                    for name, binary in shells[offset:] + shells[:offset]:
                        elapsed = sample(binary, script, (expected + "\n").encode(), env, started)
                        if i >= 5:
                            samples[name].append(elapsed)
                print(f"{case} ({'already started' if started else 'whole script'})", flush=True)
                report(samples)


if __name__ == "__main__":
    main()
