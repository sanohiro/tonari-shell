"""Inventory and differentially probe an installed GNU Bash and Tonari. See docs/compatibility.md."""
import argparse
from dataclasses import dataclass
import json
import time
import hashlib
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import signal
import socket
from surface import extra
import subprocess
import sys
import tempfile

from cases import (PRINTF, ARITHMETIC, BUILTINS, CONDITIONS, INVOCATION, KEYWORDS, LANGUAGE,
                   PARAMETERS, REDIRS, SETOPT, SHOPT, VARIABLES)


@dataclass
class Outcome:
    code: int
    out: bytes
    err: bytes
    timeout: bool = False
    ns: int = 0

    def digest(self):
        value = [self.code, self.out.hex(), self.err.hex(), self.timeout]
        return hashlib.sha256(json.dumps(value).encode()).hexdigest()

    def brief(self):
        return dict(code=self.code, out=self.out[:140].decode("utf-8", "backslashreplace"),
                    err=self.err[:140].decode("utf-8", "backslashreplace"), timeout=self.timeout)


class Audit:
    def __init__(self, bash, tonari, root):
        self.shells = (bash, tonari)
        self.root = root
        self.rows = {}

    def call(self, index, args, stdin="", fixture=False, timing=None):
        cwd = self.root / "work"
        if cwd.exists():
            shutil.rmtree(cwd)
        cwd.mkdir()
        path = self.shells[index]
        env = dict(HOME=str(cwd), PATH="/usr/bin:/bin", LC_ALL="C", TERM="dumb",
                   SHELL=str(path), INPUTRC="/dev/null", TMPDIR=str(cwd))
        if fixture == "socket":
            with socket.socket(socket.AF_UNIX) as sock:
                sock.bind(str(cwd / "socket"))
        elif fixture:
            (cwd / "script").write_text('printf "%s/%s" "$#" "$1"')
            (cwd / "rc").write_text("rc_loaded=yes\n")
            (cwd / ".bash_profile").write_text("profile=read\n")
        # Use a fresh process group. A timed-out pipeline must not keep the
        # harness's output pipes open via a descendant.
        if timing == "started":
            args = [*args[:-1], "printf '__READY__\\n'; IFS= read -r _tonari_gate\n" + args[-1]]
            stdin = "\n" + stdin
        started = time.perf_counter_ns()
        proc = subprocess.Popen(["bash", *args], executable=path, cwd=cwd, env=env,
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                start_new_session=True)
        timeout = False
        try:
            if timing == "started":
                if not select.select([proc.stdout], [], [], 2)[0] or proc.stdout.readline() != b"__READY__\n":
                    raise subprocess.TimeoutExpired(args, 2)
                started = time.perf_counter_ns()
            if timing:
                # communicate(timeout=...) can poll child exit in millisecond increments.
                def expired(signum, frame):
                    raise subprocess.TimeoutExpired(args, 2)
                previous = signal.signal(signal.SIGALRM, expired)
                try:
                    signal.setitimer(signal.ITIMER_REAL, 2)
                    out, err = proc.communicate(stdin.encode())
                    elapsed = time.perf_counter_ns() - started
                finally:
                    signal.setitimer(signal.ITIMER_REAL, 0)
                    signal.signal(signal.SIGALRM, previous)
            else:
                out, err = proc.communicate(stdin.encode(), timeout=2)
                elapsed = time.perf_counter_ns() - started
        except subprocess.TimeoutExpired:
            timeout = True
            os.killpg(proc.pid, signal.SIGKILL)
            out, err = proc.communicate(timeout=2)
            elapsed = time.perf_counter_ns() - started
        # A completed test can have short-lived children in its session.
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
        def normalize(data):
            return data.replace(os.fsencode(cwd), b"$WORK").replace(os.fsencode(path), b"$SHELL")
        return Outcome(proc.returncode, normalize(out), normalize(err), timeout, elapsed)

    def script(self, index, source, fixture=False):
        return self.call(index, ["--noprofile", "--norc", "-c", source], fixture=fixture)

    def add(self, group, name, status, witness="", outcomes=None):
        row = dict(status=status, witness=witness)
        if outcomes is not None and status not in ("match", "interface only"):
            row["observed"] = [o.brief() for o in outcomes]
        self.rows.setdefault(group, {})[name] = row

    def compare(self, group, name, source=None, args=None, stdin="", interface=False, fixture=False, text_contract=None):
        pair = [self.script(i, source, fixture) if args is None else self.call(i, args, stdin, True) for i in (0, 1)]
        a, b = pair
        if a.timeout or (group == "Invocation" and name == "--debugger" and b"bashdb-main.inc" in a.err):
            status = "control inconclusive"
        elif b.timeout:
            status = "timeout"
        elif text_contract is not None and (b.code, b.out, b.err) != (0, text_contract[0], b""):
            status = "contract violation"
        elif text_contract is not None and (a.code or a.err or a.out not in text_contract):
            status = "control inconclusive"
        elif text_contract is not None and a.out != b.out:
            status = "intentional difference"
        elif (a.code, a.out) != (b.code, b.out):
            status = "different"
        elif a.err != b.err:
            status = "stderr differs"
        else:
            status = "interface only" if interface else "match"
        self.add(group, name, status, source if args is None else json.dumps(args) + (" stdin=" + repr(stdin) if stdin else ""), pair)

        if status in ("match", "interface only"):
            self.rows[group][name]["reference_result"] = a.brief()
            self.rows[group][name]["workload"] = dict(argv=["--noprofile","--norc","-c",source] if args is None else args,
                                                      stdin=stdin, fixture=fixture if args is None else True,
                                                      expected=a.digest())

    def names(self, source):
        a = self.script(0, source)
        if a.code or a.timeout:
            raise ValueError(f"could not enumerate reference: {source}: {a.brief()}")
        return a.out.decode().splitlines()

    def tty(self, name, method):
        suite = Path(__file__).resolve().parents[1] / "shell.py"
        env = dict(os.environ, REFERENCE_BASH=str(self.shells[0]))
        p = subprocess.run([sys.executable, str(suite), str(self.shells[1]), "-q", "ShellTests." + method],
                           capture_output=True, env=env, timeout=30)
        self.add("Terminal", name, "match" if p.returncode == 0 else "different",
                 "tests/shell.py " + method, [Outcome(p.returncode, p.stdout, p.stderr)])


