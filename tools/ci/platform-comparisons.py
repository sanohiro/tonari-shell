"""Compare the platform-specific optimizations against the portable implementation."""
from pathlib import Path
import math
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

ROOT = Path.cwd()
TEMP = Path(os.environ["RUNNER_TEMP"])
OUT = ROOT / "results"
BIN = TEMP / "bin"
MAC = sys.platform == "darwin"
REF = Path("/opt/homebrew/bin/bash" if MAC else "/bin/bash")
if MAC and not REF.exists():
    subprocess.run(["brew", "install", "bash"], check=True)
TREE = TEMP / "platform-trial"
# The original implementations and their recorded alternatives all exist at this revision.
subprocess.run(["git", "worktree", "add", "-q", "--detach", TREE, "3999b0e"], check=True)
sys.path.insert(0, str(ROOT / "tools"))
from bench_scripts import sample


def replace(file, old, new):
    p = TREE / file
    text = p.read_text()
    if text.count(old) != 1:
        raise ValueError(f"{file}: expected one match for {old!r}")
    p.write_text(text.replace(old, new))


def build(label, change=lambda: None):
    subprocess.run(["git", "restore", "."], cwd=TREE, check=True)
    shutil.copy2(ROOT / "tools/build-release.sh", TREE / "tools/build-release.sh")
    change()
    path = subprocess.check_output(["sh", "tools/build-release.sh"], cwd=TREE, text=True,
                                   env=dict(os.environ, CARGO_TARGET_DIR=str(TEMP / "build"))).strip()
    result = BIN / label
    shutil.copy2(TREE / path, result)
    with (OUT / "variants.txt").open("a") as f:
        print(label, result.stat().st_size, subprocess.check_output(["shasum", "-a", "256", result], text=True).split()[0], file=f)
    return result


def run(label, driver, *args, affinity=False):
    cmd = [sys.executable, str(ROOT / "tools" / driver), *map(str, args)]
    if affinity:
        cmd = ["taskset", "-c", str(min(os.sched_getaffinity(0))), *cmd]
    print("::group::" + label, flush=True)
    with (OUT / (label + ".txt")).open("w") as f:
        subprocess.run(cmd, check=True, stdout=f)
    print((OUT / (label + ".txt")).read_text(), flush=True)
    print("::endgroup::", flush=True)


def large_globs(shells):
    with tempfile.TemporaryDirectory() as tmp, (OUT / "100k-globs.txt").open("w") as out:
        home = Path(tmp)
        folder = home / "large"
        folder.mkdir()
        for i in range(100000):
            (folder / f"{('a','B','雪','🌱')[i%4]}-{i:06}.txt").touch()
        env = dict(HOME=tmp, PATH="/usr/bin:/bin", LC_ALL="en_US.UTF-8" if MAC else "C.UTF-8")
        for name, source, count in [
            ("plain", "set -- large/*.txt", 100000),
            ("ignore", 'GLOBIGNORE="large/B-*.txt"; set -- large/*.txt', 75000),
            ("nocase", "shopt -s nocaseglob; set -- large/*.TXT", 100000),
        ]:
            script = home / "script"
            script.write_text(source + '; echo "$#"\n')
            times = {b: [] for b in shells}
            for i in range(23):
                offset = i % len(shells)
                for b in shells[offset:] + shells[:offset]:
                    ms = sample(b, script, f"{count}\n".encode(), env, True)
                    if i >= 2:
                        times[b].append(ms)
            for b, values in times.items():
                print(name, b.name, statistics.median(values), sorted(values)[math.ceil(.95*len(values))-1], file=out)


def directory():
    base = build("default")
    if MAC:
        def standard():
            replace("src/pattern/order.rs", "if sort_utf8(paths, _path_bytes, indices)", "if false && sort_utf8(paths, _path_bytes, indices)")
        other = build("strcoll", standard)
    else:
        def standard():
            p = "src/pattern/directory.rs"
            replace(p, '#[cfg(not(target_os = "linux"))]', "")
            replace(p, '#[cfg(target_os = "linux")]', "#[cfg(any())]")
        other = build("readdir", standard)
    large_globs([base, other, REF])
    run("globs", "bench_scripts.py", base, "--previous", other, "--bash", REF, "--runs", 31,
        "--case", "glob-sparse", "--case", "glob-all", "--case", "glob-subdirs", "--case", "glob-recursive",
        "--case", "glob-utf8-large-repeat", "--case", "glob-ignore-large", "--case", "glob-nocase-large")
    run("directory-completion", "bench_completion.py", base, other, "--files", 100000, "--runs", 31)


