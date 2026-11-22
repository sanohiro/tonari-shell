"""Read the actual first interactive prompt, including Bash prompt escapes."""
import fcntl
import os
import pty
import re
import select
import signal
import subprocess
import termios
import time


def stop(proc, master):
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            proc.kill()
    os.close(master)
    proc.wait()


def prompt(shell, cwd, text):
    master, slave = pty.openpty()
    tty_name = os.fsencode(os.path.basename(os.ttyname(slave)))
    def attach():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    env = dict(HOME=str(cwd), PATH="/usr/bin:/bin", LC_ALL="C", TERM="dumb",
               INPUTRC="/dev/null", PS1="PROMPTBEGIN" + text + "PROMPTEND")
    p = subprocess.Popen(["bash", "--noprofile", "--norc", "--noediting", "-i"], executable=shell,
                         cwd=cwd, stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=attach)
    os.close(slave)
    data = b""
    try:
        deadline = time.monotonic() + 2
        while b"PROMPTEND" not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([master], [], [], remaining)[0]:
                return None
            try:
                data += os.read(master, 8192)
            except OSError:
                return None
        value = data.split(b"PROMPTBEGIN", 1)[1].split(b"PROMPTEND", 1)[0]
        return b"<tty>" if text == r"\l" and value == tty_name else value
    finally:
        stop(p, master)


def audit_prompts(a):
    root = a.root / "prompt"
    root.mkdir()
    values = {
        "a": None, "d": rb"[A-Z][a-z]+ [A-Z][a-z]+ +[0-9]+", "D{%Y}": rb"[0-9]{4}",
        "e": None, "h": None, "H": None, "j": None, "l": None, "n": None, "r": None, "s": None,
        "t": rb"[0-9]{2}:[0-9]{2}:[0-9]{2}", "T": rb"[0-9]{2}:[0-9]{2}:[0-9]{2}",
        "@": rb"[0-9]{2}:[0-9]{2} [AP]M", "A": rb"[0-9]{2}:[0-9]{2}", "u": None, "v": None,
        "V": None, "w": None, "W": None, "!": None, "#": None, "$": None, "101": None,
        "\\": None, "[": None, "]": None,
    }
    for code, shape in values.items():
        source = "\\" + code
        ref, candidate = [prompt(sh, root, source) for sh in a.shells]
        if ref is None or shape and not re.fullmatch(shape, ref, re.I):
            status = "control inconclusive"
        elif candidate is None:
            status = "timeout"
        elif (shape and re.fullmatch(shape, candidate, re.I)) or candidate == ref:
            status = "match"
        else:
            status = "different"
        a.add("Prompt escape", source, status, "interactive PS1 " + source)
        if status == "match":
            a.rows["Prompt escape"][source]["workload"] = dict(prompt=source, shape=shape.decode() if shape else "", expected=ref.hex())
        if status != "match":
            a.rows["Prompt escape"][source]["samples"] = [repr(ref), repr(candidate)]

class Terminal:
    def __init__(self, shell, root, editing=False, env=None):
        self.master, slave = pty.openpty()
        import struct
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 33, 97, 0, 0))
        def attach():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        clean = dict(HOME=str(root), PATH="/usr/bin:/bin", TERM="xterm" if editing else "dumb",
                     LC_ALL="C", PS1="READY>", PS2="MORE>", INPUTRC="/dev/null")
        clean.update(env or {})
        self.proc = subprocess.Popen(["bash", "--noprofile", "--norc", *([] if editing else ["--noediting"]), "-i"],
                                     executable=shell, env=clean, cwd=root, stdin=slave, stdout=slave, stderr=slave,
                                     preexec_fn=attach)
        os.close(slave)

    def read(self, marker, timeout=1.2):
        data = b""
        deadline = time.monotonic() + timeout
        while marker not in data:
            left = deadline - time.monotonic()
            if left <= 0 or not select.select([self.master], [], [], left)[0]:
                break
            try:
                data += os.read(self.master, 16384)
            except OSError:
                break
        return data

    def send(self, text):
        os.write(self.master, text)

    def close(self):
        stop(self.proc, self.master)