def sections(path):
    text = path.read_text(errors="replace")
    matches = list(re.finditer(r"^@node ([^,\n]+).*?$", text, re.M))
    return {m[1]: text[m.end(): matches[i+1].start() if i+1 < len(matches) else len(text)]
            for i, m in enumerate(matches)}


def utf8_contract(a):
    # The first output is required from Tonari. The remaining outputs identify
    # the observed Bash behavior, so a new reference result remains visible.
    locale = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
    a.compare("UTF-8 text", "multibyte IFS after whitespace",
              f"LC_ALL={locale}; " + """IFS=' 木'; v='a 木b'; set -- $v; printf '<%s>' "$@" """,
              text_contract=(b"<a><b>", b"<a><><><b>", b"<a><\x9c\xa8b>"))
    a.compare("UTF-8 text", "character length and slice in C locale",
              """LC_ALL=C; v=雪🌸; printf '%s/<%s>' "${#v}" "${v:1:1}" """,
              text_contract=("2/<🌸>".encode(), b"7/<\x9b>"))


def collect(a, source):
    utf8_contract(a)
    ref_version = a.names('echo "$BASH_VERSION"')[0]
    bash = sections(source / "doc/bashref.texi")
    for group, cases in [("Shell language", LANGUAGE), ("Redirection", REDIRS),
                         ("Printf format", PRINTF), ("Parameter", PARAMETERS), ("Arithmetic", ARITHMETIC), ("Conditional", CONDITIONS)]:
        for name, script in cases.items():
            if name in ("-b", "-S", "-t") and group == "Conditional":
                # A false/false pair alone does not prove an implementation.
                if name == "-b":
                    block = next((p for p in sorted(Path("/dev").iterdir()) if p.is_block_device()), None)
                    if block is None:
                        a.add(group, name, "fixture unavailable", "No block device in /dev")
                        continue
                    script = script.replace("/dev/loop0", shlex.quote(str(block)))
                elif name == "-t":
                    a.add(group, name, "partial fixture", "Non-tty tested by the regression suite; tty predicate needs paired endpoints")
                    continue
            if "(5.3)" in name and tuple(map(int, ref_version.split(".")[:2])) < (5, 3):
                a.add(group, name, "not in reference", script)
                continue
            a.compare(group, name, script, fixture="socket" if group == "Conditional" and name == "-S" else False)
    for name, (args, stdin) in INVOCATION.items():
        a.compare("Invocation", name, args=args, stdin=stdin, interface=name in ("--norc", "--noediting"))
    for name in a.names("compgen -k"):
        key = KEYWORDS.get(name)
        if key:
            a.add("Reserved word", name, a.rows["Shell language"][key]["status"], "Shell language / " + key)
        else:
            a.add("Reserved word", name, "unprobed")
    for name in a.names("compgen -b"):
        found = a.script(1, 'type -t ' + repr(name))
        synopsis = a.script(0, 'help -s ' + repr(name)).out.decode().strip().splitlines()
        if found.out.strip() != b"builtin":
            a.add("Builtin", name, "missing interface", f"type -t '{name}'", [a.script(0, 'type -t ' + repr(name)), found])
        elif name in ("bg", "fg"):
            a.add("Builtin", name, a.rows["Terminal"]["foreground pipeline"]["status"], "Terminal / foreground pipeline")
        elif name == "suspend":
            a.tty("suspend and resume", "test_builtin_control_and_suspension")
            a.add("Builtin", name, a.rows["Terminal"]["suspend and resume"]["status"], "Terminal / suspend and resume")
        elif name == "logout":
            a.add("Builtin", name, a.rows["Invocation"]["login logout"]["status"], "Invocation / login logout")
        elif BUILTINS.get(name):
            a.compare("Builtin", name, BUILTINS[name])
        else:
            a.add("Builtin", name, "interface only", "type -t")
        a.rows["Builtin"][name]["synopsis"] = " ".join(synopsis)
    for name in a.names("compgen -A shopt"):
        check = f"shopt -p {name}"
        a0, a1 = [a.script(i, check) for i in (0, 1)]
        if not a1.out.strip() and a0.out.strip():
            a.add("shopt", name, "missing interface", check, (a0, a1))
        elif name in SHOPT:
            a.compare("shopt", name, SHOPT[name])
        else:
            a.compare("shopt", name, f"shopt -s {name}; shopt -p {name}; shopt -u {name}; shopt -p {name}", interface=True)
    for name in a.names("compgen -A setopt"):
        a.compare("set -o", name, SETOPT.get(name, f"set -o {name} && set +o {name} && printf accepted"), interface=name not in SETOPT)
    var_names = sorted(set(re.findall(r"^@item ([A-Za-z_][A-Za-z_0-9]*)\s*$",
                                      bash["Bash Variables"] + bash["Bourne Shell Variables"], re.M)))
    for name in var_names:
        if VARIABLES.get(name):
            contract = None
            if name == "LC_COLLATE" and sys.platform.startswith("linux"):
                contract = ("<A><z><é>".encode(), "<A><é><z>".encode())
            a.compare("Variable", name, VARIABLES[name], text_contract=contract)
            continue
        check = f'printf %s "${{{name}+set}}"'
        pair = [a.script(i, check) for i in (0, 1)]
        if pair[0].out == b"set" and pair[1].out == b"":
            status = "missing default"
        elif pair[0].out == pair[1].out == b"":
            status = "context unprobed"
        elif pair[0].out != pair[1].out:
            status = "different"
        else:
            status = "interface only"
        a.add("Variable", name, status, check, pair)
    for group, source_list in (("Readline command", "bind -l"), ("Readline variable", "bind -v")):
        items = a.names(source_list + " 2>/dev/null")
        candidate = set(a.script(1, source_list + " 2>/dev/null").out.decode().splitlines())
        if group.endswith("variable"):
            candidate = {line.split()[1] for line in candidate if len(line.split()) >= 2}
        for item in items:
            name = item.split()[1] if group.endswith("variable") else item
            check = f"bind -q {name}" if group.endswith("command") else f"bind -v | grep -F 'set {name} '"
            if name in candidate:
                a.compare(group, name, check, interface=True)
            else:
                a.add(group, name, "missing interface", check)
    for protocol in ("tcp", "udp"):
        replies = []
        for i in (0,1):
            kind = socket.SOCK_STREAM if protocol == "tcp" else socket.SOCK_DGRAM
            with socket.socket(socket.AF_INET, kind) as listener:
                listener.bind(("127.0.0.1", 0))
                if protocol == "tcp": listener.listen(1)
                port = listener.getsockname()[1]
                proc = a.script(i, f"printf witness >/dev/{protocol}/127.0.0.1/{port}")
                listener.settimeout(.15)
                try:
                    if protocol == "tcp":
                        peer, _ = listener.accept()
                        with peer: data = peer.recv(128)
                    else: data = listener.recv(128)
                except TimeoutError: data = b""
                replies.append((proc.code, data))
        status = "control inconclusive" if replies[0] != (0, b"witness") else ("match" if replies[0] == replies[1] else "different")
        name = "/dev/" + protocol + "/host/port"
        a.add("Redirection", name, status, "write witness to an ephemeral loopback listener and check received bytes")
        a.rows["Redirection"][name]["samples"] = [repr(r) for r in replies]
    for key in ("bashdefault", "default", "dirnames", "filenames", "fullquote", "noquote", "nosort", "nospace", "plusdirs"):
        a.compare("Completion option", key, f"complete -o {key} -W item app && complete -p app", interface=True)
    return ref_version, bash


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    for arg in ("bash", "tonari", "source"):
        cli.add_argument("--" + arg, type=Path, required=True)
    cli.add_argument("--upstream", type=Path)
    args = cli.parse_args()
    with tempfile.TemporaryDirectory() as work:
        a = Audit(args.bash.resolve(), args.tonari.resolve(), Path(work))
        a.tty("foreground pipeline", "test_stop_and_resume_foreground_pipeline")
        a.tty("interactive alias and signal", "test_interactive_alias_and_interrupt_trap")
        version, manual = collect(a, args.source)
        extra(a, manual, args.source, sections)
        if args.upstream:
            for name, res in json.loads(args.upstream.read_text()).items():
                a.add("GNU regression group", name, res["status"], name)
                a.rows["GNU regression group"][name]["driver"] = res
        hashes = [hashlib.sha256(shell.read_bytes()).hexdigest() for shell in a.shells]
        print(json.dumps(dict(reference=version, executable_sha256=hashes, rows=a.rows), ensure_ascii=True, indent=2))


if __name__ == "__main__":
    main()
