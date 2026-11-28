"""Compare cold and repeated Tab through a symlinked PATH and distinct-directory controls."""
import argparse
import math
from pathlib import Path
import runpy
import tempfile


editor = runpy.run_path(str(Path(__file__).with_name("whole-editor.py")))


def sample(shell, home):
    prompt = editor["PROMPT"]
    with editor["terminal"](shell, home) as display:
        display.observe(b"", lambda: display.adjacent(prompt))
        timings = []
        for _ in range(2):
            display.observe(b"zzuni", lambda: display.adjacent(prompt + "zzuni"))
            timings.append(display.observe(b"\t", lambda: display.adjacent(prompt + "zzuniquecmd "))[1])
            display.observe(b"\x15", lambda: display.adjacent(prompt))
        return timings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shells", nargs="+", metavar="LABEL=PATH")
    parser.add_argument("--runs", type=int, default=50)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    shells = [(name, str(Path(path).resolve())) for name, path in
              (s.split("=", 1) for s in args.shells)]
    with tempfile.TemporaryDirectory() as root:
        home = Path(root)
        for dirname in ("first", "other"):
            path = home / dirname
            path.mkdir()
            for i in range(1024):
                (path / f"entry-{i:04}").touch()
        (home / "alias").symlink_to("first")
        for case, search in [("system", "/usr/bin:/bin"), ("alias", "first:alias"),
                             ("distinct", "first:other"), ("single", "first")]:
            editor["rc_file"](home, f"PATH={search}\nzzuniquecmd() {{ :; }}\n")
            samples = {name: [] for name, _ in shells}
            for i in range(args.runs + 1):
                shift = i % len(shells)
                for name, path in shells[shift:] + shells[:shift]:
                    pair = sample(path, home)
                    if i:
                        samples[name].append(pair)
            for name, pairs in samples.items():
                for phase, values in zip(("cold", "warm"), zip(*pairs)):
                    values = sorted(values)
                    print(case, name, phase, "frame p50/p95/p99/max ms",
                          *(f"{values[math.ceil(q * len(values)) - 1]:.3f}"
                            for q in (.5, .95, .99, 1)), flush=True)


if __name__ == "__main__":
    main()
