"""Run original ShellSpec sample bodies with a bounded controller lifetime."""
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile


def cases(source, directory):
    for sample in sorted((source / "sample").glob("*.sh")):
        text = sample.read_bytes()
        markers = list(re.finditer(rb'^#bench "([^"\n]+)"[^\n]*\n', text, re.M))
        preamble = text[:markers[0].start()]
        for index, marker in enumerate(markers):
            end = markers[index + 1].start() if index + 1 < len(markers) else len(text)
            path = directory / sample.name
            path.write_bytes(preamble + text[marker.start():end])
            yield path, sample.name + ": " + marker[1].decode()


def measure(source, case, name, order, env):
    # Both workers signal readiness with SIGHUP. Ordinary POSIX signals may
    # coalesce; the upstream controller can then wait forever before timing.
    # Restart only the blocked sample in a new process group.
    for attempt in range(3):
        with subprocess.Popen(
            ["./shellbench", "-s", order, "-w", "1", "-t", "1", str(case)],
            cwd=source, env=env, stdout=subprocess.PIPE, start_new_session=True,
        ) as process:
            try:
                output, _ = process.communicate(timeout=45)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
                print(f"Controller timeout for {name}, order {order}, attempt {attempt + 1}",
                      file=sys.stderr, flush=True)
                continue
            if process.returncode:
                raise RuntimeError(f"{name}: upstream controller exited {process.returncode}")
        lines = [line for line in output.decode().splitlines() if line.startswith(name + " ")]
        if len(lines) != 1:
            raise ValueError(f"Missing or ambiguous ShellSpec result for {name}: {output!r}")
        return lines[0]
    raise TimeoutError(f"ShellSpec controller did not complete {name} after three attempts")


def main():
    source = Path(sys.argv[1]).resolve()
    names = ["tonari", "bash-ref"]
    names += [name for name in ("previous", "opt3") if (source / "bin" / name).exists()]
    orders = [names, names[::-1]] if len(names) > 2 else [names]
    env = dict(HOME=str(source / "home"), LC_ALL="C", PATH=str(source / "bin") + os.pathsep + os.environ["PATH"])
    with tempfile.TemporaryDirectory() as directory:
        for order in orders:
            order = ",".join(order)
            print("Shell order " + order, flush=True)
            for case, name in cases(source, Path(directory)):
                print(measure(source, case, name, order, env), flush=True)


if __name__ == "__main__":
    main()