def dialogue(shell, root, name):
    observed = root / "capture"
    observed.unlink(missing_ok=True)
    edits = name in ("completion", "FIGNORE", "EMACS", "INSIDE_EMACS", "read -e", "read -E", "read -i", "READLINE")
    env = {name: "t" if name == "EMACS" else "29.1,comint"} if name in ("EMACS", "INSIDE_EMACS") else {}
    mail = root / "mailbox"
    if name in ("MAIL", "MAILCHECK", "MAILPATH"):
        mail.write_bytes(b"old\n")
        os.utime(mail, (1, 1))
        env.update(MAIL=str(mail), MAILCHECK="0")
        if name == "MAILPATH": env["MAILPATH"] = str(mail) + "?AUDITMAIL"
    t = Terminal(shell, root, edits, env)
    try:
        first = t.read(b"READY>")
        if b"READY>" not in first:
            return b"no initial prompt"
        if name == "tty predicate":
            t.send(b'[[ -t 0 ]] && prefix=tty; [[ -t 12 ]] || printf "%sother" "$prefix" >capture\n')
        elif name == "PS1":
            return b"READY>"
        elif name == "PS2":
            t.send(b"if true; then\n")
            return b"MORE>" if b"MORE>" in t.read(b"MORE>") else b"missing"
        elif name in ("COLUMNS", "LINES"):
            t.send(b"printf '%s,%s' \"$COLUMNS\" \"$LINES\" >capture\n")
        elif name == "PROMPT_COMMAND":
            t.send(b"PROMPT_COMMAND='printf hook >capture'\n")
        elif name == "PS0":
            t.send(b"PS0=EXEC-START\n")
            t.read(b"READY>")
            t.send(b":\n")
            return b"active" if b"EXEC-START" in t.read(b"READY>") else b"missing"
        elif name == "PROMPT_DIRTRIM":
            t.send(b"mkdir -p alphadir/betadir/c; cd alphadir/betadir/c; PROMPT_DIRTRIM=1; PS1='\\w>'\n")
            return b"trimmed" if b".../c>" in t.read(b"/c>") else b"missing"
        elif name in ("MAIL", "MAILPATH", "MAILCHECK"):
            with mail.open("ab") as f: f.write(b"new mail\n")
            os.utime(mail, (1, int(time.time()) + 2))
            t.send(b":\n")
            text = t.read(b"READY>")
            seen = b"AUDITMAIL" in text if name == "MAILPATH" else b"mail" in text.lower()
            return b"notified" if seen else b"no mail notification"
        elif name == "TMOUT":
            t.send(b"TMOUT=1\n")
            t.read(b"READY>")
            try:
                t.proc.wait(timeout=2)
                return b"expired"
            except subprocess.TimeoutExpired:
                return b"still running"
        elif name == "IGNOREEOF":
            t.send(b"IGNOREEOF=2\n")
            t.read(b"READY>")
            t.send(b"\x04")
            return b"continues" if b"READY>" in t.read(b"READY>") else b"exited"
        elif name == "FIGNORE":
            for file in ("audit-a.txt", "audit-a.tmp"):
                (root / file).touch()
            t.send(b"FIGNORE=.tmp\n")
            t.read(b"READY>")
            t.send(b"printf '%s' audit-a.\t >capture\n")
        elif name == "completion":
            t.send(b'f(){ printf "%s\\0" "$COMP_LINE" "$COMP_POINT" "$COMP_CWORD" "$COMP_TYPE" "$COMP_KEY" "${COMP_WORDS[@]}" >capture; COMPREPLY=(apple); }; complete -F f example\n')
            t.read(b"READY>")
            t.send(b"example ap\t")
        elif name == "READLINE":
            t.send(b"""f(){ printf '%s\\0' "$READLINE_LINE" "$READLINE_POINT" "$READLINE_MARK" "$READLINE_ARGUMENT" >capture; }; bind -x '"\\C-xr":f'\n""")
            t.read(b"READY>")
            t.send(b"abc\x18r")
        elif name in ("EMACS", "INSIDE_EMACS"):
            t.send(b"if shopt -qo emacs; then echo editing; else echo plain; fi >capture\n")
        elif name == "read -p":
            t.send(b"read -p 'A''SK>' v; printf %s \"$v\" > capture\n")
            seen = b"ASK>" in t.read(b"ASK>")
            t.send(b"answer\n")
            return b"asked" if seen else b"missing"
        elif name == "read -s":
            t.send(b"""read -s -p 'A''SK>' v; printf '%s' "$v" >capture; printf 'DO%sE' N\n""")
            if b"ASK>" not in t.read(b"ASK>", .5):
                return b"missing prompt"
            t.send(b"hidden-input\n")
            output = t.read(b"DONE")
            if not observed.exists() or observed.read_bytes() != b"hidden-input":
                return b"missing value"
            return b"echoed" if b"hidden-input" in output else b"secret"
        elif name in ("read -e", "read -E", "read -i"):
            opts = {"read -e": "-e", "read -E": "-E", "read -i": "-e -i initial"}[name]
            t.send(f'read {opts} -p "IN""PUT>" v; printf %s "$v" >capture\n'.encode())
            t.read(b"INPUT>", .5)
            t.send(b"tail\x01X\n")
        else:
            raise ValueError(name)
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline:
            if observed.exists() and (data := observed.read_bytes()):
                return data
            # Drain redraws while waiting. A macOS PTY can block Readline in
            # tcdrain() before it dispatches the line if its master is idle.
            if select.select([t.master], [], [], .005)[0]:
                try:
                    os.read(t.master, 16384)
                except OSError:
                    break
        return b"no capture"
    finally:
        t.close()