def pipes():
    base = build("default")
    if MAC:
        def prime():
            start = "    let (read, write) = io::pipe()?;"
            replace("src/execute.rs", start, start + """
    {
        use std::io::{Read, Write};
        let fd = write.as_raw_fd();
        let mut on = 1;
        let mut off = 0;
        if unsafe { libc::ioctl(fd, libc::FIONBIO, &mut on) } == 0 {
            let mut bytes = [0u8; 16 * 1024];
            let n = (&write).write(&bytes);
            if unsafe { libc::ioctl(fd, libc::FIONBIO, &mut off) } == -1 {
                return Err(io::Error::last_os_error());
            }
            if let Ok(n) = n { (&read).read_exact(&mut bytes[..n])?; }
        }
    }""")
        shells = [base, build("primed", prime), REF]
    else:
        def capacity(n):
            return lambda: replace("src/execute.rs", "const TARGET: libc::c_int = 128 * 1024;",
                                   f"const TARGET: libc::c_int = {n} * 1024;")
        shells = [build("kernel-default", capacity(0)), base, build("256KiB", capacity(256)), REF]
        with (OUT / "pipe-capacity.txt").open("w") as f:
            for b in shells:
                n = subprocess.check_output([b, "-c", """/bin/true | /usr/bin/python3 -c 'import fcntl; print(fcntl.fcntl(0,fcntl.F_GETPIPE_SZ))'"""], text=True)
                print(b.name, n.strip(), file=f)
    for single in ([False] if MAC else [False, True]):
        suffix = "-one" if single else ""
        run("pipe" + suffix, "bench_streams.py", *shells, "--case", "pipe", "--sizes", 64, 262144, 4194304,
            16777216, 268435456, "--runs", 21, affinity=single)
        for b in shells[1:-1]:
            run("tiny-" + b.name + suffix, "bench_scripts.py", base, "--previous", b, "--bash", REF,
                "--timing", "both", "--case", "pipeline", "--case", "merged-pipeline", "--runs", 21, affinity=single)
    run("capture-64MiB", "bench_file_capture.py", base, REF, "--case", 67108864, 1, "--runs", 9)


def heap():
    base = build("default")
    def alternate():
        if MAC:
            replace("Cargo.toml", """[target.'cfg(all(target_os = "linux", target_env = "musl"))'.dependencies]\n""", "")
            replace("src/main.rs", '#[cfg(all(target_os = "linux", target_env = "musl"))]\nmod allocator;', "mod allocator;")
        else:
            replace("src/main.rs", '#[cfg(all(target_os = "linux", target_env = "musl"))]\nmod allocator;', "")
    other = build("hybrid" if MAC else "system", alternate)
    run("history", "bench_history.py", base, other, "--entries", 1000000, "--escaped", "--runs", 21)
    run("files", "bench_completion.py", base, other, "--files", 100000, "--runs", 21)
    run("script", "bench_scripts.py", base, "--previous", other, "--bash", REF, "--runs", 15)
    if not MAC:
        run("resident", "bench_heap.py", base, other, REF, "--runs", 11)


def codegen():
    base = build("default")
    if MAC:
        other = build("default-outliner", lambda: replace("tools/build-release.sh",
                      " -C llvm-args=--enable-machine-outliner=always -C llvm-args=--outliner-benefit-threshold=384", ""))
    else:
        other = build("frame-pointers", lambda: replace("tools/build-release.sh", 'export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$tuning"',
                      'export RUSTFLAGS="-Cforce-frame-pointers=yes ${RUSTFLAGS:+$RUSTFLAGS }$tuning"'))
    run("dispatch", "bench.py", base, "--compare-tonari", "Variant", other, "--bash", REF, "--runs", 101)
    run("scripts", "bench_scripts.py", base, "--previous", other, "--bash", REF, "--runs", 21)
    run("editing", "bench_editing.py", base, other, REF, "--size", 65536, "--runs", 15)
    run("search", "bench_history.py", base, other, "--entries", 1000000, "--escaped", "--runs", 21)


{"directory": directory, "pipes": pipes, "heap": heap, "codegen": codegen}[os.environ["EXPERIMENT"]]()
