"""Replay each historical source revision with its own benchmark drivers."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path.cwd()
OUT = ROOT / "results"
TEMP = Path(os.environ["RUNNER_TEMP"])
# End at the last revision that made a runtime change before this review.
tip = "3999b0e"
paths = ["src", "Cargo.toml", "Cargo.lock", "tools/build-release.sh", ":(glob)tools/bench*"]
revisions = subprocess.check_output(
    ["git", "rev-list", "--reverse", tip, "--", *paths], text=True
).splitlines()
shard = int(os.environ["SHARD"])
bash = Path("/bin/bash")
if sys.platform == "darwin":
    bash = Path("/opt/homebrew/bin/bash")
    if not bash.exists():
        subprocess.run(["brew", "install", "bash"], check=True)
env = dict(os.environ, CARGO_TARGET_DIR=str(TEMP / "build"), CARGO_PROFILE_RELEASE_PANIC="abort")
records = []
failures = []
outcome = OUT / "revisions.json"
selected = os.environ.get("REVISIONS", "").split()
for commit in revisions[shard::6]:
    if selected and not any(commit.startswith(prefix) for prefix in selected):
        continue
    tree = TEMP / commit[:10]
    subprocess.run(["git", "worktree", "add", "-q", "--detach", tree, commit], check=True)
    folder = OUT / commit[:10]
    folder.mkdir()
    # Hold compiler, ISA and optimization flags constant across source revisions.
    shutil.copy2(ROOT / "tools/build-release.sh", tree / "tools/build-release.sh")
    print("::group::" + commit, flush=True)
    with (folder / "build.log").open("w") as log:
        build = subprocess.run(["sh", "tools/build-release.sh"], cwd=tree, env=env,
                               text=True, stdout=subprocess.PIPE, stderr=log)
    record = {"commit": commit, "build": build.returncode, "workloads": {}}
    records.append(record)
    if build.returncode:
        failures.append(commit + "/build")
    else:
        binary = Path(build.stdout.strip().splitlines()[-1])
        if not binary.is_absolute():
            binary = tree / binary
        record["bytes"] = binary.stat().st_size
        for driver in sorted((tree / "tools").glob("bench*.py")):
            name = driver.stem
            if name == "bench_heap" and sys.platform != "linux":
                continue
            source = driver.read_text()
            args = [sys.executable, str(driver), str(binary)]
            if '"--bash"' in source:
                args.extend(["--bash", str(bash)])
            elif name in ("bench_heap", "bench_streams", "bench_editor_input", "bench_file_capture", "bench_editing"):
                args.append(str(bash))
            args.extend(["--runs", "11" if name == "bench" else "7"])
            with (folder / (name + ".txt")).open("w") as output:
                try:
                    code = subprocess.run(args, cwd=tree, stdout=output, stderr=subprocess.STDOUT, timeout=1200).returncode
                except subprocess.TimeoutExpired:
                    code = 124
            record["workloads"][name] = code
            if code:
                failures.append(commit + "/" + name)
    outcome.write_text(json.dumps(records, indent=2) + "\n")
    print(json.dumps(record), flush=True)
    print("::endgroup::", flush=True)
    subprocess.run(["git", "worktree", "remove", "--force", tree], check=True)
if failures:
    sys.exit("Failed historical runs: " + ", ".join(failures))
