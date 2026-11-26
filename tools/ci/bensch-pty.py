"""Run pinned bensch YAML tests, observing PTY writes without counting its sleep timers."""
import argparse
import contextlib
import csv
import gzip
import errno
import json
import os
from pathlib import Path
import tempfile
import threading
import time

import pexpect
from pexpect.fdpexpect import fdspawn
import sys
import yaml


class ObservedSpawn(pexpect.spawn):
    """Relay the unchanged output to pexpect; timestamp its arrival at the master."""
    events = None

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        read, write = os.pipe()
        self.reader = fdspawn(read, encoding=self.encoding, codec_errors="replace",
                              timeout=self.timeout)
        self.pending = None
        self.pump = threading.Thread(target=self.relay, args=(write,), daemon=True)
        self.pump.start()

    def relay(self, write):
        try:
            while True:
                data = os.read(self.child_fd, 65536)
                if not data:
                    break
                now = time.perf_counter_ns()
                event = self.pending
                if event is not None and now >= event[0] and event[1] is None:
                    event[1] = now - event[0]
                while data:
                    data = data[os.write(write, data):]
        except OSError as exc:
            if exc.errno not in (errno.EIO, errno.EBADF, errno.EPIPE):
                raise
        finally:
            os.close(write)

    def _log(self, data, direction):
        super()._log(data, direction)
        if direction == "send":
            self.pending = None
            if self.events is not None and "__STEP_SYNC_" not in data:
                event = [time.perf_counter_ns(), None]
                self.events.append(event)
                self.pending = event

    def read_nonblocking(self, size=1, timeout=-1):
        return self.reader.read_nonblocking(size, timeout)

    def close(self, *args, **kwargs):
        try:
            super().close(*args, **kwargs)
        finally:
            self.reader.close()


def run(args):
    sys.path.insert(0, str(args.source / "framework"))
    from profile import load_profile
    from runner import YAMLTestRunner

    class Runner(YAMLTestRunner):
        def _execute_step(self, *a, **kw):
            ObservedSpawn.events = self.events
            try:
                return super()._execute_step(*a, **kw)
            finally:
                ObservedSpawn.events = None

    pexpect.spawn = ObservedSpawn
    profile = load_profile("bash")
    selected_keys = None
    if args.review:
        selected_keys = set()
        for archive in args.review.glob("*.tsv.gz"):
            if archive.name not in ("macos-15.tsv.gz", "ubuntu-24.04.tsv.gz"):
                continue
            with gzip.open(archive, "rt") as source:
                selected_keys.update(row["case"] for row in csv.DictReader(source, delimiter="\t")
                                     if row["suite"] == "bensch-pty" and row["pair"] in ("neither", "bash-only"))
        if not selected_keys:
            raise ValueError("no previous interactive failures found")
    if args.control:
        profile["history_disable"]["env"]["HISTSIZE"] = "500"
    files = sorted((args.source / "suites/interactive").rglob("*.yaml"))
    with args.output.open("w", buffering=1) as output:
        batch = 0
        for path in files:
            tests = yaml.safe_load(path.read_text())["tests"]
            for start in range(0, len(tests), 10):
                selected = batch % args.shards == args.shard
                batch += 1
                if not selected:
                    continue
                shells = [("bash", args.bash), ("tonari", args.tonari)]
                if batch % 2:
                    shells.reverse()
                if args.observer:
                    shells.append(("observer", args.observer))
                for label, binary in shells:
                    with tempfile.TemporaryDirectory() as home, contextlib.chdir(home):
                        os.environ["HOME"] = home
                        runner = Runner(str(binary), profile=profile)
                        if args.control:
                            runner._use_marker_sync = False
                            runner.tests_per_session = 1
                        runner._test_count = start
                        try:
                            for i, case in enumerate(tests[start:start + 10], start):
                                if selected_keys is not None and f"{path.stem}/{i + 1}" not in selected_keys:
                                    continue
                                trace = Path(home) / "trace"
                                if label == "observer":
                                    trace.write_bytes(b"")
                                    case = dict(case, fresh_session=True,
                                                env={**case.get("env", {}), "TONARI_CASE_TRACE": str(trace)})
                                runner.events = []
                                result = runner.run_test(case)
                                runner._test_count += 1
                                row = dict(suite=path.stem, index=i+1, name=result.name,
                                           shell=label, passed=result.passed, error=result.error,
                                           seconds=result.duration,
                                           first_byte_us=[None if delay is None else delay / 1000
                                                          for _, delay in runner.events])
                                if label == "observer":
                                    row["sites"] = sorted(set(trace.read_bytes()))
                                print(json.dumps(row), file=output)
                                time.sleep(0.3 * runner.delay_scale)
                        finally:
                            runner._cleanup_session()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "output", "bash", "tonari"):
        parser.add_argument(name, type=lambda s: Path(s).absolute())
    parser.add_argument("--observer", type=lambda s: Path(s).absolute())
    parser.add_argument("--shard", type=int, required=True)
    parser.add_argument("--shards", type=int, required=True)
    parser.add_argument("--review", type=Path, help="directory holding the two previous native inventories")
    parser.add_argument("--control", action="store_true", help="enable history and omit runner-injected commands in fresh sessions")
    run(parser.parse_args())
