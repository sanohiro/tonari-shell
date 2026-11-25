"""Report Tonari functions reached by each published workload in a coverage build."""
import argparse
import csv
from functools import lru_cache
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def shellspec(source):
    script = Path(__file__).with_name("check-shellspec.py")
    spec = importlib.util.spec_from_file_location("shellspec_witnesses", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return dict(module.samples(source / "sample"))


def fumiyas(source):
    scripts = {}
    for line in source.read_text().splitlines():
        if line.startswith("## "):
            name = line[3:]
        elif line.strip():
            if "/bin/true" in line and not Path("/bin/true").is_file():
                executable = shutil.which("true", path="/usr/bin:/bin")
                if not executable:
                    continue
                name += f" ({executable})"
                line = line.replace("/bin/true", executable)
            # Retain operand sizes for field splitting and indexed reads. Shorten
            # iteration counts. _exit in the timed subshell does not flush LLVM
            # counters, so execute its body in the instrumented shell.
            prefix, body = line.split("time (")
            if "zsh -c" not in prefix:
                prefix = re.sub(r"\.\.10{4,6}", "..24", prefix)
            body = re.sub(r"\.\.10{4,6}", "..24", body)
            scripts[name] = prefix + "{ " + body[:-1].rstrip("; ") + "; }"
    return scripts


def collect(suite, source, shell, names):
    scripts = shellspec(source) if suite == "shellspec" else {}
    if suite == "fumiyas":
        scripts = fumiyas(source)
    if suite == "unixwin":
        sys.path.insert(0, str(source))
        from benchsuite.cli import load_cases
        from benchsuite.runner import Runner
        cases = {case.id: case for case in load_cases()}
    for name in names:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            work = root
            env = dict(LC_ALL="C", HOME=directory, TMPDIR=directory, PATH=os.environ["PATH"],
                       LLVM_PROFILE_FILE=str(root / "trace-%m.profraw"))
            if suite in {"shellspec", "fumiyas"}:
                if name not in scripts:
                    if suite == "fumiyas" and name.startswith("Fork & Exec"):
                        yield name, None, []
                        continue
                    raise ValueError(f"Missing published case {name}")
                command = [shell, "-c", scripts[name]]
            elif suite == "unixwin":
                case = cases[name]
                work = root / name
                command, fixture = Runner(shell, root=directory)._prepare(case, str(work))
                env.update({k: fixture[k] for k, _ in case.env})
                if name == "coproc.tr_roundtrip" and "SHELLBENCH_COREUTILS" in os.environ:
                    env["PATH"] = os.environ["SHELLBENCH_COREUTILS"] + os.pathsep + env["PATH"]
            elif suite == "luish":
                command = [shell, str(source / "bench/scripts" / (name + ".sh")), "1"]
                env["SH"] = shell
            elif suite == "gnu-hello":
                configure = str(source / "configure")
                flags = {"configure syntax check": ["-n", configure],
                         "configure --help": [configure, "--help"],
                         "GNU Hello configure": [configure, "--disable-nls", "--cache-file=/dev/null"]}
                command = [shell, *flags[name]]
                env.update(CONFIG_SHELL=shell, SHELL=shell, CC="/usr/bin/cc")
            result = subprocess.run(command, cwd=work, env=env, capture_output=True, timeout=300)
            expected = {"null.sh: blank": 2, "output.sh: print": 127}
            if sys.platform == "darwin":
                expected["stringop2.sh: substr 2 expr"] = 2
            if result.returncode != expected.get(name, 0):
                raise ValueError(f"{name} exited {result.returncode}: {result.stderr[-500:]!r}")
            raw = list(root.glob("*.profraw"))
            if not raw:
                raise ValueError(f"No LLVM profile for {name}")
            yield name, result.returncode, symbols(raw, root, shell)


@lru_cache(maxsize=None)
def source_lines(path):
    return path.read_bytes().splitlines()


def symbols(raw, root, shell):
    profile = root / "merged.profdata"
    profdata = os.environ.get("LLVM_PROFDATA", "llvm-profdata")
    cov = os.environ.get("LLVM_COV", "llvm-cov")
    subprocess.run([profdata, "merge", "-sparse", *raw, "-o", profile], check=True)
    export = json.loads(subprocess.check_output([
        cov, "export", shell, "--instr-profile=" + str(profile), "-skip-branches", "-skip-expansions",
    ]))
    repo = Path(__file__).resolve().parents[2]
    paths = set()
    for function in export["data"][0]["functions"]:
        if not function["count"]:
            continue
        # LLVM source positions use UTF-8 byte columns. Match the start of the
        # definition, excluding closures whose first region is inside its body.
        line, col, _, _, _, file_id, *_ = function["regions"][0]
        path = Path(function["filenames"][file_id])
        if not path.is_relative_to(repo / "src"):
            continue
        head = source_lines(path)[line - 1][col - 1:].lstrip()
        name = re.match(rb'(?:pub(?:\([^)]*\))?\s+)?(?:(?:const|unsafe|extern "C")\s+)*fn\s+(\w+)', head)
        if name:
            paths.add(f"{path.relative_to(repo)}::{name[1].decode()}")
    if not paths:
        raise ValueError("No executed Tonari source definitions in the LLVM profile")
    return sorted(paths)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=["shellspec", "fumiyas", "unixwin", "luish", "gnu-hello"])
    parser.add_argument("source", type=Path)
    parser.add_argument("shell", type=Path)
    parser.add_argument("--case", action="append")
    args = parser.parse_args()
    inventory = Path(__file__).resolve().parents[2] / "docs/external-shell-benchmarks.csv"
    with inventory.open() as f:
        names = [r["case"] for r in csv.DictReader(f) if r["suite"] == args.suite]
    if args.case:
        if set(args.case) - set(names):
            parser.error("unknown published case")
        names = [name for name in names if name in args.case]
    for name, status, functions in collect(args.suite, args.source.resolve(), str(args.shell.resolve()), names):
        print(json.dumps([args.suite, name, status, functions]), flush=True)


if __name__ == "__main__":
    main()
