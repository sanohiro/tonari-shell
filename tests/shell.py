"""End-to-end tests for Tonari's -c and terminal entry points."""

import errno
import fcntl
import os
import pty
import resource
import select
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


TONARI = Path(sys.argv.pop(1)).resolve()
BASH = Path(os.environ.get("REFERENCE_BASH", "/bin/bash"))
PROMPT = b"tonari$ "


class ShellTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.cwd = Path(directory.name)
        self.env = {"HOME": directory.name, "PATH": "/usr/bin:/bin", "LC_ALL": "C"}

    def command(self, shell, source, *args):
        return subprocess.run(
            [shell, "-c", source, *args], cwd=self.cwd, env=self.env,
            capture_output=True, timeout=5,
        )

    def compare(self, source, code, stdout=b""):
        for shell in [BASH, TONARI]:
            with self.subTest(shell=shell, source=source):
                proc = self.command(shell, source)
                self.assertEqual(proc.returncode, code, proc.stderr)
                self.assertEqual(proc.stdout, stdout)
                if code == 0 and "missing-tonari" not in source:
                    self.assertEqual(proc.stderr, b"")

    def test_arguments_and_lists(self):
        self.compare(r"""printf '<%s>\n' '' "こん"'にちは' a\ b""", 0,
                     "<>\n<こんにちは>\n<a b>\n".encode())
        self.compare("false; # comment\nexit", 1)
        self.compare("exit 253; printf unreachable", 253)
        self.compare("exit -1", 255)
        # Bash 5.x returns 2 here. The legacy macOS system Bash returns 255.
        version = self.command(BASH, 'printf %s "${BASH_VERSINFO[0]}"')
        if int(version.stdout) >= 5:
            self.compare("exit invalid", 2)
        else:
            self.assertEqual(self.command(TONARI, "exit invalid").returncode, 2)

    def test_pipelines(self):
        self.compare("printf abc | cat | cat", 0, b"abc")
        self.compare(r"printf '\377\000' | cat", 0, b"\xff\x00")
        self.compare("yes | head -n 1", 0, b"y\n")
        self.compare("exit 42 | cat; printf ok", 0, b"ok")
        self.compare("missing-tonari-command | cat", 0)
        self.compare("printf abc | missing-tonari-command", 127)
        self.compare("printf abc | exit 42", 42)
        self.compare("''", 127)
        tool = self.cwd / "not-executable"
        tool.write_text("plain file")
        self.compare("./not-executable", 126)
        self.compare("cat < absent", 1)

    def test_count_pipeline(self):
        for name in ["alpha", "beta", "count"]:
            (self.cwd / name).touch()
        for shell in [BASH, TONARI]:
            with self.subTest(shell=shell):
                (self.cwd / "count").write_text("previous contents")
                proc = self.command(shell, "ls -1 | wc -l > count")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"", b""))
                self.assertEqual(int((self.cwd / "count").read_text()), 3)

    def test_boolean_builtins(self):
        for name, code in [("true", 0), ("false", 1)]:
            tool = self.cwd / name
            tool.write_text("#!/bin/sh\nexit 23\n")
            tool.chmod(0o755)
            self.env["PATH"] = f"{self.cwd}:/usr/bin:/bin"
            self.compare(name, code)
            self.compare(f"{name} --help ignored", code)
            self.compare(f"./{name}", 23)
        self.compare("true | false", 1)
        self.compare("false | true", 0)
        self.compare("false; exit", 1)

    def test_file_and_descriptor_order(self):
        fixture = "/bin/sh -c 'printf out; printf err >&2'"
        for shell in [BASH, TONARI]:
            with self.subTest(shell=shell):
                proc = self.command(shell, fixture + " 2>&1 > data | cat")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"err", b""))
                self.assertEqual((self.cwd / "data").read_bytes(), b"out")
                proc = self.command(shell, fixture + " > data 2>&1 | cat")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"", b""))
                self.assertEqual((self.cwd / "data").read_bytes(), b"outerr")
                proc = self.command(shell, ": > first > data; printf a >> data; cat < data")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"a", b""))
                self.assertEqual((self.cwd / "first").read_bytes(), b"")
                self.assertEqual(self.command(shell, ": >&-; printf restored").stdout, b"restored")

                proc = self.command(shell, "true > first < absent; printf restored")
                self.assertEqual((proc.returncode, proc.stdout), (0, b"restored"))
                self.assertTrue(proc.stderr)
                self.assertEqual((self.cwd / "first").read_bytes(), b"")
                proc = self.command(shell, "false > first > data")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (1, b"", b""))
                self.assertEqual((self.cwd / "data").read_bytes(), b"")

    def test_blocking_fifo_opens_do_not_serialize_a_pipeline(self):
        os.mkfifo(self.cwd / "feed")
        self.compare("cat < feed > data | printf x > feed; cat data", 0, b"x")

    def test_silent_builtin_file_redirections(self):
        file = self.cwd / "data"
        for shell in [BASH, TONARI]:
            with self.subTest(shell=shell):
                file.write_bytes(b"keep")
                proc = self.command(shell, "false >> data")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (1, b"", b""))
                self.assertEqual(file.read_bytes(), b"keep")
                proc = self.command(shell, "true 2>data; printf kept")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"kept", b""))
                self.assertEqual(file.read_bytes(), b"")
        # A backup temporarily allocated by the shell must not become a valid input.
        self.compare("true < /dev/fd/3", 1)

    def test_silent_builtin_fifo_open(self):
        fifo = self.cwd / "feed"
        os.mkfifo(fifo)
        with subprocess.Popen([sys.executable, "-c",
                               "import os, sys; os.close(os.open(sys.argv[1], os.O_WRONLY))",
                               str(fifo)]) as writer:
            try:
                proc = self.command(TONARI, "true < feed; printf ready")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"ready", b""))
                self.assertEqual(writer.wait(timeout=5), 0)
            finally:
                writer.kill()

    def test_silent_builtin_at_descriptor_limit(self):
        # With one spare fd, Bash 5 opens and truncates the file before its
        # attempt to save stdout fails. A second spare fd allows the redirection.
        for limit, status in [(4, 1), (5, 0)]:
            with self.subTest(limit=limit):
                file = self.cwd / "data"
                file.write_bytes(b"old")
                proc = subprocess.run(
                    [TONARI, "-c", "true >data; exit"], cwd=self.cwd, env=self.env,
                    preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_NOFILE, (limit, limit)),
                    capture_output=True, timeout=5,
                )
                self.assertEqual(proc.returncode, status, proc.stderr)
                self.assertEqual(file.read_bytes(), b"")

    def test_parse_errors_precede_execution(self):
        for source in ["touch side-effect; printf '", "touch side-effect | | cat",
                       "touch side-effect; echo ${", "if ; then touch side-effect; fi"]:
            proc = self.command(TONARI, source)
            self.assertEqual(proc.returncode, 2)
            self.assertTrue(proc.stderr)
            self.assertFalse((self.cwd / "side-effect").exists())

    def test_conditionals_and_last_status(self):
        self.compare("false && exit 23 || true && printf ok", 0, b"ok")
        self.compare("true || false && printf ok", 0, b"ok")
        self.compare("false ||\n# continued\nprintf '%s' \"$?\"", 0, b"1")
        self.compare("false && printf x > side-effect; exit", 1)
        self.assertFalse((self.cwd / "side-effect").exists())

    def test_variables_and_argument_boundaries(self):
        self.compare("a='a  b'; empty=''; printf '<%s>' $a \"$a\" $empty \"$empty\"", 0,
                     b"<a><b><a  b><>")
        self.compare("a=foo; unset a; printf '%s,%s' \"${a:-default}\" \"${a+set}\"", 0,
                     b"default,")
        self.compare("x=old; x=new /bin/sh -c 'printf %s \"$x\"'; printf ':%s' \"$x\"", 0,
                     b"new:old")
        self.compare("v=private; /bin/sh -c 'printf %s \"${v-unset}\"'; export v; "
                     "/bin/sh -c 'printf :%s \"$v\"'; unset v; "
                     "/bin/sh -c 'printf :%s \"${v-unset}\"'", 0, b"unset:private:unset")
        self.compare("s='a b'; export V=$s; /bin/sh -c 'printf %s \"$V\"'", 0, b"a b")
        self.compare("x=parent; x=child | cat; printf %s \"$x\"", 0, b"parent")
        for shell in [BASH, TONARI]:
            proc = self.command(shell, 'printf "<%s>" "$0" "$#" "$@"', "job", "a b", "", "雪")
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, "<job><3><a b><><雪>".encode(), b""))

    def test_directory_and_child_environment(self):
        physical = self.cwd.resolve()
        (self.cwd / "real").mkdir()
        (self.cwd / "link").symlink_to("real", target_is_directory=True)
        self.compare("cd link && pwd -L && pwd -P && cd .. && pwd", 0,
                     f"{physical}/link\n{physical}/real\n{physical}\n".encode())
        self.compare("cd real | cat; pwd", 0, f"{physical}\n".encode())
        self.compare("cd real && /bin/sh -c 'printf %s \"$PWD\"'", 0,
                     f"{physical}/real".encode())

    def test_script_source_eval_and_stdin(self):
        lib = self.cwd / "lib"
        lib.write_text('export G="from $1"\nreturn 7\nprintf unreachable\n')
        script = self.cwd / "main"
        script.write_text('. ./lib inner\nprintf "%s:%s:%s" "$?" "$G" "$1"\n')
        for shell in [BASH, TONARI]:
            cmd = [shell, str(script), "outer"]
            proc = subprocess.run(cmd, cwd=self.cwd, env=self.env, capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"7:from inner:outer", b""))
            proc = subprocess.run([shell, "-s", "--", "hello world"], input=b'printf "%s" "$1"\ncat\nrest\n',
                                  cwd=self.cwd, env=self.env, capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"hello worldrest\n", b""))
            stream = self.cwd / "stream"
            stream.write_text("cat\nremaining file bytes\n")
            with stream.open() as src:
                proc = subprocess.run([shell], stdin=src, cwd=self.cwd, env=self.env,
                                      capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"remaining file bytes\n", b""))
        self.compare("eval 'v=hello'; printf %s \"$v\"", 0, b"hello")
        self.compare("exit 12\nprintf 'unterminated", 12)
        self.compare("printf first\nprintf 'unterminated", 2, b"first")
        proc = subprocess.run([TONARI, "-n", "-c", "touch unexpected"], cwd=self.cwd,
                              env=self.env, capture_output=True, timeout=5)
        self.assertEqual(proc.returncode, 0)
        self.assertFalse((self.cwd / "unexpected").exists())

    def test_bash_env(self):
        startup = self.cwd / "startup"
        startup.write_text('export MARK=from_bash_env\n')
        self.env["BASH_ENV"] = str(startup)
        self.compare("printf %s \"$MARK\"", 0, b"from_bash_env")
        # Bash reads BASH_ENV even with profile and interactive rc disabled.
        for shell in [BASH, TONARI]:
            p = subprocess.run([shell, "--noprofile", "--norc", "-c", 'printf %s "$MARK"'],
                               cwd=self.cwd, env=self.env, capture_output=True, timeout=5)
            self.assertEqual((p.returncode, p.stdout), (0, b"from_bash_env"))

    def test_compounds_and_functions(self):
        cases = [
            ("if false; then echo X; elif true; then echo yes; else echo X; fi", b"yes\n"),
            ("if false; then false; fi; printf %s \"$?\"", b"0"),
            ("for v in 'a b' c; do echo \"<$v>\"; done", b"<a b>\n<c>\n"),
            ("v=y; while [ -n \"$v\" ]; do echo loop; unset v; done", b"loop\n"),
            ("for a in 1 2; do for b in 3 4; do echo \"$a$b\"; break 2; done; done", b"13\n"),
            ("f() { local x=inner; for i; do printf '<%s>' \"$i\"; done; return 7; }; "
             "x=outer; f a 'b c'; printf ':%s:%s' \"$?\" \"$x\"", b"<a><b c>:7:outer"),
            ("f() { cd /; x=changed; }; x=outer; f | cat; printf %s \"$x\"", b"outer"),
        ]
        for source, expected in cases:
            self.compare(source, 0, expected)

    def test_substitution_and_shell_options(self):
        self.compare("x=outer; v=$(x=inner; printf '%s\\n\\n' \"$x\"); echo \"$x:$v\"", 0, b"outer:inner\n")
        self.compare("x=$(false); printf %s \"$?\"", 0, b"1")
        self.compare("n=no; echo \"${n:-$(touch unexpected)}\"", 0, b"no\n")
        self.assertFalse((self.cwd / "unexpected").exists())
        for shell in [BASH, TONARI]:
            # Use an assignment so the captured value can exceed the kernel's argv limit.
            proc = self.command(shell, "v=$(yes x | head -c 262144); echo ok")
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"ok\n", b""))
            proc = subprocess.run([shell, "-eu", "-o", "pipefail", "-c",
                                   "false && echo skipped; false | true; echo unreachable"],
                                  cwd=self.cwd, env=self.env, capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (1, b"", b""))
            proc = self.command(shell, "set -u; printf '%s' \"$MISSING\"; echo unreachable")
            self.assertNotEqual(proc.returncode, 0)
            self.assertEqual(proc.stdout, b"")
            self.assertTrue(proc.stderr)
        self.compare('set -- a "b c"; shift; echo "$#:$1"', 0, b"1:b c\n")
        self.compare("set -e; f() { false; echo recovered; }; if f; then echo yes; fi",
                     0, b"recovered\nyes\n")

    def test_arithmetic_patterns_and_globs(self):
        (self.cwd / "a.x").touch()
        (self.cwd / "b.x").touch()
        (self.cwd / ".hidden.x").touch()
        self.compare('p="*.x"; printf "<%s>" $p "$p" missing*.x; set -f; '
                     'for i in *.x; do printf "<%s>" "$i"; done',
                     0, b"<a.x><b.x><*.x><missing*.x><*.x>")
        self.compare('v=abc12; if [[ $v == a* && $v =~ ^(ab|c).*([0-9]+)$ ]]; '
                     'then case $v in abc*) printf match;; esac; fi; '
                     'for ((i=0; i<3; i++)); do printf :%s "$((2*i))"; done',
                     0, b"match:0:2:4")
        self.compare('v=no; [[ $v == "n*" ]] || printf literal; '
                     '[[ ok == ok || $(touch unexpected) == yes ]]', 0, b"literal")
        self.assertFalse((self.cwd / "unexpected").exists())

    def test_heredoc_and_file_substitution(self):
        self.compare("v='two words'; cat <<EOF\n$v $(printf three)\nEOF\n"
                     "cat <<-'END'\n\t$v and `false`\n\tEND\n"
                     'cat <<< "$v" | cat', 0, b"two words three\n$v and `false`\ntwo words\n")
        self.compare("cat <<ONE <<'TWO'\nfirst\nONE\nsecond\nTWO\n", 0, b"second\n")
        self.compare('p="space name"; printf "data\\n\\n" > "$p"; x=$(<"$p"); '
                     'printf "<%s>:%s" "$x" "$?"', 0, b"<data>:0")
        for shell in [TONARI, BASH]:
            proc = self.command(shell, "x=$(<absent); printf '<%s>:%s' \"$x\" \"$?\"")
            self.assertEqual((proc.returncode, proc.stdout), (0, b"<>:1"))
            self.assertTrue(proc.stderr)
            document = b"xx\n" * 40000
            proc = subprocess.run([shell], input=b"cat <<'EOF'\n" + document + b"EOF\n",
                                  capture_output=True, cwd=self.cwd, env=self.env, timeout=8)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, document, b""))

    def test_shell_read_printf_and_lookup(self):
        self.compare("IFS=: read -r a b <<< 'x:y:z'; printf -v out '<%s><%s>' \"$a\" \"$b\"; "
                     "printf '%s\\n%b' \"$out\" 'a\\0101'",
                     0, b"<x><y:z>\naA")
        self.compare("printf x | echo \"$(cat)\"", 0, b"x\n")
        self.compare("f() { echo fn; }; type -t f; command -v f; "
                     "type -t printf; builtin printf '%s' ok",
                     0, b"function\nf\nbuiltin\nok")

    def test_async_and_wait(self):
        self.compare("/bin/sh -c 'exit 17' & p=$!; wait \"$p\"; printf %s \"$?\"", 0, b"17")
        self.compare("false && touch unexpected & wait; printf ok", 0, b"ok")
        self.assertFalse((self.cwd / "unexpected").exists())
        self.compare("x=parent; (x=child; cd /; printf %s \"$x\"); printf :%s \"$x\"",
                     0, b"child:parent")
        self.compare("true | /bin/sh -c 'exit 9' & wait $!; printf %s \"$?\"", 0, b"9")

    def test_indexed_arrays_patterns_and_scope(self):
        self.compare('a=(one "two words" ""); a[5]=five; a+=("six"); '
                     'f() { local -a a=(inner); printf "<%s>" "${a[@]}"; }; '
                     'f; printf "|%s" "${!a[@]}"; printf "<%s>" "${a[@]}"',
                     0, b"<inner>|0|1|2|5|6<one><two words><><five><six>")
        self.compare('[[ "file42" =~ ^(file)([0-9]+)(x)?$ ]]; '
                     'printf "<%s>" "${BASH_REMATCH[@]}"', 0, b"<file42><file><42><>")
        self.compare('x=/a/b/x.txt; printf "<%s>" "${x##*/}" "${x%.*}" "${x#*/}" "${x%%/*}"',
                     0, b"<x.txt></a/b/x><a/b/x.txt><>")
        self.compare('n=2; printf "<%s>" "$((n++))" "$n" "$((++n))"', 0, b"<2><3><4>")

    def test_aliases_are_applied_when_commands_are_read(self):
        self.compare("shopt -s expand_aliases\nalias a='printf yes | cat'\n"
                     "say() { a; }; unalias a; say; echo .", 0, b"yes.\n")
        self.compare("shopt -s expand_aliases\nalias headword='echo '; alias arg=expanded\n"
                     "headword arg; headword 'arg'", 0, b"expanded\narg\n")
        self.compare("shopt -s expand_aliases\nalias f='printf nested'\necho \"$(f)\"",
                     0, b"nested\n")

    def test_numbered_descriptors_and_exec(self):
        self.compare("exec 3>file; echo first >&3; f() { echo second >&3; }; f >unused; "
                     "exec 3>&-; cat file; test ! -s unused", 0, b"first\nsecond\n")
        self.compare('v=ok; export v; p=$$; export p; exec /bin/sh -c '
                     """'test "$p" = "$$" && printf %s "$v"'""", 0, b"ok")
        path = self.cwd / "redirect-script"
        path.write_text("exec 10>out\n" + "#padding\n" * 1800 + "printf end >&10\n")
        for shell in (TONARI, BASH):
            proc = subprocess.run([shell, path], cwd=self.cwd, env=self.env,
                                  capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stderr), (0, b""))
            self.assertEqual((self.cwd / "out").read_text(), "end")

    def test_profile_source_names_and_environment_path(self):
        (self.cwd / "env file").write_text("export LOADED=env\n")
        self.env["BASH_ENV"] = "$HOME/env file"
        self.compare('printf %s "$LOADED"', 0, b"env")
        (self.cwd / "fn").write_text('f() { printf "<%s>" "${BASH_SOURCE[0]}" "${FUNCNAME[0]}"; }\n')
        self.compare('. ./fn; f', 0, b"<./fn><f>")
        self.env["SHELL"] = "/some/other/shell"
        self.compare('printf %s "$SHELL"', 0, b"/some/other/shell")

    def terminal_session(self, editing=False):
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.chdir(self.cwd)
                env = dict(self.env, TERM="xterm-256color" if editing else "dumb")
                os.execve(TONARI, [str(TONARI), "--norc"], env)
            finally:
                os._exit(125)
        def close():
            try:
                foreground = os.tcgetpgrp(fd)
                if foreground > 0 and foreground != pid and foreground != os.getpgrp():
                    os.killpg(foreground, signal.SIGKILL)
            except OSError:
                pass
            try:
                os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                os.waitpid(pid, 0)
            except ChildProcessError:
                pass
            os.close(fd)
        self.addCleanup(close)
        self.read_until(fd, PROMPT)
        return fd

    def test_stop_and_resume_foreground_pipeline(self):
        fd = self.terminal_session()
        os.write(fd, b"/bin/sh -c 'printf R%sY EAD; exec sleep 20' | cat\n")
        self.read_until(fd, b"READY")
        os.write(fd, b"\x1a")
        self.assertIn(b"Stopped", self.read_until(fd, PROMPT))
        os.write(fd, b"jobs\n")
        self.assertIn(b"Stopped", self.read_until(fd, PROMPT))
        os.write(fd, b"bg\n")
        self.read_until(fd, PROMPT)
        os.write(fd, b"jobs\n")
        self.assertIn(b"Running", self.read_until(fd, PROMPT))
        shell_group = os.tcgetpgrp(fd)
        os.write(fd, b"fg\n")
        end = time.monotonic() + 3
        while os.tcgetpgrp(fd) == shell_group and time.monotonic() < end:
            time.sleep(0.005)
        self.assertNotEqual(os.tcgetpgrp(fd), shell_group)
        os.write(fd, b"\x03")
        self.read_until(fd, PROMPT)
        os.write(fd, b"printf 'STATUS=%s\\n' $?\n")
        self.assertIn(b"STATUS=130\r\n", self.read_until(fd, PROMPT))

    def test_completion_command_callbacks(self):
        source = r"""marker=parent
callback() {
    marker=child
    printf '%s\n' "$2"
}
compgen -C callback -- '$(touch should-not-exist)' 2>/dev/null
printf '%s\n' "$marker"
"""
        self.compare(source, 0, b"$(touch should-not-exist)\nparent\n")
        self.assertFalse((self.cwd / "should-not-exist").exists())

        terminal = self.terminal_session(editing=True)
        os.write(terminal, b'reply() { COMPREPLY=("$2-$COMP_CWORD-${COMP_WORDS[0]}"); }; complete -F reply echo\r')
        self.read_until(terminal, b"\x1b[?2004h")
        os.write(terminal, b"echo target\t> completion-result\r")
        self.read_until(terminal, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "completion-result").read_text(), "target-1-echo\n")

    def test_editor_paste_suggestions_completion_and_shared_history(self):
        self.env["HISTCONTROL"] = "ignoreboth"
        fd = self.terminal_session(editing=True)
        other = self.terminal_session(editing=True)
        editing = b"\x1b[?2004h"

        def enter(terminal, text):
            os.write(terminal, text + b"\r")
            return self.read_until(terminal, editing)

        # Paste is one edit, including its final newline. A separate Enter runs it.
        os.write(fd, b"\x1b[200~printf 'A\\n' >> pasted\nprintf 'B\\n' >> pasted\n\x1b[201~")
        self.read_until(fd, b"pasted")
        self.assertFalse((self.cwd / "pasted").exists())
        enter(fd, b"")
        self.assertEqual((self.cwd / "pasted").read_bytes(), b"A\nB\n")
        (self.cwd / "a path").write_text("worked\n")
        self.assertIn(b"worked\r\n", enter(fd, b"cat a\\ p\t"))
        enter(fd, b"complete -W 'alpha beta' printf")
        self.assertIn(b"alpha", enter(fd, b"printf alp\t"))
        enter(fd, b"echo violet >> recalled")
        enter(fd, b"echo viol\x1b[C")
        enter(other, b"\x12violet")
        self.assertEqual((self.cwd / "recalled").read_text().splitlines(), ["violet"] * 3)
        enter(other, b" echo omit-secret > /dev/null")
        journal = self.cwd / ".local/state/tonari/history"
        self.assertNotIn("omit-secret", journal.read_text())
        self.assertFalse((self.cwd / ".bash_history").exists())
        # Remove a complete UTF-8 character and insert a replacement at the caret.
        enter(fd, 'echo 猫雪'.encode() + b'\x1b[D\x7f' + '犬'.encode() + b'\x1b[F > unicode')
        self.assertEqual((self.cwd / "unicode").read_text(), "犬雪\n")

    def test_editor_reads_complete_history_rows_while_a_writer_appends(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        row = b"1\t1\t0\t/tmp\t"
        journal.write_bytes((row + b"printf padding\n") * 700
                            + row + b"echo shared > restored\n")
        fd = self.terminal_session(editing=True)
        with journal.open("ab", buffering=0) as writer:
            fcntl.flock(writer, fcntl.LOCK_EX)
            writer.write(row + b"echo shell > partial")
            os.write(fd, b"echo sh")
            # The newest complete row supplies the suffix while the next row is in flight.
            self.read_until(fd, b"\x1b[2mared > restored")
            writer.write(b"\n")
        os.write(fd, b"\x1b[C\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "restored").read_text(), "shared\n")
        # A fresh prompt picks up the completed append, even with Right already queued.
        os.write(fd, b"echo she\x1b[C\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "partial").read_text(), "shell\n")

    def test_terminal(self):
        # Precreate count so ls has the same input regardless of stage launch order.
        for name in ["one", "two", "count"]:
            (self.cwd / name).touch()
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.chdir(self.cwd)
                os.execve(TONARI, [str(TONARI), "--norc"], self.env)
            finally:
                os._exit(125)
        try:
            self.read_until(fd, PROMPT)
            os.write(fd, b"ls -1 | wc -l > count\n")
            self.read_until(fd, PROMPT)
            self.assertEqual((self.cwd / "count").read_text().strip(), "3")
            os.write(fd, b"printf 'A\\n' |\n")
            self.read_until(fd, b"> ")
            os.write(fd, b"cat\n")
            self.assertIn(b"A\r\n", self.read_until(fd, PROMPT))
            # Wait for the foreground process to start before sending a real Ctrl-C.
            os.write(fd, b"/bin/sh -c 'printf R%sY EAD; exec sleep 20' | cat\n")
            self.read_until(fd, b"READY")
            os.write(fd, b"\x03")
            self.read_until(fd, PROMPT)
            os.write(fd, b"exit\n")
            self.read_until(fd, None)
            _, status = os.waitpid(pid, 0)
            pid = None
            self.assertEqual(os.waitstatus_to_exitcode(status), 130)
        finally:
            if pid is not None:
                try:
                    os.killpg(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                os.waitpid(pid, 0)
            os.close(fd)

    def read_until(self, fd, marker):
        deadline = time.monotonic() + 5
        data = b""
        while marker is None or marker not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
                self.fail(f"terminal timeout waiting for {marker!r}: {data!r}")
            try:
                chunk = os.read(fd, 4096)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                chunk = b""
            if not chunk:
                self.assertIsNone(marker, data)
                break
            data += chunk
        return data


if __name__ == "__main__":
    unittest.main()
