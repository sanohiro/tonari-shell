"""End-to-end tests for Tonari's -c and terminal entry points."""

import errno
import fcntl
import os
import pty
import resource
import select
import shlex
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

    def shells_for(self, minimum=0):
        if minimum:
            version = self.command(BASH, 'echo "${BASH_VERSINFO[0]}"')
            if int(version.stdout) < minimum:
                return (TONARI,)
        return (BASH, TONARI)

    def compare(self, source, code, stdout=b"", minimum=0):
        for shell in self.shells_for(minimum):
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

    def test_shell_word_escapes_preserve_argv(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        source = r"""set -- '' a\ b 'a\b' 'don'\''t' "\$v" "\`" "\\x" "\q" \" $'\'\\' \
'雪'🌱 a\
b "c\
d" $'x\ty\nz' $'\303\251\xC3\xA9' $'a\0discard'b
/bin/sh -c 'printf "%s\0" "$#" "$@"' sh "$@" """
        words = ["", "a b", "a\\b", "don't", "$v", "`", "\\x", "\\q", '"', "'\\",
                 "雪🌱", "ab", "cd", "x\ty\nz", "éé", "ab"]
        expected = b"\0".join(w.encode() for w in [str(len(words)), *words]) + b"\0"
        self.compare(source, 0, expected)
        if int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) >= 5:
            self.compare(r"""printf '%s\0' $'\u96EA\U0001F331'""", 0, "雪🌱\0".encode())

    def test_printf_q_round_trip(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        words = ["", "a b", "\t\n", "\\", "'", '"', "`", "$x", "$(echo oops)", "a; b", "*?[", "雪🌱"]
        source = r"""for w do printf -v encoded '%q' "$w"; eval "set -- $encoded"
printf '%s\0' "$#" "$@"; done"""
        expected = b"".join(b"1\0" + word.encode() + b"\0" for word in words)
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                p = self.command(shell, source, "sh", *words)
                self.assertEqual((p.returncode, p.stdout, p.stderr), (0, expected, b""))

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

    def test_merged_external_pipeline(self):
        # 2>&1 | also exercises the lowering of |& on older reference shells.
        for join in ["2>&1 |", "|&"]:
            shells = [BASH, TONARI]
            if join == "|&" and int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) < 4:
                shells = [TONARI]
            for shell in shells:
                with self.subTest(shell=shell, join=join):
                    source = ("""set -o pipefail; printf input | /bin/sh -c """
                              """'cat; printf err >&2; exit 23' """ + join + " /bin/cat")
                    proc = self.command(shell, source)
                    self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (23, b"inputerr", b""))
                    proc = self.command(shell, "set -o pipefail; ./missing-tonari-exe " + join + " /bin/cat")
                    self.assertEqual((proc.returncode, proc.stderr), (127, b""))
                    self.assertIn(b"missing-tonari-exe", proc.stdout)

    def test_external_standard_descriptor_copies(self):
        cmd = """VALUE=tag /bin/sh -c 'printf "$VALUE"; printf err >&2; exit 23' """
        denied = self.cwd / "no-exec"
        denied.write_text("exit 0")
        for shell in [BASH, TONARI]:
            for redirects, stdout, stderr in [("2>&1 1>&2", b"tagerr", b""),
                                              ("1>&2 2>&1", b"", b"tagerr")]:
                with self.subTest(shell=shell, redirects=redirects):
                    proc = self.command(shell, cmd + redirects + '; exit $?')
                    self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (23, stdout, stderr))
            with self.subTest(shell=shell):
                proc = self.command(shell, """printf x | /bin/sh -c 'cat; printf e >&2; exit 23' 2>&1; exit $?""")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (23, b"xe", b""))
                proc = self.command(shell, "exec 1>&-; /usr/bin/true 2>&1; exit $?")
                self.assertEqual((proc.returncode, proc.stdout), (1, b""))
                self.assertTrue(proc.stderr)
                for name, status in [("./no-exec", 126), ("./no-file", 127)]:
                    proc = self.command(shell, name + " 2>&1; exit $?")
                    self.assertEqual((proc.returncode, proc.stderr), (status, b""))
                    self.assertIn(name.encode(), proc.stdout)

    def test_ifs_separators_and_escaped_read_fields(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare('IFS=" :"; v=" :α::β: "; set -- $v; printf "<%s>" "$@"; '
                     'IFS=😀; v="左😀😀右😀"; set -- $v; printf "<%s>" "$@"; '
                     'read -r a b <<< "😀左😀😀右😀"; printf "<%s>" "$a" "$b"; '
                     r'IFS=: read a b <<< "a\:b:c:d:"; printf "<%s>" "$a" "$b"; '
                     'IFS=; v=" a:b "; set -- $v; printf "<%s>" "$@"',
                     0, "<><α><><β><左><><右><><左😀😀右😀><a:b><c:d:>< a:b >".encode())
        # Reference builds disagree on whitespace followed by a multibyte IFS
        # delimiter. Keep each UTF-8 scalar intact when consuming the delimiter.
        proc = self.command(TONARI, 'IFS=" 木"; v="a 木b"; set -- $v; printf "<%s>" "$@"')
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"<a><b>", b""))

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

    def test_utf8_expansions_and_regex_captures(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for val, count in [("😀", 1), ("👍🏽", 2), ("🇯🇵", 2), ("👩‍💻", 3)]:
            (self.cwd / (val + ".txt")).touch()
            self.compare(f'v={val}; key=v; re="^(.)(.*)$"; [[ $v =~ $re ]] || exit 1; '
                         'printf "<%s>" "${#v}" "${!key}" "${v:1}" "${v//?/_}" '
                         '"${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"',
                         0, f"<{count}><{val}><{val[1:]}><{'_' * count}><{val[0]}><{val[1:]}>".encode())
        self.compare('set -- ?.txt; printf "<%s>" "$#" "$1"; set -- ???.txt; printf "<%s>" "$#" "$1"',
                     0, "<1><😀.txt><1><👩‍💻.txt>".encode())
        indirect = ('k=real; unset real; printf "<%s>" "${!k:=🌱}" "$real"; '
                    'real=""; printf "<%s>" "${!k-default}" "${!k:-default}"')
        expected = "<🌱><🌱><><default>".encode()
        if int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) >= 5:
            self.compare(indirect, 0, expected)
        else:
            # Bash 3.2 assigns to the reference itself for this operator.
            proc = self.command(TONARI, indirect)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, expected, b""))
        self.compare('set -- v 👩‍💻; v=👍🏽; printf "<%s>" "${!1}" "${!#}"',
                     0, "<👍🏽><👩‍💻>".encode())
        # Both reference releases preserve captures and return 2. Bash 5.3
        # also prints a diagnostic for the invalid expression.
        source = '[[ abc =~ (a) ]]; r="("; [[ abc =~ $r ]]; echo "$?:${BASH_REMATCH[1]}"'
        for shell in [BASH, TONARI]:
            proc = self.command(shell, source)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"2:a\n"))
            if shell == TONARI:
                self.assertEqual(proc.stderr, b"")

    def test_single_star_literal_edges(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for name in ["aba", "ababa", "ab🌱ba", ".log", "雪🌱.log", "a*b"]:
            (self.cwd / name).touch()
        self.compare('for v in aba ababa ab🌱ba; do [[ $v == ab*ba ]]; echo "$?"; done; '
                     'case 雪🌱.log in 雪*.log) echo yes ;; esac; '
                     r'[[ "a*b" == a\*b ]] || exit 1; set -- ab*ba; echo "$#"; '
                     'set -- *.log; echo "$#:$1"; shopt -s dotglob; set -- *.log; echo "$#"',
                     0, "1\n0\n0\nyes\n2\n1:雪🌱.log\n2\n".encode())

    def test_case_patterns_are_expanded_only_when_reached(self):
        self.compare("""classify() {
            case "$1" in
                'a*b') echo literal ;;
                'a*'*) echo mixed ;;
                "$pattern") echo exact ;;
                $pattern) echo glob ;;
                "$(echo visited >&2; echo unreachable)") exit 9 ;;
            esac
        }
        classify 'a*b'; classify 'a*tail'
        pattern='*.rs'; classify '*.rs'; classify main.rs
        pattern='*.py'; classify main.py""", 0, b"literal\nmixed\nexact\nglob\nglob\n")

    def test_glob_reads_literal_prefixes_and_quoted_class_members(self):
        for name in ["item-000雪.txt", "item-999.txt", "item^.txt", "a", "-", "m", "z", ".hidden"]:
            (self.cwd / name).touch()
        self.compare("printf '<%s>' item-000*.txt item'^'*.txt; "
                     """printf '<%s>' [a"-"z]; shopt -s dotglob; printf '<%s>' .h*""",
                     0, "<item-000雪.txt><item^.txt><-><a><z><.hidden>".encode())
        # Tonari's UTF-8 pathname policy also applies to borrowed directory entries.
        try:
            with open(os.fsencode(self.cwd) + b"/item-000\xff.txt", "wb"):
                pass
        except OSError as error:
            if error.errno != errno.EILSEQ:
                raise
        result = self.command(TONARI, "shopt -s dotglob; printf '<%s>' item-000*.txt .*")
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, "<item-000雪.txt><.hidden>".encode(), b""))

    def test_glob_escaping_matches_bash_argv(self):
        symbols = ["*", "?", "[", "]", "!", "-", "^", ":", "=", ".", "\\", "a", "b", "z",
                   " ", "\t", "\n", "雪", "😀", "$", "'", '"', "*?", "[]", "\\*", "\\?", "x\\", "[a]", "a-b"]
        for value in symbols:
            for name in [value, f"x{value}y.txt", f"xx{value}zz.txt", f"{value}at.ext"]:
                if name != ".":
                    (self.cwd / name).touch()
        for name in ["x*y", "xxy", "x\\y", "x\\*y", "a\\b", "ab", "x]yz", "x[yz", "x-*", "x\\-q"]:
            (self.cwd / name).touch()

        cases = []
        for value in symbols:
            double = '"' + "".join("\\" + c if c in '\\"$`' else c for c in value) + '"'
            escaped = "".join("\\" + c if c != "\n" else "'\n'" for c in value)
            for word in [shlex.quote(value), double, escaped, '"$s"', '$s']:
                for left, right in [("", ""), ("x*", "*txt"), ("x", "*"), ("", "*.ext"), ("x[", "]*.txt")]:
                    cases.append((value, left + word + right))
        for value in [r"\*", r"\?", r"\[", r"\\", r"x\*y", r"x\*y*", r"*\\*", r"x\\*", r"x\?",
                      r"x\[", r"x\y", r"x\*", r"\a", r"\\*", r"\**", r"\\\*", r"\\a", r"x\\y", r"x\$\[\]*"]:
            for word in ['$s', '"$s"', '$s*', '"$s"*', 'x*"$s"*', '"$s""?"*']:
                cases.append((value, word))

        # Batch subprocesses and include the argument count so empty argv and
        # one empty argument remain distinguishable. NUL cannot occur in a filename.
        self.env.pop("LC_ALL")
        self.env["LC_COLLATE"] = "C"
        for ctype in ["C", "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"]:
            self.env["LC_CTYPE"] = ctype
            for offset in range(0, len(cases), 60):
                batch = cases[offset:offset + 60]
                source = "\n".join(f"s={shlex.quote(v)}; set -- {word}; " +
                                   r"""printf '%s\0' "$#" "$@" '<end>' """ for v, word in batch)
                records = []
                for shell in (BASH, TONARI):
                    proc = self.command(shell, source)
                    self.assertEqual((proc.returncode, proc.stderr), (0, b""))
                    rows = proc.stdout.split(b"\0<end>\0")
                    self.assertEqual(rows.pop(), b"")
                    self.assertEqual(len(rows), len(batch))
                    records.append(rows)
                for case, expected, actual in zip(batch, *records):
                    self.assertEqual(actual, expected, (ctype, case))

    def test_quoted_lists_in_case_conditionals_and_parameter_expansion(self):
        values = ["*", "?", "[", "]", "!", "^", "-", "\\", "a", "m", "z", "2", "a]", "=]", ":]", "[]"]
        sets = [
            (r'["*"]', {"*"}), (r'[!"?"]', {v for v in values if len(v) == 1 and v != "?"}),
            (r'["[]"]', {"[", "]"}), (r'["!^"]', {"!", "^"}), (r'[a"-"z]', {"a", "-", "z"}),
            (r'[\\]', {"\\"}), (r'[[:digit:]\]]', {"2", "]"}),
            (r'[[":"digit:]]', {":]", "[]"}), (r'[[="a"=]]', {"a]", "=]", "[]"}),
            (r'[[."a".]]', set()),
        ]
        version = self.command(BASH, 'printf "%s.%s" "${BASH_VERSINFO[0]}" "${BASH_VERSINFO[1]}"').stdout
        for pattern, selected in sets:
            source = ('for v in ' + " ".join(map(shlex.quote, values)) + '; do '
                      f'case "$v" in {pattern}) printf 1;; *) printf 0;; esac; '
                      f'if [[ $v == {pattern} ]]; then printf 1; else printf 0; fi; done')
            for shell in (BASH, TONARI):
                # Bash 5.3 accepts the quoted operand of a POSIX collating or
                # equivalence symbol. Tonari follows Debian's 5.2 semantics here.
                members = selected
                if shell == BASH and tuple(map(int, version.split(b"."))) >= (5, 3) and pattern in (r'[[="a"=]]', r'[[."a".]]'):
                    members = {"a"}
                proc = self.command(shell, source)
                expected = b"".join(b"11" if v in members else b"00" for v in values)
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, expected, b""), (shell, pattern))
        self.compare(r"""p='[\*\]]'; v='*a]'; printf '%s\0' "${v#$p}" "${v%$p}" "${v//$p/X}" """,
                     0, b"a]\0*a\0XaX\0")

    def test_nullglob_with_unclosed_and_escaped_brackets(self):
        for name in ["x[a", "x[]b", ".dot"]:
            (self.cwd / name).touch()
        self.compare(r"""shopt -s nullglob
set -- missing[ missing[x missing\[ missing[] x[* x[]* ["."]dot
printf '%s\0' "$#" "$@" """, 0,
                     b"6\0missing[\0missing[x\0missing[\0x[]b\0x[a\0x[]b\0")

        # Modern Bash recognizes a bracket pair only within one path component.
        script = r"""shopt -s nullglob; set -- missing[x/y]; printf '%s\0' "$#" "$@" """
        p = self.command(TONARI, script)
        self.assertEqual((p.returncode, p.stderr, p.stdout), (0, b"", b"1\0missing[x/y]\0"))
        if int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) >= 5:
            self.compare(script, 0, p.stdout)

    def test_glob_collation_uses_the_selected_locale(self):
        for name in ["A1", "a9", "B2", "b8", "z", "éé", "雪"]:
            (self.cwd / name).touch()
        utf8 = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        source = 'printf "<%s>" *'
        for all_, collate, lang in [("C", utf8, utf8), ("", "POSIX", utf8),
                                    ("", "", "C"), (utf8, "C", "C")]:
            with self.subTest(all_=all_, collate=collate, lang=lang):
                self.env.update(LC_ALL=all_, LC_COLLATE=collate, LANG=lang)
                expected = self.command(BASH, source).stdout
                self.compare(source, 0, expected)

    def test_glob_sort_order_after_directory_changes(self):
        for i in range(96):
            (self.cwd / f"{['A', 'é', '雪', '🌱'][i % 4]}-{i:03}.txt").touch()
        (self.cwd / "a b\nc.txt").touch()
        commands = r"""printf '%s\0' *.txt; printf '%s\0' *.txt
mv A-000.txt Z-000.txt
printf '%s\0' *.txt
mv Z-000.txt A-000.txt
: > another.txt
printf '%s\0' *.txt
rm another.txt
printf '%s\0' *.txt"""
        for locale in ["C", "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"]:
            self.env["LC_ALL"] = locale
            expected = self.command(BASH, commands)
            actual = self.command(TONARI, commands)
            self.assertEqual((expected.returncode, expected.stderr), (0, b""))
            self.assertEqual((actual.stdout, actual.stderr, actual.returncode), (expected.stdout, b"", 0))

    def test_glob_multiple_directory_batches_and_symlink_types(self):
        group = self.cwd / "group"
        group.mkdir()
        stem = "x" * 165
        for i in range(260):
            (group / f"{stem}-{i:03}.txt").touch()
        (group / "sub").mkdir()
        (group / "sub" / "file.rs").touch()
        (group / "alias").symlink_to("sub")
        (group / "broken").symlink_to("gone")
        os.mkfifo(group / "fifo")
        self.compare('set -- group/*x*2*.txt; printf "%s\n" "$#"; '
                     'printf "<%s>" group/*/*.rs group/*/', 0,
                     b"98\n<group/alias/file.rs><group/sub/file.rs><group/alias/><group/sub/>")

    def test_glob_options_and_recursive_symlinks(self):
        for path in ["a/top.txt", "a/.hidden/inner.txt", "a/sub/leaf.txt", ".root.txt", "name.txt"]:
            dest = self.cwd / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.touch()
        (self.cwd / "alias").symlink_to("a")
        (self.cwd / "a/sub/back").symlink_to("..")
        self.compare('shopt -s nullglob; set -- missing*.txt "missing*.txt"; echo "$#:$1"; '
                     'shopt -s dotglob; printf "<%s>" *.txt; set -f; printf "<%s>" *.txt',
                     0, b"1:missing*.txt\n<.root.txt><name.txt><*.txt>")
        script = ('shopt -s globstar; printf "<%s>" **/**/leaf.txt alias/**/*.txt; '
                  'shopt -s dotglob; printf "<%s>" **/inner.txt; '
                  'shopt -u globstar; printf "<%s>" **/top.txt')
        expected = b"<a/sub/leaf.txt><alias/sub/back/top.txt><alias/sub/leaf.txt><alias/top.txt><a/.hidden/inner.txt><a/top.txt><alias/top.txt>"
        if self.command(BASH, 'shopt -s globstar').returncode == 0:
            self.compare(script, 0, expected)
        else:
            actual = self.command(TONARI, script)
            self.assertEqual((actual.returncode, actual.stdout, actual.stderr), (0, expected, b""))

    def test_substitution_printf_isolation(self):
        self.compare("""v=parent; old=$PWD
x=$(v=child; cd /; printf '%s' "$v"; exit 23); status=$?
[[ $old == "$PWD" ]] && printf '%s/%s/%s' "$x" "$v" "$status" """,
                     0, b"child/parent/23")
        self.compare(r'''n=parent; x=$(printf 'a%n%b\n' n 'b\n'); s=$?
printf '<%s>' "$x" "$n" "$s"
y=$(printf -v n inner); printf '<%s>' "$y" "$n"
printf() { echo overridden; }; z=$(printf '%s' hi); echo "$z"''',
                     0, b"<ab><parent><0><><parent>overridden\n")
        if self.command(BASH, '[[ -n $BASHPID ]]').returncode == 0:
            self.compare('x=$(printf "%s" "$BASHPID"); [[ -n $BASHPID ]] && [[ $x != "$BASHPID" ]]', 0)

    def test_external_capture_and_environment(self):
        self.env["INHERITED"] = "old"
        self.compare("""export V=outer; unset INHERITED
/bin/sh -c 'echo "$V:${INHERITED-absent}"'
V=temp /bin/sh -c 'echo "$V"'
f() { local V=inside; /bin/sh -c 'echo "$V"'; }; f
export V=changed; /bin/sh -c 'echo "$V"'
unset V; /bin/sh -c 'echo "${V-unset}"'""", 0, b"outer:absent\ntemp\ninside\nchanged\nunset\n")
        source = """x=$(/bin/sh -c 'cat; printf err >&2; exit 23'); s=$?
printf '%s:%s' "$s" "$x" """
        data = b"output" * 22000
        for shell in [BASH, TONARI]:
            with self.subTest(shell=shell):
                proc = subprocess.run([shell, "-c", source], input=data + b"\n\n",
                                      cwd=self.cwd, env=self.env, capture_output=True, timeout=5)
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"23:" + data, b"err"))

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

    def test_alias_and_nested_function_status(self):
        self.compare("""shopt -s expand_aliases
alias say='printf "%s"'
function outer { local x=inner; inner() { say "$x:$1"; return 17; }; inner "$1"; }
unalias say
set -- before; x=outer; outer "two words"; printf ':%s:%s:%s' "$?" "$1" "$x" """,
                     0, b"inner:two words:17:before:outer")
        self.compare("shopt -s expand_aliases\nalias a='b'; alias b='printf ok'\na", 0, b"ok")

    def test_exit_and_signal_traps(self):
        self.compare("""trap 'printf "exit:%s" "$?"' EXIT; /bin/sh -c 'exit 17'""", 17, b"exit:17")
        self.compare("trap 'exit 42' EXIT; exit 17", 42)
        self.compare("trap 'false; exit' EXIT; exit 17", 17)
        for shell in (BASH, TONARI):
            proc = self.command(shell, "trap 'printf cleanup' EXIT; kill -TERM $$; echo unreachable")
            self.assertEqual((proc.returncode, proc.stdout), (-signal.SIGTERM, b"cleanup"))
        self.compare("""trap 'printf "outer:%s" "$?"' EXIT
(trap 'printf "inner:%s/" "$?"' EXIT; exit 7); exit 2""", 2, b"inner:7/outer:2")
        self.compare("""f() { printf 'trap:%s:%s/' "$?" "$x"; seen=yes; }
trap f USR1; trap '' USR2; kill -USR2 "$$"
g() { local x=local; kill -USR1 "$$"; }; g
trap - USR1; printf '%s' "$seen" """, 0, b"trap:0:local/yes")
        self.compare("""trap '' PIPE; /bin/sh -c 'kill -PIPE $$; printf ok'; trap - PIPE""", 0, b"ok")
        self.compare("""trap 'printf done' EXIT; exec /bin/sh -c 'printf replaced; exit 9'""", 9, b"replaced")
        for shell in (BASH, TONARI):
            proc = self.command(shell, "trap : USR1; trap - USR1; kill -USR1 $$; echo unreachable")
            self.assertEqual((proc.returncode, proc.stdout), (-signal.SIGUSR1, b""))

    def test_interrupted_wait_keeps_child_status(self):
        source = """trap 'printf "trap:%s/" "$?"' USR1
p=$$; (sleep 0.05; kill -USR1 "$p"; sleep 0.05; exit 23) & child=$!
wait "$child"; printf "wait:%s/" "$?"
wait "$child"; printf "done:%s" "$?" """
        code = 128 + signal.SIGUSR1
        self.compare(source, 0, f"trap:{code}/wait:{code}/done:23".encode())

    def test_pipeline_and_background_status(self):
        self.compare("""set -o pipefail
/bin/sh -c 'exit 7' | /bin/sh -c 'exit 19' | true; printf '<%s>' "$?" "${PIPESTATUS[@]}"
! false | true; printf '<%s>' "$?" "${PIPESTATUS[@]}"
/bin/sh -c 'exit 4' | true & p=$!; printf '<%s>' "$?"; wait "$p"; printf '<%s>' "$?" """,
                     0, b"<19><7><19><0><0><1><0><0><4>")
        # Bash 5.x carries the enclosing pipe into this background command.
        source = 'printf foo | { cat & wait; printf ok; }'
        self.assertEqual(self.command(TONARI, source).stdout, b"foook")
        if int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) >= 5:
            self.compare(source, 0, b"foook")
        self.compare('sleep 3 & p=$!; kill -TERM %1; wait "$p" 2>/dev/null; printf %s "$?"', 0, b"143")
        self.compare('(exit 23) & p=$!; sleep 0.02; jobs >/dev/null; wait "$p"; printf %s "$?"',
                     0, b"23")
        # Bash 3.2 consumes the saved status on a second wait by PID.
        if int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) >= 5:
            self.compare('(exit 23) & p=$!; wait "$p"; wait "$p"; printf %s "$?"', 0, b"23")
        for shell in (BASH, TONARI):
            proc = self.command(shell, """/bin/sh -c 'kill -TERM $$'; printf %s "$?" """)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"143"))

    def test_braces_keep_quoting_order_and_side_effects(self):
        self.compare(r"""printf '[%s]\n' pre{a,{b,c}}{1,2} {,} end '{x,y}' z\{a,b\}
printf '<%s>\n' {03..-1..2} {d..a..2} {1.."3"} {word}
i=0; printf '(%s)' {x,y}$((++i)); echo ":$i"
x='{a,b}'; printf '%s/' "$x" $x
set +B; echo {q,r}; set -o braceexpand; echo {q,r}""", 0,
                     b"[prea1]\n[prea2]\n[preb1]\n[preb2]\n[prec1]\n[prec2]\n[end]\n"
                     b"[{x,y}]\n[z{a,b}]\n<03>\n<01>\n<-1>\n<d>\n<b>\n<{1..3}>\n<{word}>\n"
                     b"(x1)(y2):2\n{a,b}/{a,b}/{q,r}\nq r\n", minimum=4)
        self.compare("""touch a.rs b.rs c.txt; printf '[%s]' {a*,*.txt}; echo
a=pre{x,y}; echo "$a"; arr=(foo{1,2} {04..06}); printf '%s/' "${arr[@]}" """,
                     0, b"[a.rs][c.txt]\npre{x,y}\nfoo1/foo2/04/05/06/", minimum=4)

    def test_case_fallthrough_and_retest(self):
        version = self.command(BASH, 'echo "${BASH_VERSINFO[0]}"')
        if int(version.stdout) < 4:
            self.skipTest("case fallthrough requires Bash 4")
        self.compare("""case x in
x) printf first ;& "$(echo bad >&2)") printf ':next'; false ;;&
y) printf wrong ;; x) printf ':retest:%s' "$?";; esac
case z in z) false ;;& a) :;; esac; printf '/%s' "$?" """,
                     0, b"first:next:retest:1/1")

    def test_clobber_readwrite_and_descriptor_moves(self):
        self.compare("""echo first >file; set -C
echo lost 2>/dev/null >file; echo "blocked:$?"; cat file
echo allowed >|file; cat file; : >/dev/null; echo "device:$?"
exec 3<>file; IFS= read -ru3 line; echo "read:$line"; printf 'more' >&3; exec 3>&-
exec 6<file; exec 7<&6-; read -ru7 line; printf 'moved:%s' "$line"
exec 8>|output; printf out 9>&8- >&9; { printf ok >&8; } 2>/dev/null; echo "/$?"; cat output""",
                     0, b"blocked:1\nfirst\nallowed\ndevice:0\nread:allowed\nmoved:allowed/1\nout")

    def test_integer_declarations_and_local_scope(self):
        self.compare("""base=5; declare -i n='base*2'; n+=3; declare -p n
export n+=2; n='n*2'; echo "$n"
f() { local -i n=7; let n++; echo "$n"; }; f; echo "$n"
printf -v n '%s' 'base+4'; echo "$n"; read -r n <<< '9+1'; echo "$n"
declare +i n; n+=a; echo "$n"; declare -ai a=('3+1' '5*2'); a[0]+=2; declare -p a""",
                     0, b'declare -i n="13"\n30\n8\n30\n9\n10\n10a\ndeclare -ai a=([0]="6" [1]="10")\n', minimum=4)

    def test_mapfile_and_readarray(self):
        version = self.command(BASH, 'echo "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"')
        if tuple(map(int, version.stdout.split(b"."))) < (4, 4):
            self.skipTest("mapfile -d requires Bash 4.4")
        self.compare("""arr=(old keep)
cb() { printf 'cb:%s:<%s>:%s/' "$1" "$2" "${#arr[@]}"; }
readarray -ts1 -O1 -n2 -C cb -c2 arr < <(printf 'skip\nfirst\na b\nunused')
printf '[%s]' "${arr[@]}"; echo
printf 'aa\\0x y\\0tail' > records; exec 3<records
mapfile -u3 -d '' -n2; printf '<%s>' "${MAPFILE[@]}"
mapfile -u 3 -t; printf '<%s>' "${MAPFILE[@]}" """,
                     0, b"cb:2:<a b>:2/[old][first][a b]\n<aa><x y><tail>")
        self.compare("""exec 3< <(printf 'first\nsecond\nlast'); mapfile -u3 -tn1 a
IFS= read -ru3 line; echo "${a[0]}:$line"; mapfile -tu3 a; echo "${a[0]}"
printf 'a\\0b\nc\n' > input; mapfile -t a <input; printf '[%s]' "${a[@]}" """,
                     0, b"first:second\nlast\n[a][c]")

    def test_mapfile_chunk_boundaries_and_sparse_edits(self):
        self.env["LC_ALL"] = "en_US.UTF-8"
        (self.cwd / "chunked").write_bytes(b"a" * 8191 + b"\n" + b"b" * 8192 + "雪\n".encode()
                                           + b"\nlast\0discarded")
        if not self.command(BASH, 'type -t mapfile').stdout:
            shells = (TONARI,)
        else:
            shells = (BASH, TONARI)
        script = """mapfile -t v <chunked; printf '%s/' "${#v[@]}" "${#v[0]}" "${#v[1]}" "${v[2]}" "${v[3]}"
v[1]=changed; unset 'v[0]'; v[20]=tail; printf '[%s]' "${!v[@]}" "${v[-1]}" """
        for shell in shells:
            with self.subTest(shell=shell):
                proc = self.command(shell, script)
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                 (0, b"4/8191/8193//last/[1][2][3][20][tail]", b""))

    def test_umask_and_job_filters(self):
        self.compare("""umask 022; (umask 077; umask); umask -p
umask u=rwx,g=r,o=; umask -pS; : > formed; ls -l formed | cut -c 1-10""",
                     0, b"0077\numask 0022\numask -S u=rwx,g=r,o=\n-rw-r-----\n")
        self.compare("""sleep 0.12 & p=$!; jobs -s; jobs -r %1 >listing; awk '{$1=$1; print}' listing
test "$(jobs -p %1)" = "$p" && echo selected
jobs -l | grep -q "$p" && echo long; jobs -n; wait "$p" """,
                     0, b"[1]+ Running sleep 0.12 &\nselected\nlong\n")

    def test_command_substitution_capture_boundaries(self):
        self.compare("""x=outside; v=$(printf 'a'; echo -ne 'b\\nc'; printf ':'); echo "$v"
v=$(printf '%s' "$x"; false); echo "$?:$v"
v=$(printf -v x changed; echo "$x"); echo "$x:$v"
echo() { printf 'function:%s' "$1"; }; v=$(echo shadow); printf '%s' "$v" """,
                     0, b"ab\nc:\n1:outside\noutside:changed\nfunction:shadow")
        self.compare("""n=8; v=$(printf 'xy%n' n; printf '%s' "$n"); echo "$v:$n"
v=$(false; printf '%s' "$?"); echo "$v" """, 0, b"xy2:8\n1\n")

    def test_parent_job_visibility_without_job_ownership(self):
        self.compare("""sleep 0.15 & p=$!
f() { jobs -p; }
test "$(jobs -p)" = "$p" && echo direct
test "$(true; f)" = "$p" && echo nested
test "$(jobs -p | cat)" = "$p" && echo pipe
(jobs -p); wait "$p"; echo "$?" """, 0, b"direct\nnested\npipe\n0\n")

    def test_wait_next_selected_ids_and_signals(self):
        version = self.command(BASH, 'echo "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"')
        if tuple(map(int, version.stdout.split(b"."))) < (5, 1):
            self.skipTest("wait -p requires Bash 5.1")
        self.compare("""sleep 0.18 & slow=$!
(sleep 0.04; exit 13) & fast=$!
wait -np id "$fast"; rc=$?; test "$fast" = "$id" && echo "first:$rc"
wait "$fast"; echo "again:$?"
wait -n -p id "$slow"; rc=$?; test "$slow" = "$id" && echo "last:$rc"
wait -np id; echo "empty:$?:${id-unset}" """,
                     0, b"first:13\nagain:13\nlast:0\nempty:127:unset\n")
        self.compare("""trap 'echo signal' USR1; me=$$
(sleep 0.05; kill -USR1 "$me") & sender=$!
sleep 0.18 & slow=$!; result=old; wait -np result "$slow"; rc=$?
test "$rc" -gt 128 && echo "interrupted:${result-unset}"
wait "$slow"; echo "$?" """, 0, b"signal\ninterrupted:unset\n0\n")

    def test_getopts_clusters_scope_and_reset(self):
        self.compare("""set -- -abvalue -c -- tail; OPTIND=1
while getopts ':ab:c' op; do echo "$op:${OPTARG--}:$OPTIND"; done
echo "$op:${OPTARG--}:$OPTIND"
f() { local OPTIND=1; getopts xy o -xy; echo "$o:$OPTIND"; }
OPTIND=1; getopts xy o -xy; f; getopts xy o -xy; echo "$o:$OPTIND"
OPTIND=1; getopts ':b:' op -b; echo "$op:$OPTARG"
OPTIND=1; getopts ':a' op -z; echo "$op:$OPTARG" """,
                     0, b"a:-:1\nb:value:2\nc:-:3\n?:-:4\nx:1\ny:2\n::b\n?:z\n", minimum=5)

    def test_let_and_completion_definition(self):
        self.compare("""let 'x=2' 'x*=3'; echo "$?:$x"; let 'x-=6'; echo "$?:$x" """,
                     0, b"0:6\n1:0\n")
        self.compare("""complete -v -d -A directory -W one -P pre -o nospace name
complete -p name""", 0, b"complete -o nospace -d -v -W 'one' -P 'pre' name\n")

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

    def terminal_session(self, editing=False, shell=TONARI):
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.chdir(self.cwd)
                env = dict(self.env, PS1=PROMPT.decode(), TERM="xterm-256color" if editing else "dumb")
                options = [] if editing else ["--noediting"]
                os.execve(shell, [str(shell), "--norc", *options], env)
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
            os.close(fd)
            try:
                os.waitpid(pid, 0)
            except ChildProcessError:
                pass
        self.addCleanup(close)
        self.read_until(fd, PROMPT)
        return fd

    def test_silent_terminal_read_restores_echo(self):
        for shell in self.shells_for(4):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell)
                os.write(fd, b"read -rs -p 'Value? ' x; echo length=${#x}\n")
                # Wait for the real prompt after the echoed command line.
                self.read_until(fd, b"\r\nValue? ")
                os.write(fd, b"example\n")
                output = self.read_until(fd, PROMPT)
                self.assertIn(b"length=7", output)
                self.assertNotIn(b"example", output)
                os.write(fd, b"echo restored\n")
                self.assertIn(b"echo restored", self.read_until(fd, PROMPT))

    def test_interactive_alias_and_interrupt_trap(self):
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                legacy = shell == BASH and int(self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout) < 4
                self.check_interactive_alias_and_interrupt_trap(shell, legacy)

    def check_interactive_alias_and_interrupt_trap(self, shell, legacy=False):
        fd = self.terminal_session(shell=shell)
        os.write(fd, b"alias marker='printf A%sZ lias'\n")
        self.read_until(fd, PROMPT)
        os.write(fd, b"marker\n")
        self.assertIn(b"AliasZ", self.read_until(fd, PROMPT))
        os.write(fd, b"trap 'printf TRAP=%s! \"$?\"' INT; false\n")
        self.read_until(fd, PROMPT)
        os.write(fd, b"\x03")
        self.read_until(fd, b"TRAP=129!" if legacy else b"TRAP=1!")

    def test_stop_and_resume_foreground_pipeline(self):
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                self.check_foreground_pipeline(self.terminal_session(shell=shell))

    def check_foreground_pipeline(self, fd):
        os.write(fd, b"/bin/sh -c 'printf R%sY EAD; exec sleep 20' | cat\n")
        self.read_until(fd, b"READY")
        os.write(fd, b"\x1a")
        self.assertIn(b"Stopped", self.read_until(fd, PROMPT))
        os.write(fd, b"printf 'STOP=%s\\n' $?\n")
        self.assertIn(f"STOP={128 + signal.SIGTSTP}\r\n".encode(), self.read_until(fd, PROMPT))
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

    def test_native_completion_insertion_listing_and_typeahead(self):
        self.env["PATH"] = f"{self.cwd}:{self.cwd}"
        (self.cwd / "safe path").mkdir()
        (self.cwd / "x-link").symlink_to("safe path")
        (self.cwd / "item-as").touch()
        (self.cwd / "item-at").touch()
        (self.cwd / ".item-hidden").touch()
        (self.cwd / "tool-allowed").symlink_to("/bin/sh")
        (self.cwd / "tool-denied").touch()
        fd = self.terminal_session(editing=True)

        def send(keys, marker):
            os.write(fd, keys)
            return self.read_until(fd, marker)

        clear = b"\x15"
        send(b": it\tX", PROMPT + b": item-aX")
        send(clear + b": it\t", PROMPT + b": item-a")
        (self.cwd / "item-au").touch()
        send(b"\t", b"item-as  item-at  item-au")
        send(clear + b": x-l\tmore", PROMPT + b": x-link/more")
        send(clear + b"tool-\targ", PROMPT + b"tool-allowed arg")
        send(clear + b"complete -d -o nospace -o default demo\r", b"\x1b[?2004h")
        send(b"demo safe\tmore", PROMPT + b"demo safe\\ path/more")
        send(clear + b"demo item-at\targ", PROMPT + b"demo item-at arg")
        send(clear + b"complete -W 'start stop start' demo\r", b"\x1b[?2004h")
        send(b"demo s\t", PROMPT + b"demo st")
        send(b"\t", b"start  stop")

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
