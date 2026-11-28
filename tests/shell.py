"""End-to-end tests for Tonari's -c and terminal entry points."""

import array
import errno
import fcntl
import functools
import os
import pty
import resource
import random
import select
import shlex
import signal
import subprocess
import sys
import tempfile
import termios
import threading
import time
import unittest
from pathlib import Path


TONARI = Path(sys.argv.pop(1)).resolve()
BASH = Path(os.environ.get("REFERENCE_BASH", "/bin/bash"))
PROMPT = b"tonari$ "


@functools.cache
def bash_version():
    result = subprocess.check_output(
        [BASH, "--noprofile", "--norc", "-c", 'echo "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"'],
        env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
    )
    return tuple(map(int, result.split(b".")))


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

    def test_whole_audit_function_import_modes(self):
        functions = {"BASH_FUNC_good%%": "() { echo ok; }", "BASH_FUNC_a-b%%": "() { echo dash; }"}
        source = 'type -t good; type -t a-b; shopt -qo posix; echo "mode=$? env=${POSIXLY_CORRECT-unset}"'
        cases = [
            ([], {}, False, b"function\nfunction\nmode=1 env=unset\n", False),
            (["--posix"], {}, False, b"function\nmode=0 env=y\n", True),
            ([], {"POSIXLY_CORRECT": ""}, False, b"function\nmode=0 env=\n", True),
            ([], {"POSIX_PEDANTIC": ""}, False, b"function\nmode=0 env=y\n", True),
            (["+o", "posix"], {"POSIXLY_CORRECT": "user"}, False, b"function\nfunction\nmode=0 env=user\n", False),
            ([], {"SHELLOPTS": "posix"}, False, b"function\nfunction\nmode=0 env=y\n", False),
            ([], {}, True, b"function\nfunction\nmode=0 env=y\n", False),
            (["-p"], {}, False, b"mode=1 env=unset\n", False),
            (["--posix", "-n"], {}, False, b"", False),
        ]
        for shell in self.shells_for(5):
            for flags, options, as_sh, expected, rejected in cases:
                for entries in (functions | options, options | functions):
                    with self.subTest(shell=shell, flags=flags, keys=list(entries), as_sh=as_sh):
                        result = subprocess.run(["sh" if as_sh else shell, *flags, "-c", source],
                                                executable=shell, env=self.env | entries, cwd=self.cwd,
                                                capture_output=True, timeout=5)
                        self.assertEqual((result.returncode, result.stdout), (0, expected))
                        if rejected:
                            self.assertIn(b"error importing function definition for `a-b'", result.stderr)
                        else:
                            self.assertEqual(result.stderr, b"")

    def test_whole_audit_shared_stdin_command_boundary(self):
        for shell in self.shells_for(5):
            for padding in (8, 800, 3600):
                for consume, expected, suffix in [
                    ("IFS= read -r line", b"ok\\data/next\n", 'ok\\data\nprintf "%s/next\\n" "$line"\n'),
                    ("/bin/cat", b"external data\n", "external data\n"),
                ]:
                    source = self.cwd / "input.sh"
                    source.write_text("if true; then\n" + "if true; then :; fi\n" * padding
                                      + consume + "\nfi\n" + suffix)
                    with self.subTest(shell=shell, padding=padding, consume=consume), source.open("rb") as script:
                        result = subprocess.run([shell], stdin=script, env=self.env, cwd=self.cwd,
                                                capture_output=True, timeout=5)
                        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, expected, b""))

    def test_second_review_job_traps_and_exec(self):
        source = """f(){ trap 'echo ERR' ERR; false; }; f; echo first:$?; false; echo last:$?
trap - ERR; trap : USR1 CONT CHLD; trap '' KILL STOP; trap"""
        signals = sorted((signal.SIGUSR1, signal.SIGCONT, signal.SIGCHLD, signal.SIGKILL, signal.SIGSTOP))
        listed = b"".join(f"trap -- '{'' if sig in (signal.SIGKILL, signal.SIGSTOP) else ':'}' {sig.name}\n".encode()
                          for sig in signals)
        for shell in self.shells_for(5):
            result = self.command(shell, source)
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, b"ERR\nfirst:1\nERR\nlast:1\n" + listed, b""))
            result = self.command(shell, """trap '' USR1; echo $$; /bin/sh -c 'echo $$'""")
            first, final = result.stdout.splitlines()
            self.assertEqual(first, final)
            result = self.command(shell, """set -x; select k in q; do break; done </dev/null; :""")
            self.assertEqual(result.stderr.count(b"+ select k in q\n"), 1)
            result = self.command(shell, """trap '' CHLD; (exit 7) & p=$!
"$1" -c 'import signal; print(signal.getsignal(signal.SIGCHLD) == signal.SIG_IGN)'
wait "$p"; echo parent:$?""", "sh", sys.executable)
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, b"True\nparent:7\n", b""))

    def test_second_review_background_shell_foreground_handoff(self):
        for shell in self.shells_for(5):
            # A separate session owns this PTY. Its controller only addresses
            # the child shell it forks and reaps here.
            controller, master = pty.fork()
            if controller == 0:
                child = os.fork()
                if child == 0:
                    os.setpgid(0, 0)
                    source = "case $- in *m*) echo monitor-on;; esac"
                    os.execve(shell, [shell, "--noprofile", "--norc", "-i", "-c", source], self.env)
                try:
                    for _ in range(200):
                        got, status = os.waitpid(child, os.WUNTRACED | os.WNOHANG)
                        if got:
                            break
                        time.sleep(.01)
                    if not got or not os.WIFSTOPPED(status) or os.WSTOPSIG(status) != signal.SIGTTIN:
                        if got and not os.WIFSTOPPED(status):
                            os._exit(2)
                        raise RuntimeError("shell did not stop for the controlling terminal")
                    os.tcsetpgrp(0, child)
                    os.kill(child, signal.SIGCONT)
                    for _ in range(200):
                        got, status = os.waitpid(child, os.WNOHANG)
                        if got:
                            os._exit(os.waitstatus_to_exitcode(status))
                        time.sleep(.01)
                finally:
                    try:
                        os.kill(child, signal.SIGKILL)
                        os.waitpid(child, 0)
                    except (ProcessLookupError, ChildProcessError):
                        pass
                    os._exit(3)
            try:
                output = self.read_until(master, b"monitor-on")
                self.assertIn(b"monitor-on", output)
            finally:
                _, status = os.waitpid(controller, 0)
                os.close(master)
            self.assertEqual(os.waitstatus_to_exitcode(status), 0)

    def test_finite_nameref_chain_has_a_depth_diagnostic(self):
        finite = "; ".join(f"declare -n ref{i}=ref{i+1}" for i in range(40))
        cases = [(finite, "ref0", b"name reference depth limit exceeded"),
                 ("declare -n first=second second=first", "first", b"circular name reference")]
        for setup, name, diagnostic in cases:
            with self.subTest(name=name):
                p = self.command(TONARI, setup + '; echo "<$' + name + '>"; echo done')
                self.assertEqual((p.returncode, p.stdout), (0, b"<>\ndone\n"))
                self.assertIn(diagnostic, p.stderr)

    def test_local_nameref_self_reference_preserves_other_scopes(self):
        source = """r=global
inner(){ local -n r=r || return; r=changed; }
outer(){ local r=outer; inner; echo "$?/$r"; }
outer; echo "$r" """
        r = self.command(TONARI, source)
        self.assertEqual((r.returncode, r.stdout), (0, b"1/outer\nglobal\n"))
        self.assertIn(b"nameref variable self references not allowed", r.stderr)
        if bash_version() >= (5, 2):
            r = self.command(BASH, source)
            self.assertEqual((r.returncode, r.stdout), (0, b"0/outer\nchanged\n"))
            self.assertIn(b"circular name reference", r.stderr)

    def test_second_review_job_statuses_and_notices(self):
        # A stopped job stays in both shells' job tables until it is resumed.
        for shell in self.shells_for(5):
            result = self.command(shell, """set -m
(exit 3) | /bin/sh -c 'read -r line; kill -STOP $$; exit 5' & p=$!
wait "$p"; jobs -l; bg %+ >/dev/null; wait "$p"; echo finished:$?""")
            rows = result.stdout.splitlines()
            self.assertEqual((result.returncode, len(rows)), (0, 3), (shell, result))
            self.assertRegex(rows[0], rb"^\[1\]\+ +\d+ Exit 3 +")
            self.assertRegex(rows[1], rb"^ +\d+ (Stopped|Suspended).*\| ")
            self.assertEqual(rows[2], b"finished:5")
            for wait in ("wait", "wait -n"):
                result = self.command(shell, "/bin/sh -c 'kill -HUP $$' & " + wait + "; echo after >&2")
                self.assertEqual(result.returncode, 0)
                self.assertEqual(len(result.stderr.splitlines()), 2, result.stderr)
                self.assertIn(b"Hangup", result.stderr.splitlines()[0])
                self.assertEqual(result.stderr.splitlines()[1], b"after")
            result = self.command(shell, "/bin/sh -c 'kill -TERM $$'; echo after >&2")
            self.assertEqual(result.stderr.splitlines()[-1], b"after")
            self.assertIn(b"Terminated", result.stderr.splitlines()[0])
            result = self.command(shell, "set -o posix; wait 2147483647; kill -0 2147483647")
            self.assertEqual(result.returncode, 1)
            self.assertIn(b"pid 2147483647 is not a child", result.stderr)
            self.assertIn(b"No such process", result.stderr)
            self.assertNotIn(b"os error", result.stderr)

        # Bash may already have reported and removed a completed job. Tonari
        # retains the completed pipeline for this first explicit listing.
        result = self.command(TONARI, """set -m
(exit 3) | (read -r line; exit 5) & p=$!
while kill -0 "$p" 2>/dev/null; do :; done
jobs -l; wait""")
        rows = result.stdout.splitlines()
        self.assertEqual((result.returncode, len(rows)), (0, 2), result)
        self.assertRegex(rows[0], rb"^\[1\]\+ +\d+ Exit 3 +")
        self.assertRegex(rows[1], rb"^ +\d+ Exit 5 +\| ")

    def test_second_review_pipe_cleanup_and_select_eof(self):
        result = self.command(TONARI, "select item in a; do break; done </dev/null >&-")
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"select: write error", result.stderr)
        for shell in self.shells_for(5):
            for producer, status in ((":; echo payload", 0), ("false; echo payload", 1),
                                     ("f(){ false; echo payload; }; f", 1)):
                reader, writer = os.pipe()
                os.close(reader)
                with os.fdopen(writer, "wb") as output:
                    result = subprocess.run([shell, "-c", "trap 'printf \"cleanup:%s\\n\" \"$?\" >&2' EXIT; "
                                             + producer + "; echo extra >&2"], cwd=self.cwd,
                                            env=self.env, stdout=output, stderr=subprocess.PIPE, timeout=5)
                self.assertEqual(result.returncode, -signal.SIGPIPE)
                self.assertIn(f"cleanup:{status}\n".encode(), result.stderr)
                self.assertNotIn(b"write error", result.stderr)
                self.assertNotIn(b"extra", result.stderr)

    def test_second_review_stored_subscripts_and_quoting(self):
        from review_r2_language import verify

        env = dict(self.env, LC_ALL="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8")
        programs = {"tonari": TONARI}
        version = subprocess.check_output([BASH, "-c", 'printf "%s.%s.%s" "${BASH_VERSINFO[@]:0:3}"'],
                                          text=True, env=env)
        if int(version.split(".")[0]) >= 5:
            programs["bash"] = BASH
        verify(programs, env, version)

    def test_second_review_option_and_variable_state(self):
        cases = [
            ("a=(one two); more=yes; unset 'a[-5]' more; echo \"$?/${a[*]}/${more-unset}\"",
             b"1/one two/unset\n"),
            ("s=ok; unset 's[-1]'; echo \"$?/$s\"; unset 's[0]'; echo \"$?/${s-unset}\"",
             b"1/ok\n0/unset\n"),
            ("export -n missing; declare -p missing 2>/dev/null; echo $?; "
             "export -n PATH; command -v cat >/dev/null; echo \"$?/${PATH+present}\"",
             b"1\n0/present\n"),
            ("set -eo pipefail -Z; printf '%s/' \"$?\"; shopt -qo pipefail; echo \"$?\"; "
             "set -u -o nonoption; echo \"$?/${absent-fallback}\"; shopt -qo nounset; echo $?",
             b"2/1\n2/fallback\n0\n"),
        ]
        for source, expected in cases:
            for shell in (TONARI, BASH):
                result = self.command(shell, source)
                self.assertEqual((result.returncode, result.stdout), (0, expected), result.stderr)

    def test_second_review_integer_whitespace(self):
        source = r"""set -- word
for whitespace in $'\v' $'\r' $'\f' $'\n'; do
  test "${whitespace}7${whitespace}" -eq 7 && test -v "${whitespace}1${whitespace}" || exit 9
  read -n "2$whitespace" v <<<abcd; printf '<%s>' "$v"
done
test '7 ' -eq 7 2>/dev/null; echo "/$?"
"""
        shells = (TONARI, BASH) if bash_version() >= (5, 3) else (TONARI,)
        for shell in shells:
            result = self.command(shell, source)
            self.assertEqual((result.returncode, result.stdout), (0, b"<ab>" * 4 + b"/2\n"), result.stderr)

    def test_null_redirect_status_uses_the_redirect_environment(self):
        source = """set -e
{fd}>"$(printf /dev/null; exit 5)"
v=$(exit 8) <"$(printf /dev/null; exit 5)"
echo "reached:${fd-unset}:${v+x}"
set +e
>"$(printf /dev/null; exit 5)"; echo output:$?
v=$(exit 8) 3>/dev/null; echo saved:$?
fd=1; 2>&"$fd" 3>"$(printf /dev/null; exit 6)"; echo ordered:$?
{fd}<missing-file; echo failed:$?"""
        for shell in self.shells_for(5):
            result = self.command(shell, source)
            self.assertEqual((result.returncode, result.stdout),
                             (0, b"reached:unset:x\noutput:5\nsaved:8\nordered:6\nfailed:1\n"), result.stderr)

    def test_deferred_descriptor_external_updates_parent_hash(self):
        source = """hash -r; fd=1; uname >/dev/null 2>&$fd; hash -t uname
hash -r; fd=no; uname >/dev/null 2>&$fd; hash -t uname
hash -p /bin/false uname; fd=1; command -p uname >/dev/null 2>&$fd; hash -t uname
PATH=/usr/bin:/bin uname >/dev/null 2>&$fd; hash -t uname 2>/dev/null; echo prefix:$?
uname(){ :; }; uname >/dev/null 2>&$fd; hash -t uname 2>/dev/null; echo function:$?"""
        result = self.command(TONARI, source)
        expected = self.command(BASH, source)
        if bash_version() >= (5, 3):
            self.assertEqual((result.returncode, result.stdout), (expected.returncode, expected.stdout))
        rows = result.stdout.splitlines()
        self.assertEqual(rows[:2], [b"/usr/bin/uname"] * 2)
        self.assertTrue(rows[2].endswith(b"/uname"))
        self.assertEqual(rows[3:], [b"prefix:1", b"function:1"])
        self.assertIn(b"ambiguous redirect", result.stderr)

    def test_second_review_redirection_expansion_context(self):
        script = """x=1; >"out$((x++))"; printf '%s/' "$x"; test -f out1 || exit 1
>file <"out$((x=1))"; printf '%s/' "$x"
>file 3<"out$((x=1))"; echo "$x"
"""
        for shell in (BASH, TONARI):
            result = self.command(shell, script)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"2/2/1\n", b""))

    def test_second_review_source_skips_unreadable_path(self):
        for name in ("first", "second"):
            (self.cwd / name).mkdir()
            (self.cwd / name / "library").write_text(f"marker={name}\n")
        bad = self.cwd / "first" / "library"
        bad.chmod(0)
        if os.access(bad, os.R_OK):
            self.skipTest("current user can read mode 000")
        for shell in (BASH, TONARI):
            result = self.command(shell, 'PATH=first:second; . library && echo "$marker"')
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"second\n", b""))

    def test_second_review_case_patterns_and_read_boundaries(self):
        script = """shopt -s expand_aliases
alias D=do
alias B='{ echo function;'
for n in 1 2; D echo "$n"; done
for ((n=3; n<4; n++)); D echo "$n"; done
f() B }; f
v=$(case x in a|x) printf x;; esac; case y in
y) printf y;; esac; echo case z in word)
echo "$v"
"""
        for shell in (BASH, TONARI):
            r = self.command(shell, script)
            self.assertEqual((r.returncode, r.stdout, r.stderr),
                             (0, b"1\n2\n3\nfunction\nxycase z in word\n", b""))

    def test_second_review_associative_edit_order(self):
        script = """declare -A a
for ((i=0;i<300;i++)); do a["prefix $i"]=v; done
printf '%s\n' "${!a[@]}"
for k in 'prefix 10' 'prefix 90' 新 🔎; do
    unset 'a[$k]'; a["$k"]=x; printf '%s\n' "${!a[@]}"
done
for ((i=0;i<200;i++)); do unset 'a[prefix '$i']'; done
printf '%s\n' "${!a[@]}" """
        ref = self.command(BASH, script)
        actual = self.command(TONARI, script)
        self.assertEqual((ref.returncode, ref.stderr), (0, b""))
        self.assertEqual((actual.returncode, actual.stdout, actual.stderr), (0, ref.stdout, b""))

    def test_second_review_posix_function_prefix(self):
        for shell in (BASH, TONARI):
            for builtin, expected in [('eval :', 'g'), ('. /dev/null', 'g'), (':', 'p')]:
                script = "set -o posix; x=g; f(){ x=p " + builtin + '; echo "$x"; }; x=pre f; echo "$x"'
                r = self.command(shell, script)
                self.assertEqual((r.returncode, r.stdout, r.stderr), (0, f"p\n{expected}\n".encode(), b""))

    def test_second_review_path_and_implicit_indices(self):
        for shell in (BASH, TONARI):
            for setting, message in [('unset PATH', b'No such file or directory'), ('PATH=', b'command not found')]:
                r = self.command(shell, setting + '; no_such_tonari_test_command')
                self.assertEqual((r.returncode, r.stdout), (127, b''))
                # GNU 5.2 treats an empty PATH like unset for this diagnostic.
                if shell == BASH and bash_version() < (5, 3):
                    message = b'No such file or directory'
                self.assertIn(message, r.stderr)
            r = self.command(shell, '[[ ${BASH_VERSINFO[-1]} == "${BASH_VERSINFO[5]}" && -n ${BASH_VERSINFO[-1]} ]]')
            self.assertEqual((r.returncode, r.stdout, r.stderr), (0, b'', b''))

    def test_printf_overflowing_literal_quote_fields(self):
        source = r"""printf 'head\n[%.9223372036854775825Q]/[%s]\n' 'a b' next
echo "status:$?"
printf -v out '[%9223372036854775825q][%.9223372036854775825b]' 'a b' text
echo "status:$?/$out" """
        for shell in (TONARI, BASH) if bash_version() >= (5, 3) else (TONARI,):
            result = self.command(shell, source)
            self.assertEqual((result.returncode, result.stdout),
                             (0, b"head\n[a\\ b]/[next]\nstatus:1\nstatus:1/[a\\ b][]\n"), result.stderr)
            self.assertEqual(result.stderr.count(b"9223372036854775825"), 3, result.stderr)

    def test_second_review_printf_merged_output(self):
        cases = [
            (r"%d\n", b"ERR\n5\n6\nERR\n1\n"),
            ("%d,", b"ERR\nERR\n5,6,1,"),
            (r"%d\n%d,", b"ERR\n5\nERR\n6,1\n0,"),
        ]
        import re
        for shell in (BASH, TONARI):
            for fmt, expected in cases:
                r = self.command(shell, 'printf "$1" "5x" 6 "1e3" 2>&1', 'shell', fmt)
                output = re.sub(rb"[^\n]*?printf: [^\n]+: invalid number", b"ERR", r.stdout)
                self.assertEqual((r.returncode, output, r.stderr), (1, expected, b""))

    def test_second_review_shell_level_at_exec(self):
        cases = [
            ("/usr/bin/printenv SHLVL", b"0"),
            ("/usr/bin/printenv SHLVL; :", b"1"),
            ("SHLVL=3; exec /usr/bin/printenv SHLVL", b"2"),
            ("SHLVL=3; SHLVL=9 /usr/bin/printenv SHLVL", b"2"),
            ("/usr/bin/printenv SHLVL >/dev/stdout", b"1"),
            ("(SHLVL=4; exec /usr/bin/printenv SHLVL)", b"4"),
            ("(echo $(exec /usr/bin/printenv SHLVL))", b"1"),
            ("(exec /usr/bin/printenv SHLVL) | cat", b"1"),
            ("{ exec /usr/bin/printenv SHLVL; } & wait", b"0"),
            ("{ /usr/bin/printenv SHLVL; }", b"1"),
            ("if :; then /usr/bin/printenv SHLVL; fi", b"1"),
            ("case x in x) /usr/bin/printenv SHLVL;; esac", b"1"),
            ("true && /usr/bin/printenv SHLVL", b"0"),
            ("echo $(/usr/bin/printenv SHLVL)", b"0"),
            ("echo $(SHLVL=9 /usr/bin/printenv SHLVL)", b"8"),
            ("echo $(echo ok | /usr/bin/printenv SHLVL)", b"1"),
            ("f(){ /usr/bin/printenv SHLVL; }; echo $(f)", b"0"),
            ("level(){ /usr/bin/printenv SHLVL; }; compgen -C level 2>/dev/null", b"0"),
            ("SHLVL='3 '; exec /usr/bin/printenv SHLVL", b"0"),
            ("unset SHLVL; level=4; declare -n SHLVL=level; exec /usr/bin/printenv level", b"3"),
        ]
        for shell in self.shells_for(5):
            for command, level in cases:
                r = self.command(shell, command)
                self.assertEqual((r.returncode, r.stdout.strip(), r.stderr), (0, level, b""), (shell, command))
            for command, level, error in [
                ("readonly SHLVL; exec /usr/bin/printenv SHLVL", b"1", b"SHLVL: readonly variable"),
                ("SHLVL=1001; exec /usr/bin/printenv SHLVL", b"1", b"shell level (1000) too high"),
            ]:
                r = self.command(shell, command)
                self.assertEqual((r.returncode, r.stdout.strip()), (0, level))
                self.assertIn(error, r.stderr)
            r = self.command(shell, "unset SHLVL; export levels=4; declare -n SHLVL='levels[0]'; "
                                    "exec /usr/bin/printenv levels 'levels[0]'")
            self.assertEqual((r.returncode, r.stdout, r.stderr), (1, b"", b""))
            r = self.command(shell, 'exec -c /usr/bin/printenv SHLVL')
            self.assertEqual((r.returncode, r.stdout, r.stderr), (1, b"", b""))

    def test_second_review_dynamic_stack_and_exported_locals(self):
        for shell in (BASH, TONARI):
            script = """pushd / >/dev/null
DIRSTACK=(ignored kept); printf '<%s>' "${DIRSTACK[@]}"
DIRSTACK+=(extra); dirs +1
declare -a DIRSTACK=([1]=replaced); dirs +1
unset DIRSTACK; DIRSTACK=(ordinary value); printf '<%s>' "${DIRSTACK[@]}"; dirs +1
export marker=outer; f(){ local marker; /usr/bin/printenv marker; marker=; /usr/bin/printenv marker; }
f; echo "$marker"
compgen -A arrayvar BASH_ARG"""
            r = self.command(shell, script)
            self.assertEqual((r.returncode, r.stdout, r.stderr), (0,
                b"</><kept>kept\nreplaced\n<ordinary><value>replaced\nouter\n\nouter\nBASH_ARGC\nBASH_ARGV\n", b""))
            for decl in ("declare -i n=1/0", "f(){ local -i n=1/0; }; f", "declare a[1/0]=x"):
                r = self.command(shell, decl + "; printf unexpected")
                self.assertEqual((r.returncode, r.stdout, bool(r.stderr)), (1, b"", True))
            r = self.command(shell, 'declare a[-1]=v; echo $?; declare -p a')
            self.assertEqual((r.returncode, r.stdout, bool(r.stderr)), (0, b"1\ndeclare -a a=()\n", True))

    def test_second_review_pattern_options_by_context(self):
        for name in ("a1", "b1", ".c1"):
            (self.cwd / name).touch()
        for shell in (BASH, TONARI):
            script = """shopt -s extglob nocasematch
x=abcabc; printf '<%s>' "${x^^@(a|b)}" "${x^^!(a)}"
GLOBIGNORE=a1; compgen -G '*1' | sort; printf '<%s>' *1
HISTIGNORE='@(echo a|echo b)'; history -s 'echo a'; history -s 'echo A'; history -s 'echo c'
history | sed -E 's/^ *[0-9]+ +//'"""
            r = self.command(shell, script)
            # Bash 5.2 completion does not inherit dotglob from GLOBIGNORE.
            dot = b".c1\n" if shell == TONARI or bash_version() >= (5, 3) else b""
            self.assertEqual((r.returncode, r.stdout, r.stderr),
                             (0, b"<ABcABc><aBCaBC>" + dot + b"a1\nb1\n<.c1><b1>echo A\necho c\n", b""))

    def test_second_review_parameter_pattern_locale(self):
        self.env["LC_ALL"] = "C"
        utf8 = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        script = r"""LC_ALL="$1"; v=éa; printf '<%s>' "${v#?}" "${v#[[:alpha:]]}" "${v//[[:alpha:]]/X}" "${v^^[[:alpha:]]}"
LC_ALL=C; printf '<%s>' "${v#[[:alpha:]]}"
v='xa\'; p='[a]\'; printf '<%s>' "${v/%$p/Z}" "${v//$p/Z}" """
        for shell in (BASH, TONARI):
            r = self.command(shell, script, "shell", utf8)
            self.assertEqual((r.returncode, r.stdout, r.stderr),
                             (0, "<a><a><XX><ÉA><éa><xZ><xa\\>".encode(), b""))

    def test_second_review_seekable_stdin_last_argument(self):
        script = self.cwd / "script"
        script.write_bytes(b"echo old\n:\nprintf '<%s>' \"$_\"\ntrue\nprintf '<%s>' \"$_\"\nx=1\nx=2\nprintf '<%s>' \"$_\"\n")
        for shell in (BASH, TONARI):
            with script.open("rb") as fd:
                result = subprocess.run([shell], cwd=self.cwd, env=self.env, stdin=fd,
                                        capture_output=True, timeout=5)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"old\n<:><true><>", b""))

    def test_second_review_inherited_closed_standard_descriptors(self):
        for fd in range(3):
            source = f"exec 9>&{fd}"
            for shell in (TONARI, BASH):
                with self.subTest(fd=fd, shell=shell):
                    r = subprocess.run([shell, "-c", source], env=self.env, cwd=self.cwd,
                                       capture_output=True, preexec_fn=lambda: os.close(fd), timeout=5)
                    self.assertNotEqual(r.returncode, 0)
                    control = self.command(shell, f"exec {fd}<>/dev/null; {source}")
                    self.assertEqual(control.returncode, 0, control.stderr)
        path = self.cwd / "large-group"
        path.write_text("{\n#" + "x" * 32768 + "\n: 4<&0; echo status=$?\n}\n")
        for shell in (BASH, TONARI):
            r = subprocess.run([shell, path], env=self.env, cwd=self.cwd, capture_output=True,
                               preexec_fn=lambda: os.close(0), timeout=5)
            self.assertEqual((r.returncode, r.stdout), (0, b"status=1\n"), r.stderr)

    def test_second_review_builtin_operands(self):
        for script, output in [
            ("f(){ :; }; unset -fn f; type -t f; echo $?", b"1\n"),
            ("set -- base; set -o '' -Z >/dev/null; echo \"$?/$1\"; "
             "set -o - -Z >/dev/null; echo \"$?/$1\"", b"2/base\n2/base\n"),
            ("cat <<$\"EOF\"\nline\nEOF\nprintf done\n", b"line\ndone"),
        ]:
            for shell in (TONARI, BASH):
                r = self.command(shell, script)
                self.assertEqual((r.returncode, r.stdout), (0, output), (shell, script, r.stderr))
        utf8 = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for shell in ((TONARI, BASH) if bash_version() >= (5, 3) else (TONARI,)):
            # Reset OPTIND when switching explicit option lists in the differential check.
            # Without a reset, results vary between GNU Bash builds.
            r = self.command(shell, 'getopts abc o -abc; getopts abc o -abc; OPTIND=1; getopts xyz o -x; echo "$?/$o/$OPTIND"')
            self.assertEqual((r.returncode, r.stdout, r.stderr), (0, b"0/x/2\n", b""))
            r = self.command(shell, """LC_ALL="$1"; printf '%.1Q/' é; printf '%d/' ''; echo $?
printf '%d/'; echo $?""", "shell", utf8)
            self.assertEqual((r.returncode, r.stdout), (0, b"$'\\303'/0/1\n0/0\n"), r.stderr)

        r = self.command(TONARI, 'getopts abc o -abc; getopts abc o -abc; getopts xyz o -x; echo "$?/$o/$OPTIND"')
        self.assertEqual((r.returncode, r.stdout, r.stderr), (0, b"0/x/2\n", b""))

    def test_scaling_multiline_readers_leave_following_input(self):
        source = ("a=(\n" + "entry\n" * 300 +
                  ")\nx=$(\n:\necho one\n); y=\"first\nsecond\"\n" +
                  "(\n" + "x=$((1))\n" * 300 + "echo \"$x\"\n)\n" +
                  "a0=ab\\\ncd\n" +
                  "read -r v\nfrom-stdin\necho \"${#a[@]}/$x/$a0/$v\"\n")
        for shell in (BASH, TONARI):
            proc = subprocess.run([shell], input=source.encode(), capture_output=True,
                                  cwd=self.cwd, env=self.env, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, b"1\n300/one/abcd/from-stdin\n", b""))

    def test_scaling_array_updates_keep_declaration_and_callback_scope(self):
        script = r"""declare -A m=([k]=old); a=(two 2 three)
f() { local -A m=([k]=inner); declare -g 'm[k]=new'; echo "${m[k]}"; }
f; declare 'a[a[1]]=third'; declare 'a[0]+=!'; echo "${a[*]}/${m[k]}"
cb() { if [[ $1 == 3 ]]; then a[0]=changed; fi; }
mapfile -O3 -t -c1 -C cb a <<<$'next\nlast'
echo "${a[*]}"
declare -ia n=([0]=2); n[0]="${n[0]}3"; echo "${n[0]}"
declare -A h=([k]=x); for i in 1 2 3; do h[k]="${h[k]}y"; done; echo "${h[k]}"
"""
        for shell in self.shells_for(5):
            proc = self.command(shell, script)
            # Bash redirects this element assignment into the local despite -g.
            scopes = b"new\ntwo! 2 third/old" if shell == BASH else b"inner\ntwo! 2 third/new"
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, scopes + b"\nchanged 2 third next last\n23\nxyyy\n", b""))

    def test_scaling_unicode_spans_and_required_suffix(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        script = r"""shopt -s extglob; s=; for ((i=0;i<130;i++)); do s+=雪桜; done
for p in '+(雪|桜)z' '*[[:digit:]]' '雪*1' '*ァ' '+(雪)桜'; do
v=${s}1z; printf '<%s>\n' "${v//$p/R}" "${v/%$p/R}" "${v##$p}" "${v%%$p}"
done
shopt -s nocasematch; s=${s}K; echo "${s//*k/ok}"
"""
        ref = self.command(BASH, script)
        result = self.command(TONARI, script)
        self.assertEqual((ref.returncode, ref.stderr), (0, b""))
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, ref.stdout, b""))
        mismatch = self.command(TONARI, "s=aaa; [[ $s == !(z)*!(z)*[y] ]]")
        self.assertEqual(mismatch.returncode, 1)

    def test_scaling_incomplete_escape_in_pattern(self):
        script = r"""p='*\'; printf -v s '%0300d' 0; s=${s//0/\\}
for option in -u -s; do
    shopt "$option" nocasematch
    v=${s//$p/X}; x=${s##$p}; [[ $s == $p ]]; echo "$? ${#v} ${#x}"
done
p='*\\'; v=${s//$p/X}; x=${s##$p}; echo "$v ${#x}" """
        self.compare(script, 0, b"1 300 300\n1 300 300\nX 0\n")

    def test_scaling_erasedups_tracks_trim_delete_and_import(self):
        (self.cwd / "history-data").write_text("aa\nbb\naa\n")
        script = r"""HISTSIZE=6; HISTCONTROL=erasedups
history -s one; history -s two; history -s one; history -r history-data
history -s zz; history -d 2; history -s aa
history | sed -E 's/^ *[0-9]+ +//'
history -c; history -s one; history -s one; history | sed -E 's/^ *[0-9]+ +//'"""
        ref = self.command(BASH, script)
        proc = self.command(TONARI, script)
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (ref.returncode, ref.stdout, ref.stderr))
        self.assertIn(b"aa\n", proc.stdout)

    def test_scaling_pattern_edges_and_braces(self):
        script = r"""shopt -s extglob
printf -v pad '%0300d' 0; s="a${pad}b1z"
for p in '*z' '[a-z]*z' '*[0-9]' '[0-9]*' '+(a|b)z' '+([0-9])b' '*(a|b)z'; do
printf '<%s>\n' "${s#$p}" "${s##$p}" "${s%$p}" "${s%%$p}" "${s//$p/X}" "${s/%$p/X}"
done
shopt -s nocasematch; echo "${s//*Z/ok}"
printf '%s\n' {outer,{one,two}} {unclosed{inner,a} {x{1..2},y}
"""
        ref = self.command(BASH, script)
        proc = self.command(TONARI, script)
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (ref.returncode, ref.stdout, ref.stderr))

    def test_array_evaluation_append_keeps_scope_and_partial_writes(self):
        self.compare("""declare -A m=([old]=one); m+=([new]=two [old]+=!)
a=(2 4); a+=([a[0]]=six [-1]+=!); declare -n ref=a; ref+=([4]=eight)
f() { local -A m=([inner]=x); m+=([new]=y); echo "${m[inner]}${m[new]}"; }
f; printf '%s/%s/%s/%s/%s\\n' "${m[old]}" "${m[new]}" "${a[0]}" "${a[2]}" "${a[4]}"
declare -ia nums=(2); nums+=([1]='nums[0]+3'); echo "${nums[*]}"
""", 0, b"xy\none!/two/2/six!/eight\n2 5\n", minimum=5)

    def test_array_evaluation_declare_global_append_with_local_shadow(self):
        self.compare("""declare -A m=([outer]=old)
f() { local -A m=([inner]=here); declare -gA m+=([new]=yes); echo "${m[inner]}/${m[new]-absent}"; }
f; echo "${m[outer]}/${m[new]}"
""", 0, b"here/absent\nold/yes\n", minimum=5)

    def test_array_evaluation_partial_append_before_error(self):
        script = """a=(old); trap 'echo "${a[*]}"' EXIT
a+=([1]=first [-99]=invalid)"""
        for shell in self.shells_for(minimum=5):
            with self.subTest(shell=shell):
                proc = self.command(shell, script)
                expected_status = 0 if shell == BASH and self.bash_before_53() else 1
                self.assertEqual(proc.returncode, expected_status)
                self.assertEqual(proc.stdout, b"old first\n")
                self.assertTrue(proc.stderr.strip())

    def test_array_evaluation_completion_observes_child_state(self):
        self.compare("""v=parent; wl='${v:=wrong}'; unset side
out=$(compgen -W '${side:=child} literal' -- ch); echo "$out/${side-unset}"
cur=ab; words='ab abc space'; COMPREPLY=( $(compgen -W "$words" -- "$cur") ); echo "${COMPREPLY[*]}"
out=$(compgen -W x -- unavailable); echo "$?:$out"
compgen() { v=local; echo from-function; }; out=$(compgen -W a); echo "$v/$out"
""", 0, b"child/unset\nab abc\n1:\nparent/from-function\n", minimum=5)

    def test_array_evaluation_group_isolation(self):
        self.compare("""x=parent; ( x=child; :; exit 17 ); printf '%s/%s\\n' "$?" "$x"
( false; exit ); echo "$?"
echo() { printf '[%s]' "$1"; }; ( echo child ); printf '\\n'
unset -f echo; ( printf '%s\\n' hi ) >file; cat file
( printf -v x %s changed ); printf '%s\\n' "$x"
( printf '%s' "$BASH_SUBSHELL" ); echo
""", 0, b"17/parent\n1\n[child]\nhi\nparent\n1\n", minimum=5)

    def test_array_evaluation_output_signal_stays_in_child(self):
        reader, writer = os.pipe()
        os.close(reader)
        self.addCleanup(os.close, writer)
        for shell in (BASH, TONARI):
            with (self.cwd / "output").open("wb") as file:
                for fd, setup, payload, sig in (
                    (writer, "", "hi", signal.SIGPIPE),
                    (file, "ulimit -f 1; ", "a" * 8192, signal.SIGXFSZ),
                ):
                    with self.subTest(shell=shell, signal=sig):
                        script = setup + f"( echo {payload} ); echo survived_$? >&2"
                        proc = subprocess.run([shell, "-c", script], stdout=fd, stderr=subprocess.PIPE,
                                              cwd=self.cwd, env=self.env, timeout=5)
                        self.assertEqual(proc.returncode, 0)
                        self.assertEqual(proc.stderr.splitlines()[-1], f"survived_{128 + sig}".encode())

    def test_array_evaluation_medium_here_input(self):
        self.compare("""printf -v s '%16384s' ''; s+=end
cb() { IFS= read -r extra; echo "$1/${#2}/$extra"; }
mapfile -t -c 1 -C cb a <<EOF
$s
second
EOF
mapfile -t -n1 a <<<"$s"; echo "${#a[0]}"
readarray -t -u 3 a 3<<<other <<<"$s"; echo "${a[0]}"
""", 0, b"0/16387/second\n16387\nother\n", minimum=4)

    def test_array_evaluation_case_snapshot_and_quotes(self):
        self.env["LC_ALL"] = "C.UTF-8"
        self.compare(r"""s=abc; case $s in a*) s=zzz ;;& abc) echo old;; esac
s=abc; case $s in z|${x:=abc}) echo "$x";; esac
s="a'b"; eval "v=${s@Q}"; echo "$v"; eval "${s@A}"; echo "$s"
s='line\nend'; printf '%s\n' "${s@E}"
s=éΑİ; echo "${s^^}/${s,,}"
""", 0, "old\nabc\na'b\na'b\nline\nend\nÉΑİ/éαi\n".encode(), minimum=5)

    def test_shell_paths_capture_queries(self):
        self.compare("""mkdir -p real; ln -s real link; cd link
a=$(pwd); b=$(pwd -P); echo "${a##*/} ${b##*/}"
x=$(command -v printf); echo "$x $?"
x=$(type -t no-such-r7-name); echo "<$x> $?"
f() { :; }; x=$(type -t f); echo "$x"
type() { echo override; }; x=$(type -t ls); echo "$x"
command() { echo override2; }; x=$(command -v printf); echo "$x"
pwd() { echo override3; }; x=$(pwd); echo "$x"
""", 0, b"link real\nprintf 0\n<> 1\nfunction\noverride\noverride2\noverride3\n")

    def test_shell_paths_helper_isolation(self):
        self.compare("""set -- outside; a=original; n=100; count=0
f() { local a=$1; if [ "$a" = "x y" ]; then echo "$a"; else echo no; fi; }
x=$(f 'x y'); printf '<%s,%s,%s>\\n' "$x" "$a" "$1"
g() { local n=$(($1 + 1)); echo "$n"; }; x=$(g 12); echo "$x $n"
x=$(g 'count++'); echo "$x $count"
h() { if [[ $1 == foo* ]]; then echo yes; else echo no; fi; }
echo "$(h foobar) $(h no)"
""", 0, b"<x y,original,outside>\n13 100\n1 0\nyes no\n")

    def test_shell_paths_expansions(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare("""printf -v pad '%1100s' ''; s=雪${pad}🐟${pad}桜
echo "${s:1101:1} ${s: -1102:1} ${s:1101:-1101}"
a=(x '' z); IFS=雪; echo "${a[*]}"; unset 'a[1]'; echo "${a[*]}"; printf '%s ' "${!a[@]}"; echo
s=ababa; echo "${s//ab/\\&} ${s#a*b} ${s%*ba}"
s=abcd; unset v; echo "${s/${v:=ab}/X} $v"
shopt -s nocasematch; for v in a7X aX; do case $v in A*[0-9]*x) echo yes;; *) echo no;; esac; done
v=old; case "$v" in old) v=new; echo 1;;& old) echo 2;; esac
""", 0, "🐟 🐟 🐟\nx雪雪z\nx雪z\n0 2 \n&&a aba aba\nXcd ab\nyes\nno\n1\n2\n".encode(), minimum=5)

    def test_shell_paths_completions_and_wait(self):
        (self.cwd / "bin").mkdir()
        self.compare("""PATH=$PWD/bin:/usr/bin:/bin
rm -f bin/r7cmd; touch bin/r7cmd; chmod +x bin/r7cmd
compgen -c r7cmd
chmod -x bin/r7cmd; compgen -c r7cmd; echo $?
rm bin/r7cmd; mkdir bin/r7cmd; compgen -c r7cmd; echo $?
rmdir bin/r7cmd; ln -s /bin/ls bin/r7cmd; compgen -c r7cmd
exec 3< <(echo ready; exit 7); p=$!; read -ru3 v; wait "$p"; echo "$?:$v"
""", 0, b"r7cmd\n1\n1\nr7cmd\n7:ready\n", minimum=5)

    def test_shell_paths_folded_glob_and_prompt(self):
        self.compare(r"""mkdir -p inner; cd inner
touch a A.TXT .A.txt aZZtxt
shopt -s nocaseglob
printf '<%s>' a*.txt; echo
shopt -s dotglob; printf '<%s>' *.TXT; echo
p='\w'; HOME=$PWD; echo "${p@P}"
HOME=/; PROMPT_DIRTRIM=1; [[ ${p@P} == */inner ]] && echo path
""", 0, b"<A.TXT>\n<.A.txt><A.TXT>\n~\npath\n", minimum=5)

    def test_shell_paths_read_restores_offset_before_assignment(self):
        (self.cwd / "records").write_text("first\nkey\nlast\n")
        self.compare("""shopt -u assoc_expand_once; declare -A a; exec 3<records
IFS= read -ru3 'a[$(IFS= read -ru3 v; printf %s "$v")]'
IFS= read -ru3 tail; printf '%s/%s' "${a[key]}" "$tail"
""", 0, b"first/last", minimum=5)

    def test_shell_paths_fields_and_formats(self):
        self.compare("""printf 'a:b::c:d:e:f:g:h:i:j:\\nsecond\\n' >records
{ IFS=: read -r a b c d e f g h; read -r next; } <records
echo "$a|$c|$h|$next"
printf '<%+08d><%08x><%08X><%08o><%08.3d>\\n' -3 31 31 8 9
v=x; declare -p v; f() { local v=L; declare -p v; }; f; declare -p v
""", 0, b"a||g:h:i:j:|second\n<-0000003><0000001f><0000001F><00000010><     009>\ndeclare -- v=\"x\"\ndeclare -- v=\"L\"\ndeclare -- v=\"x\"\n", minimum=5)

    def test_runtime_review_cached_lengths_and_references(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare('''s=abc; out=${#s}; echo "$out"
s=雪; out=${#s}; echo "$out"
f() { local s="🐟🐟"; echo "${#s}"; }; f; echo "${#s}"
printf -v s %s xy; echo "${#s}"; IFS= read -r s <<<z; echo "${#s}"
target=first; declare -n ref=target chain=ref; copy=$chain; chain=second
echo "$copy $target"; a=(first); declare -n indexed='a[0]'; indexed=next; echo "$indexed"
''', 0, b"3\n1\n2\n1\n2\n1\nfirst second\nnext\n", minimum=5)

    def test_runtime_review_braces_and_loop_snapshot(self):
        self.compare('''printf '<%s> ' pre_{x,{a,b}_{1..2},z}_end; echo
v=X; printf '<%s> ' {a,}${v} {,b} "q"{x,y}; echo
a=(one two); for x in "${a[@]}"; do echo "$x"; a=(changed); done
''', 0, b"<pre_x_end> <pre_a_1_end> <pre_a_2_end> <pre_b_1_end> <pre_b_2_end> <pre_z_end> \n<aX> <X> <b> <qx> <qy> \none\ntwo\n")

    def test_runtime_review_loop_rechecks_braces_and_positional_arguments(self):
        self.compare('''set -- starting; echo "$_"
for i in 1 2 3 4 5 6; do
    case "$i" in 4) set +B;; 5) set -B;; esac
    set -- pre{a,b}_{1..2}; printf '<%s/%s/%s> ' "$#" "$1" "$_"; shift
done
echo; set --; echo "$_"
''', 0, b"starting\n" + b"<4/prea_1/preb_2> " * 3
             + b"<1/pre{a,b}_{1..2}/pre{a,b}_{1..2}> " + b"<4/prea_1/preb_2> " * 2 + b"\n--\n")

    def test_runtime_review_patterns_recheck_options_and_path_boundaries(self):
        self.compare('''mkdir -p sub/nest
touch sub/a1.txt sub/a2.txt sub/A3.txt sub/nest/a1.txt
GLOBIGNORE='sub/a[13]*.txt'
printf '<%s> ' sub/*.txt sub/nest/*.txt; echo
shopt -s nocaseglob; printf '<%s> ' sub/*.txt sub/nest/*.txt; echo
shopt -u nocaseglob; printf '<%s> ' sub/*.txt; echo
for ((i=0;i<72;i++)); do p="^(ok|$i)$"; [[ ok =~ $p ]] || exit 1; done
p='('; [[ ok =~ $p ]] 2>/dev/null; echo "$?"
''', 0, b"<sub/A3.txt> <sub/a2.txt> <sub/nest/a1.txt> \n<sub/a2.txt> <sub/nest/a1.txt> \n<sub/A3.txt> <sub/a2.txt> \n2\n", minimum=5)

    def test_runtime_review_arithmetic_fallback(self):
        self.compare('''x=0; v=2; for i in 1 2 3; do ((x += (v*3+7)^(i>>1))); done; echo "$x"
v='++calls'; calls=0; for i in 1 2; do n=$((v+1)); echo "$n $calls"; done
a=0; v=$((0 && ++a)); v=$((1 ? 5 : ++a)); echo "$a $v"
echo $((2**3**2))
''', 0, b"37\n2 1\n3 2\n0 5\n512\n")

    def test_runtime_review_current_capture_effects(self):
        source = '''i=2; set -- "one two"; x=${ echo "v_$i" "$1"; }; echo "$x"
x=${ echo "$((++i))"; }; echo "$x $i"
x=${ printf -v changed %s yes; }; echo "<$x> $changed"
x=${ printf 'abc%n' count; }; echo "$x $count"
echo() { printf '[%s]' "$*"; }; x=${ echo hi; }; printf '%s' "$x"'''
        expected = b"v_2 one two\n3 3\n<> yes\nabc 3\n[hi]"
        for shell in ((TONARI,) if self.bash_before_53() else (TONARI, BASH)):
            with self.subTest(shell=shell):
                result = self.command(shell, source)
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, expected, b""))

    def test_readonly_helper_uses_callee_arguments_and_restores_locals(self):
        self.compare(r'''set -- outside
v=parent; p=parent
normalize() { local v="$1"; case "$v" in v*) echo "$v";; *) echo "v$v";; esac; }
strip() { local p="$1"; if [ -n "$p" ]; then printf '%s\n' "${p#/}"; else echo empty; fi; }
printf '<%s>\n' "$(normalize 'a b')" "$(normalize v)" "$(strip /ok)" "$(strip '')"
echo "$1 $v $p"
function [ { return 1; }
echo "$(strip /ok)"
unset -f '['
shopt -s nocasematch
echo "$(normalize Vyes)"
observe() { local v="$1"; case "$v" in *) echo "$BASH_SUBSHELL";; esac; }
echo "$(observe ok)"
''', 0, b"<va b>\n<v>\n<ok>\n<empty>\noutside parent parent\nempty\nVyes\n1\n")

    def test_compiled_parameter_patterns_and_conditional_options(self):
        self.compare(r'''shopt -s extglob
a=(foofoo-ffoo foo fo '雪fo')
printf '<%s>\n' "${a[@]//+(f|fo|foo)/X}"
printf '<%s>\n' "${a[@]##+(foo|fo)}"
match() { [[ $1 == @(foo|bar)_+([0-9]) ]]; }
match foo_71; echo "$?"; match FOO_7; echo "$?"
shopt -s nocasematch; match FOO_7; echo "$?"
s=''; set -- "${s:+words}" "${s:-a b}"; printf '<%s>\n' "$@"
''', 0, b"<X-X>\n<X>\n<X>\n<\xe9\x9b\xaaX>\n<-ffoo>\n<>\n<>\n<\xe9\x9b\xaafo>\n0\n1\n0\n<>\n<a b>\n", minimum=4)

    def test_utf8_scalar_length_at_block_boundaries(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare("""for w in 1022 1023 1024 1025 8192; do
printf -v s '%*s' "$w" x; s+=🐟雪; echo "${#s}"
done""", 0, b"1024\n1025\n1026\n1027\n8194\n")

    def test_query_prefers_executable_and_completion_rechecks_permissions(self):
        first = self.cwd / "first"
        second = self.cwd / "second"
        for directory in (first, second):
            directory.mkdir()
            (directory / "probe").touch()
        (second / "probe").chmod(0o700)
        self.env["PATH"] = f"{first}:{second}:/bin:/usr/bin"
        script = '''command -v probe; type -t probe; compgen -c pr | /usr/bin/grep -x probe | /usr/bin/head -1
/bin/chmod 600 "$1/probe"
command -v probe; type -t probe
type -aP probe >/dev/null; echo all=$?
compgen -c pr | /usr/bin/grep -x probe; echo completion=$?
/bin/chmod 700 "$1/probe"
command -v probe'''
        expected = f"{second}/probe\nfile\nprobe\n{first}/probe\nfile\nall=1\ncompletion=1\n{second}/probe\n".encode()
        for shell in self.shells_for():
            (second / "probe").chmod(0o700)
            proc = self.command(shell, script, "bench", str(second))
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, expected, b""))

    def test_keyword_operands_beyond_bitset_and_quoted_spelling(self):
        filler = " ".join(["x"] * 70)
        self.compare("f() { printf '<%s:%s:%s>\\n' \"$one\" \"$two\" \"$#\"; }\n"
                     "set -k\nf one=1 " + filler + " two=2 'quoted=yes'\n"
                     "set +k\nf one=1 two=2", 0, b"<1:2:71>\n<::2>\n")

    def test_memory_here_inputs_preserve_fd_observers(self):
        self.compare(r'''IFS=: read -ra a <<< 'a:b'
printf '<%s>\n' "${a[@]}"
read -rd : x <<< 'ab:cd'; echo "$?:$x"
read -rN 4 x <<< 'a b:c'; echo "$?:$x"
read -rn 8 x <<< 'ab'; echo "$?:$x"
exec 3<<<outer
read -ru3 x <<< inner; echo "$x"
cb() { IFS= read -r x; echo "callback:$x"; }
mapfile -C cb -c 1 a <<'END'
one
two
END
echo "first=${a[0]}"
''', 0, b"<a>\n<b>\n0:ab\n0:a b:\n0:ab\nouter\ncallback:two\nfirst=one\n\n", minimum=4)

    def test_simple_lists_in_path_filters_and_folded_extglob(self):
        self.env["LC_ALL"] = "C"
        self.compare("""mkdir -p tree; touch tree/a tree/b tree/c
GLOBIGNORE='tree/[ab]'; printf '%s\n' tree/*
shopt -s nocasematch
[[ Abba == +([ab]) ]]; echo "$?"
[[ cab == +([ab]) ]]; echo "$?"
rm -r tree""", 0, b"tree/c\n0\n1\n")

    def test_folded_class_distinguishes_turkish_dotted_i(self):
        self.env["LC_ALL"] = "tr_TR.UTF-8"
        proc = self.command(TONARI, """shopt -s nocasematch; p='+([i])'
for ch in I i; do [[ $ch == $p ]]; echo "$?"; done""")
        self.assertEqual((proc.stdout, proc.returncode), (b"1\n0\n", 0))

    def test_static_conditional_rechecks_locale_after_first_use(self):
        locales = subprocess.check_output(["locale", "-a"], text=True).splitlines()
        utf8 = next((name for name in locales if name.lower().replace("-", "") in
                     ("en_us.utf8", "c.utf8")), None)
        if utf8 is None:
            self.skipTest("requires an installed UTF-8 locale")
        script = """f() { [[ $1 == +([a-ú]) ]]; }
pattern='+([a-ú])'
for LC_ALL in C "$1" C "$1"; do
    f é; cached=$?
    [[ é == $pattern ]]; direct=$?
    [[ $cached == "$direct" ]] || exit 1
done"""
        for shell in self.shells_for():
            tested = self.command(shell, script, "bench", utf8)
            self.assertEqual((tested.stdout, tested.stderr, tested.returncode), (b"", b"", 0))

    def test_short_here_input_reports_descriptor_exhaustion(self):
        for name in ("read -r x", "mapfile -t x"):
            for limit, expected in [(4, b"1:\n"), (5, b"0:ok\n")]:
                source = f'ulimit -n {limit}; {name} <<<ok; echo "$?:$x"'
                with self.subTest(builtin=name, limit=limit):
                    tested = self.command(TONARI, source)
                    self.assertEqual((tested.returncode, tested.stdout), (0, expected))
                    self.assertEqual(bool(tested.stderr), limit == 4)

    def test_counted_read_rewinds_to_next_utf8_character(self):
        (self.cwd / "words").write_text("雪a\n🐟b\nxyz", encoding="utf-8")
        proc = self.command(TONARI, """exec 3<words
IFS= read -ru3 -n1 x; echo "$?:$x"
IFS= read -ru3 -n4 x; echo "$?:$x"
IFS= read -ru3 -n1 x; echo "$?:$x"
IFS= read -ru3 x; echo "$?:$x"
IFS= read -ru3 -n8 x; echo "$?:$x" """)
        self.assertEqual((proc.returncode, proc.stdout.decode(), proc.stderr),
                         (0, "0:雪\n0:a\n0:🐟\n0:b\n1:xyz\n", b""))

    def test_closed_builtin_stdout_reports_only_attempted_writes(self):
        shells = (TONARI,) if self.bash_before_53() else (BASH, TONARI)
        for shell in shells:
            for command in ("printf %s hi", "printf '%05d' 7", "pwd", "umask -p",
                            "hash", "alias a=text; alias", "dirs", "times", "x=1; declare -p x",
                            "export -p", "type -t pwd", "shopt -p", "f(){ :; }; declare -f f",
                            "hash -p /bin/ls mark; hash -t mark", "complete -W yes mark; complete -p mark"):
                with self.subTest(shell=shell, command=command):
                    proc = self.command(shell, command + " >&-; exit $?")
                    self.assertEqual((proc.returncode, proc.stdout), (1, b""))
                    self.assertEqual(len(proc.stderr.splitlines()), 1, proc.stderr)
            for command in ("alias", "printf %s ''", "printf -v value %s hi", "umask 022",
                            "shopt -q promptvars", "complete -W yes mark"):
                proc = self.command(shell, command + " >&-; exit $?")
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"", b""))

    def test_simple_and_nested_alias_parameters(self):
        self.compare("""shopt -s expand_aliases
alias show='printf %s "${v}"'
alias nested='printf "<%s>" "$(show)"'
for v in 'two words' '*'; do eval nested; done
set -o posix
v=changed; eval nested
alias show='printf %s again'; eval nested
""", 0, b"<two words><*><changed><again>")

    def test_function_name_order_tracks_live_definitions(self):
        self.compare("""fn_z() { :; }; fn_a() { :; }; fn_b() { :; }
compgen -A function fn_
unset -f fn_b; fn_m() { :; }; fn_a() { echo updated; }
compgen -A function fn_; fn_a
export -f fn_m fn_a
export -fp | sed -n '/^declare -fx fn_/p'
declare -F | sed -n '/ fn_/p'
set | sed -n '/^fn_[amz] ()/s/ .*//p'
""", 0, b"fn_a\nfn_b\nfn_z\nfn_a\nfn_m\nfn_z\nupdated\n"
        b"declare -fx fn_a\ndeclare -fx fn_m\n"
        b"declare -fx fn_a\ndeclare -fx fn_m\ndeclare -f fn_z\nfn_a\nfn_m\nfn_z\n",
                     minimum=4)

    def test_large_variable_table_visibility_and_environment(self):
        self.compare("""for ((i=0;i<1000;i++)); do printf -v "p_$i" %s "$i"; done
q_z=last; q_a=first; q_m=middle; unset q_m; echo "${!q_*}"
q_b=added; unset q_z; echo "${!q_*}"
g() { local q_c=local; echo "${!q_*}"; }; g; echo "${!q_c*}"
export ANSWER=outer
f() { local ANSWER=inner; /bin/sh -c 'echo "$ANSWER"'; }
f; /bin/sh -c 'echo "$ANSWER"'; unset ANSWER; /bin/sh -c 'echo "${ANSWER-missing}"'
""", 0, b"q_a q_z\nq_a q_b\nq_a q_b q_c\n\ninner\nouter\nmissing\n")

    def test_repeated_scalar_assignments_keep_attributes(self):
        self.compare("""a=ab; b=; declare -u upper=old; declare -i n=0
f() { for k in one two; do b="$a"; upper=$a; n=$1; done; }
f 2+3; echo "$b $upper $n"
declare -n ref=a; ref=$b; a+=$a; echo "$ref"
set -a; b=$a; /bin/sh -c 'echo "$b"'
""", 0, b"ab AB 5\nabab\nabab\n", minimum=5)

    def test_repeated_function_arguments_and_plain_locals(self):
        self.compare("""show() { printf '<%s>' "$@"; echo; }
f() { local a=first a+=x empty=; show "$a" "$1" "$empty" "${late:=two words}" "$late"; }
f 'one two'; unset late; f '*'
n=outside; f2() { local n=inner; export check=$n; /bin/sh -c 'echo "$check"'; }
f2; echo "$n"
""", 0, b"<firstx><one two><><two words><two words>\n<firstx><*><><two words><two words>\ninner\noutside\n")

    def test_local_listing_and_special_scope_restore(self):
        self.compare("""f() { local z=1 a=2 m=3; local -p; }
f
PATH=/usr/bin:/bin; GLOBIGNORE='*.bak'; BASH_COMPAT=4.4
g() { local PATH=/bin GLOBIGNORE='*.tmp' BASH_COMPAT=5.1;
      printf '%s %s %s\\n' "$PATH" "$GLOBIGNORE" "$BASH_COMPAT"; }
g; printf '%s %s %s\\n' "$PATH" "$GLOBIGNORE" "$BASH_COMPAT"
""", 0, b'declare -- a="2"\ndeclare -- m="3"\ndeclare -- z="1"\n/bin *.tmp 5.1\n/usr/bin:/bin *.bak 4.4\n', minimum=5)

    def test_repeated_arithmetic_reads_live_elements(self):
        self.compare("""a=(2 6); i=0
f() { ((a[i]+=1)); ((v+=(i*3+7)^(i>>1))); }
v=0; f; i=1; f; a[1]='3+4'; f; i=-1; f
echo "${a[@]} / $v"
( i=0; f; echo "${a[0]}" ); echo "${a[0]}"
declare -A a2=([other]=3); name=other
g() { ((a2[$name]+=2)); }; g; g; echo "${a2[other]}"
""", 0, b"3 9 / 22\n4\n3\n7\n", minimum=4)

    def test_inline_compound_terminators(self):
        for header, end in (("{ :", "; }"), ("if :; then", "; fi"),
                            ("for item in x; do", "; done"), ("case x in x)", " ;; esac")):
            close = end.split()[-1]
            body = ("shopt -s expand_aliases\n" + header + "\n: <<'END'\n"
                    + (f":; {close} >false-close\n" * 64) + "END\n"
                    + "alias after='printf alias'\n" + "v=ok\n" * 250
                    + "printf '%s' \"$v\"" + end + " 2>&1 >result\n"
                    + "after; cat result; test ! -e false-close\n")
            (self.cwd / "unit.sh").write_text(body)
            for shell in self.shells_for():
                for command in (body, ". ./unit.sh"):
                    with self.subTest(close=close, shell=shell, sourced=command != body):
                        p = self.command(shell, command)
                        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"aliasok", b""))

    def test_associative_field_cache(self):
        (self.cwd / "hit-one").touch()
        source = r"""declare -A m=(['space key']='a b' [p]='hit-*' [blank]='' [雪]='冬')
declare -n ref=m
dump() { printf 'k<%s>\n' "${!m[@]}"; printf 'v<%s>\n' "${ref[@]}"; }
dump; m['space key']=updated; dump
unset 'm[blank]'; m[new]='x:y'; IFS=:; dump
printf 'field<%s>\n' ${ref[@]}; (m[new]=sub; dump); dump
alias one=live; printf 'live<%s>\n' "${BASH_ALIASES[@]}"; unalias one
alias two=changed; printf 'live<%s>\n' "${BASH_ALIASES[@]}"
"""
        p = self.command(TONARI, source)
        self.assertEqual((p.returncode, p.stderr), (0, b""))
        for line in (b"v<a b>", b"v<>", b"v<updated>", b"field<hit-one>", b"field<x>",
                     b"field<y>", b"v<sub>", b"live<live>", b"live<changed>"):
            self.assertIn(line, p.stdout.splitlines())
        if BASH in self.shells_for(minimum=5):
            ref = self.command(BASH, source)
            self.assertEqual((ref.returncode, ref.stdout, ref.stderr), (0, p.stdout, b""))

    def test_background_trap_listing(self):
        # Bash 5.3 prints dispositions ignored on entry. Earlier versions differ.
        for shell in ((TONARI,) if self.bash_before_53() else (TONARI, BASH)):
            for mask in range(4):
                signals = (("INT", signal.SIGINT), ("QUIT", signal.SIGQUIT))
                def setup():
                    for i, (_, sig) in enumerate(signals):
                        signal.signal(sig, signal.SIG_IGN if mask & (1 << i) else signal.SIG_DFL)
                for posix in (False, True):
                    opts = ["--posix"] if posix else []
                    prefix = "" if posix else "SIG"
                    def listing(explicit):
                        lines = []
                        for i, (name, _) in enumerate(signals):
                            ignored = mask & (1 << i) or explicit
                            if ignored or posix:
                                action = "''" if ignored else "-"
                                lines.append(f"trap -- {action} {prefix}{name}\n")
                        return "".join(lines)
                    code = ("trap -p >/dev/null; (trap -p INT QUIT; trap '' INT QUIT; "
                            "trap -p INT QUIT; trap - INT QUIT; trap -p INT QUIT) & wait")
                    p = subprocess.run([shell, *opts, "-c", code], env=self.env, cwd=self.cwd,
                                       preexec_fn=setup, capture_output=True, timeout=5)
                    with self.subTest(shell=shell, mask=mask, posix=posix):
                        self.assertEqual((p.returncode, p.stdout, p.stderr),
                                         (0, (listing(False) + listing(True) + listing(False)).encode(), b""))

    def test_bulk_listing_format(self):
        self.compare("hash -p /bin/echo mark; mark >/dev/null; hash", 0,
                     b"hits\tcommand\n   1\t/bin/echo\n")
        self.compare("for ((i=0;i<11;++i)); do pushd -n /tmp >/dev/null; done; dirs -v +10",
                     0, b"10  /tmp\n")
        self.compare("f(){ :; }; g(){ :; }; export -f f; readonly -f f; declare -ft f; export -fp",
                     0, b"f () \n{ \n    :\n}\ndeclare -frtx f\n", minimum=5)

    def test_bulk_listing_errors(self):
        commands = ("complete -F _f one; complete -p", "f(){ :; }; declare -f",
                    "f(){ :; }; export -f f; export -fp", "dirs -v",
                    "hash -p /bin/ls mark; hash", "trap : USR2; trap -p")
        for code in commands:
            with self.subTest(code=code):
                p = self.command(TONARI, code + " >&-")
                self.assertEqual((p.returncode, p.stdout), (1, b""))
                self.assertEqual(len(p.stderr.splitlines()), 1, p.stderr)
        self.compare("complete -p >&-; echo $?", 0, b"0\n")

    def test_listing_diagnostic_order(self):
        cases = [
            ("shopt -p nullglob __absent dotglob", b"shopt -u nullglob", b"shopt -u dotglob", 1),
            ("x=ok; declare -p x __absent x", b'declare -- x="ok"', b'declare -- x="ok"', 1),
            ("hash -p /bin/ls mark; hash -t mark __absent mark", b"mark\t/bin/ls", b"mark\t/bin/ls", 1),
            ("complete -W yes mark; complete -p mark __absent mark", b"complete -W 'yes' mark",
             b"complete -W 'yes' mark", 1),
            ("alias mark=yes; alias mark __absent mark", b"alias mark='yes'", b"alias mark='yes'", 1),
            ("type printf __absent pwd", b"printf is a shell builtin", b"pwd is a shell builtin", 1),
            ("command -V printf __absent pwd", b"printf is a shell builtin", b"pwd is a shell builtin", 0),
        ]
        for shell in (BASH, TONARI):
            for script, first, last, status in cases:
                with self.subTest(shell=shell, command=script):
                    proc = self.command(shell, "{ " + script + "; } 2>&1")
                    lines = proc.stdout.splitlines()
                    self.assertEqual((len(lines), proc.stderr), (3, b""))
                    self.assertEqual((lines[0], lines[2]), (first, last))
                    self.assertIn(b"__absent", lines[1])
                    if shell == BASH and script.startswith("type ") and self.command(BASH, 'echo "${BASH_VERSINFO[0]}"').stdout.strip() == b"3":
                        status = 0
                    self.assertEqual(proc.returncode, status)
            # declare -f skips a missing function silently but returns failure.
            proc = self.command(shell, "f(){ :; }; declare -f f __absent f 2>&1")
            self.assertEqual((proc.returncode, proc.stdout.count(b"f ()"), proc.stderr), (1, 2, b""))
            self.assertNotIn(b"__absent", proc.stdout)

    def test_literal_brace_products(self):
        source = r"""IFS=_; printf '<%s>\n' head p_{-02..02..2}_{c..a..2} {9..11}{0..1} end
n=0; printf '[%s]' {1..2}$((++n)){1..2}; echo "/$n" """
        self.compare(source, 0,
                     b"<head>\n<p_-02_c>\n<p_-02_a>\n<p_000_c>\n<p_000_a>\n<p_002_c>\n<p_002_a>\n"
                     b"<90>\n<91>\n<100>\n<101>\n<110>\n<111>\n<end>\n[111][122][231][242]/4\n", minimum=4)

    def test_indexed_listing_round_trip(self):
        source = r"""a=(); a[7]=''; a[9]=$'q"\n雪'; printf -v pad '%06000d' 0; a[2000]="$pad"
line=$(declare -p a); unset a; eval "$line"
printf '%s/%s/%s/%s' "${!a[*]}" "${a[7]}" "${a[9]}" "${#a[2000]}" """
        self.compare(source, 0, '7 9 2000//q"\n雪/6000'.encode())

    @unittest.skipUnless(sys.platform == "darwin", "Darwin file ACLs")
    def test_path_lookup_skips_acl_only_executable(self):
        first = self.cwd / "first"
        second = self.cwd / "second"
        for directory in (first, second):
            directory.mkdir()
            cmd = directory / "acl-command"
            cmd.write_text("#!/bin/sh\nprintf 'ran\\n'\n")
            cmd.chmod(0o700)
        cmd = first / "acl-command"
        cmd.chmod(0o600)
        import pwd
        subprocess.run(["chmod", "+a", pwd.getpwuid(os.getuid()).pw_name + " allow execute", cmd], check=True)
        self.assertTrue(os.access(cmd, os.X_OK))
        self.env["PATH"] = f"{first}:{second}"
        self.compare("set +h; type -P acl-command; acl-command", 0,
                     os.fsencode(second / "acl-command") + b"\nran\n")

    def test_command_cache_observes_permissions_and_new_names(self):
        directory = self.cwd / "bin"
        directory.mkdir()
        self.env["PATH"] = str(directory)
        cmd = directory / "probe-alpha"
        cmd.touch(mode=0o700)
        fd = self.terminal_session(editing=True)

        def complete(prefix, expected):
            os.write(fd, prefix + b"\t@")
            self.read_until(fd, b"@")
            os.write(fd, b"\x0c")
            self.read_until(fd, PROMPT + expected + b"@")
            os.write(fd, b"\x03")
            self.read_until(fd, PROMPT)

        complete(b"probe-al", b"probe-alpha ")
        cmd.chmod(0o600)
        complete(b"probe-al", b"probe-al")
        cmd.chmod(0o700)
        target = self.cwd / "target"
        (directory / "probe-link").symlink_to(target)
        complete(b"probe-li", b"probe-li")
        target.touch(mode=0o700)
        complete(b"probe-li", b"probe-link ")
        target.unlink()
        complete(b"probe-li", b"probe-li")
        if os.geteuid() != 0:
            directory.chmod(0o100)
            try:
                complete(b"probe-al", b"probe-al")
            finally:
                directory.chmod(0o700)
        cmd.rename(directory / "probe-beta")
        complete(b"probe-al", b"probe-al")
        complete(b"probe-be", b"probe-beta ")

    def test_function_source_after_changing_arg0(self):
        source = "shopt -s extdebug\na() { :; }\nBASH_ARGV0=changed\nb() { :; }\ndeclare -F a b"
        for shell in self.shells_for(minimum=5):
            with self.subTest(shell=shell):
                expected = (b"a 2 environment\nb 4 environment\n"
                            if shell == BASH and self.bash_before_53()
                            else b"a 2 initial\nb 4 changed\n")
                result = self.command(shell, source, "initial")
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, expected, b""))

    def test_path_lookup_reuses_overflow_storage(self):
        directory = Path(*(["x" * 150] * 4))
        (self.cwd / directory).mkdir(parents=True)
        command = self.cwd / directory / "pick"
        command.write_text("#!/bin/sh\necho found\n")
        command.chmod(0o700)
        self.env["PATH"] = f"{directory}/missing:absent:{directory}"
        self.compare("set +h; type -P pick; pick", 0, os.fsencode(directory / "pick") + b"\nfound\n")

    def test_declaration_failures_leave_scope_and_attributes_intact(self):
        source = """x=outer
f() { local a=ok; X=2 eval 'local -p a'; local -n x=123 2>/dev/null; x=changed; }
f; echo "$x"; readonly r=text; declare -a r 2>/dev/null; echo "$? $r"
declare -n ref; { ref+=(one); } 2>/dev/null; declare -p ref"""
        for shell in self.shells_for(minimum=5):
            result = self.command(shell, source)
            # Bash 5.2 allows declare -a to change a readonly scalar's type.
            readonly_status = b"0" if shell == BASH and self.bash_before_53() else b"1"
            expected = b'declare -- a="ok"\nchanged\n' + readonly_status + b' text\ndeclare -a ref=([0]="one")\n'
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, expected, b""))

    def test_posix_prefix_on_redirected_special_builtin(self):
        self.compare("""set -o posix; declare -i x=1
x=1+2 : >/dev/null; declare -p x""", 0, b'declare -ix x="3"\n', minimum=5)

    def test_read_count_utf8_leaves_next_record(self):
        for option in ("-n", "-N"):
            result = subprocess.run([TONARI, "-c",
                f'read -r {option} 1 x; a=$?; read -r y; printf "%s|%s" "$a" "$y"'],
                env=self.env, input=b"\xffnext\n", capture_output=True, timeout=3)
            self.assertEqual((result.returncode, result.stdout), (0, b"1|next"))

    def test_fc_latest_explicit_index_and_history_suffix(self):
        result = self.command(TONARI, """history -s first; history -s 'second tail'
fc -ln 2 2; history -p '!$-suffix'""")
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, b"\t second tail\ntail-suffix\n", b""))

    def test_printf_attached_and_repeated_variable_options(self):
        self.compare("""printf -vx '%s' one; printf -v x -vy -- '%s' two
printf '<%s><%s>' "$x" "$y"; printf -- '-v%s' ok""", 0, b"<one><two>-vok", minimum=5)

    def test_conditional_umask_uses_initial_permissions(self):
        script = """for initial in 077 777; do umask "$initial"; umask a+X && umask; done
umask 777; umask u+x,g+X && umask; umask a+st && umask"""
        result = self.command(TONARI, script)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, b"0066\n0777\n0677\n0677\n", b""))
        if not self.bash_before_53():
            self.compare(script, 0, result.stdout)

    def test_function_loop_control_follows_compat_level(self):
        for command in ("break", "continue"):
            code = f'f() {{ {command}; }}; for i in 1 2; do f; echo "$?:$i"; done'
            self.compare("set -o posix; " + code, 0, b"0:1\n0:2\n", minimum=5)
            self.compare("BASH_COMPAT=4.3; " + code, 0, b"", minimum=5)

    def test_repeated_arithmetic_reads_live_variable_bound(self):
        self.compare("""n=6; for ((i=0;i<n;i+=2)); do echo "$i"; ((n--)); done
n=4; declare -n bound=n; for ((i=4;i>=bound;i-=3)); do echo "$i"; done
for ((i=0;i<10;i+=010)); do echo "$i"; done""", 0, b"0\n2\n4\n0\n8\n", minimum=5)

    def test_history_trim_keeps_numbers_and_timestamp_pairs(self):
        source = """HISTSIZE=300; for ((i=0;i<1900;i++)); do history -s "row $i"; done
history -d 1602; history -w saved; history 1; head -n 2 saved; tail -n 1 saved"""
        self.compare(source, 0, b" 1899  row 1899\nrow 1600\nrow 1602\nrow 1899\n", minimum=5)

    def test_excessive_braces_fail_before_command_runs(self):
        for word in ("{1..5000000000}", "a{5000000000..1}", "{1..5000000000..2}x",
                     "{1..65536}{a,b}{1..9}", "{a,b}" * 21):
            result = self.command(TONARI, "touch sentinel " + word)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"brace expansion exceeds word limit", result.stderr)
            self.assertFalse((self.cwd / "sentinel").exists())

        padded = "{" + "0" * 2047 + "1..32769}"
        result = self.command(TONARI, "touch sentinel " + padded)
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"brace expansion exceeds byte limit", result.stderr)
        self.assertFalse((self.cwd / "sentinel").exists())

    def test_readonly_printf_count_signals_failure(self):
        result = self.command(TONARI, """readonly n=9; printf '%n' n 2>/dev/null; echo "$? $n" """)
        self.assertEqual((result.returncode, result.stdout), (0, b"1 9\n"))

    def test_clock_flags_use_shell_timezone_and_prompt_octets_form_utf8(self):
        source = r"""export TZ=Asia/Tokyo; printf '%(%-s|%z|%-z|%-Z)T\n' 0
p='\360\237\230\200'; echo "${p@P}" """
        result = self.command(TONARI, source)
        self.assertEqual(result.stdout.decode(), "0|+0900|+0900|JST\n😀\n")
        self.assertEqual((result.returncode, result.stderr), (0, b""))

    def test_flat_conditional_chains_keep_short_circuit_order(self):
        prefix = " && ".join(["1"] * 24)
        empty = " || ".join(["''"] * 24)
        self.compare(f"n=0; [[ {prefix} && '' && $((n++)) || {empty} || yes || $((n++)) ]]; echo $?:$n",
                     0, b"0:0\n")

    def test_regex_cache_tracks_shell_ctype(self):
        utf8 = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        source = f"""for LC_ALL in {utf8} C {utf8}; do
  [[ é =~ ^.$ ]]; echo $?; done"""
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"0\n1\n0\n", b""))

    def test_read_rejects_closed_descriptors_without_assigning(self):
        for fd in (9, 1200):
            for flags in ("-t 0", "-t 0.05", "-n 0", "-N 0", "-t 1 -a a"):
                src = f'v=old; a=(keep); read -u {fd} {flags} v; echo "$?:$v:${{a[*]}}"'
                for sh in self.shells_for(minimum=5):
                    result = self.command(sh, src)
                    self.assertEqual((result.returncode, result.stdout), (0, b"1:old:keep\n"))
                    self.assertTrue(result.stderr)

    def test_high_numbered_input_descriptor_observes_eof(self):
        soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
        if hard != resource.RLIM_INFINITY and hard <= 1200:
            self.skipTest("requires 1201 descriptors")
        with open(os.devnull, "rb") as source:
            try:
                if soft != resource.RLIM_INFINITY and soft <= 1200:
                    resource.setrlimit(resource.RLIMIT_NOFILE, (1201, hard))
                high = fcntl.fcntl(source, fcntl.F_DUPFD, 1200)
            finally:
                resource.setrlimit(resource.RLIMIT_NOFILE, (soft, hard))
            try:
                result = subprocess.run([TONARI, "-c",
                    f'v=keep; read -u {high} -t 0 v; echo "$?:$v"; read -u {high} -t 1 v; echo "$?:$v"'],
                    pass_fds=(high,), capture_output=True, timeout=3)
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"0:keep\n1:\n", b""))
            finally:
                os.close(high)

    def wait_for_input_consumed(self, fd):
        pending = array.array("i", [0])
        limit = time.monotonic() + 3
        while time.monotonic() < limit:
            fcntl.ioctl(fd, termios.FIONREAD, pending, True)
            if not pending[0]:
                return
            time.sleep(0.005)
        self.fail("reader did not consume the queued input")

    def test_function_limit_unwinds_nested_inputs(self):
        (self.cwd / "included").write_text("leaf; echo source-tail\necho another-line\n")
        for shell in self.shells_for(5):
            for nested in ("eval leaf", "source ./included", "mapfile -tc1 -C leaf a <<<'item'",
                           "trap 'leaf; echo trap-tail' USR1; kill -USR1 $$"):
                script = ("FUNCNEST=1; leaf(){ :; }; parent(){ " + nested +
                          "; echo inner-tail; }; parent; echo outer-tail\necho next:$?\n")
                for mode in ("-c", "file", "stdin"):
                    with self.subTest(shell=shell, command=nested, mode=mode):
                        path = self.cwd / "main"
                        path.write_text(script)
                        args = ["-c", script] if mode == "-c" else [path] if mode == "file" else []
                        result = subprocess.run([shell, *args], cwd=self.cwd, env=self.env,
                                                input=script.encode() if mode == "stdin" else b"",
                                                capture_output=True, timeout=3)
                        expected = (1, b"") if mode == "-c" else (0, b"next:1\n")
                        if shell == BASH and bash_version() < (5, 3):
                            prefix = b"another-line\n" if nested.startswith("source ") else b""
                            expected = (0, prefix + b"inner-tail\nouter-tail\nnext:0\n")
                        self.assertEqual((result.returncode, result.stdout), expected)
                        self.assertEqual(result.stderr.count(b"nesting"), 1)

    def test_terminal_interrupt_unwinds_loops_and_foreground_jobs(self):
        child = """/bin/sh -c 'echo run""READY; exec sleep 2'"""
        # Emit READY from inside the finite loop or the foreground child.
        cases = (
            ("for ((i=0;i<20000;i++)); do ((i)) || echo run''READY; :; done", False),
            (child, False),
            ("for i in one; do " + child + "; echo loop''TAIL; done", False),
            ("trap 'echo caught' INT; " + child, True),
        )
        for shell in self.shells_for(5):
            for command, continues in cases:
                with self.subTest(shell=shell, command=command):
                    fd = self.terminal_session(shell=shell)
                    os.write(fd, (command + "; echo line''TAIL\n").encode())
                    self.read_until(fd, b"runREADY\r\n")
                    os.write(fd, b"\x03")
                    reply = self.read_until(fd, PROMPT)
                    self.assertEqual(b"lineTAIL" in reply, continues)
                    self.assertNotIn(b"loopTAIL", reply)
                    os.write(fd, b"echo code=$?\n")
                    self.assertIn(b"code=" + (b"0" if continues else b"130") + b"\r\n",
                                  self.read_until(fd, PROMPT))
                    os.write(fd, b"/bin/sh -c 'exit 130'; echo normal=$?\n")
                    self.assertIn(b"normal=130\r\n", self.read_until(fd, PROMPT))

    def test_foreground_restop_keeps_job_order_and_listing_format(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell)
                os.write(fd, b"""/bin/sh -c 'kill -STOP $$; kill -STOP $$; exit 0' & p=$!; wait %1
""")
                self.read_until(fd, PROMPT)
                os.write(fd, b"sleep .8 & q=$!; fg %1; jobs > job-list\n")
                reply = self.read_until(fd, PROMPT)
                rows = (self.cwd / "job-list").read_bytes().splitlines()
                self.assertEqual([row[:3] for row in rows], [b"[1]", b"[2]"])
                self.assertIn(rows[0] + b"\r\n", reply)
                self.assertIn(b"Stopped", rows[0])
                os.write(fd, b'kill -CONT %1; wait "$p"; wait "$q"\n')
                self.read_until(fd, PROMPT)

    def test_interactive_wait_interrupt_with_inherited_hangup_ignore(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell, ignored=(signal.SIGHUP,))
                os.write(fd, b"sleep 1 & p=$!; echo wait''READY; wait; echo wait''TAIL\n")
                self.read_until(fd, b"waitREADY\r\n")
                os.write(fd, b"\x03")
                self.assertNotIn(b"waitTAIL", self.read_until(fd, PROMPT))
                os.write(fd, b'echo code=$?; wait "$p"\n')
                self.assertIn(b"code=130\r\n", self.read_until(fd, PROMPT))

    def test_terminal_interrupt_discards_input_command_list(self):
        cases = (("read -ru3 a", b"partial", b"old"),
                 ("mapfile -tu3 a", b"saved\npart", b"saved"),
                 ("mapfile -tu3 -n5 a", b"saved\npart", b"saved"))
        for shell in self.shells_for(5):
            for command, data, expected in cases:
                r, w = os.pipe()
                with self.subTest(shell=shell, command=command), os.fdopen(r, "rb") as reader, os.fdopen(w, "wb", buffering=0) as writer:
                    os.set_inheritable(r, True)
                    fd = self.terminal_session(shell=shell)
                    os.write(fd, f"a=old; exec 3<&{r}; echo input''READY; {command}; echo still''RUNNING\n".encode())
                    self.read_until(fd, b"inputREADY\r\n")
                    writer.write(data)
                    self.wait_for_input_consumed(reader.fileno())
                    os.write(fd, b"\x03")
                    reply = self.read_until(fd, PROMPT)
                    self.assertNotIn(b"stillRUNNING", reply)
                    self.assertNotIn(b"Interrupted system call", reply)
                    os.write(fd, b'echo "status=$? value=${a[*]}"\n')
                    self.assertIn(b"status=130 value=" + expected + b"\r\n", self.read_until(fd, PROMPT))

    def test_edited_read_resumes_after_a_caught_signal(self):
        for shell in self.shells_for(5):
            pid, fd = self.terminal_session(editing=True, shell=shell, with_pid=True)
            for sig in (signal.SIGUSR1, signal.SIGINT):
                with self.subTest(shell=shell, sig=sig):
                    command = f"""trap 'echo caught=$?' {sig.name[3:]}; false; read -e -p 'reading'INPUT value; echo "status=$? value=$value"\n"""
                    os.write(fd, command.encode())
                    self.read_until(fd, b"readingINPUT")
                    os.write(fd, b"hel")
                    self.read_until(fd, b"hel")
                    os.kill(pid, sig)
                    self.read_until(fd, b"caught=1\r\n")
                    os.write(fd, b"lo\n")
                    self.assertIn(b"status=0 value=hello\r\n", self.read_until(fd, PROMPT))

    def test_edited_read_resumes_split_terminal_packets(self):
        text = "日本🌱".encode()
        end = b"\x1b[201~"
        flower = "🌸".encode()
        cases = [(b"\x1b[200~" + text + end[:i], end[i:], "日本🌱") for i in range(len(end))]
        cases.extend((flower[:i], flower[i:], "🌸") for i in range(1, len(flower)))
        cases.extend([(b"\x1b[8", b"q", "bound"), (b"\x16", b"Q", "Q")])
        pid, fd = self.terminal_session(editing=True, with_pid=True)
        os.write(fd, b"""tty >ttyname; bind 'set keyseq-timeout 5000'; bind '"\\e[8q":"bound"'; echo tty''READY\n""")
        self.read_until(fd, b"ttyREADY\r\n")
        with open((self.cwd / "ttyname").read_text().strip(), "rb", buffering=0) as terminal:
            for prefix, suffix, expected in cases:
                with self.subTest(expected=expected):
                    os.write(fd, b"""trap 'echo caught=$?' USR1; false; read -e -p 'receiving'PACKET result; printf 'got=<%s>\\n' "$result"\n""")
                    self.read_until(fd, b"receivingPACKET")
                    os.write(fd, prefix)
                    self.wait_for_input_consumed(terminal.fileno())
                    os.kill(pid, signal.SIGUSR1)
                    self.read_until(fd, b"caught=1\r\n")
                    os.write(fd, suffix + b"\n")
                    self.read_until(fd, ("got=<" + expected + ">\r\n").encode())

    def test_partial_binding_observes_edited_read_timeout(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"""bind 'set keyseq-timeout 10000'; bind '"\\e[8q":"bound"'; read -e -t 0.4 -p 'timed'INPUT v; echo timedout=$?\n""")
        self.read_until(fd, b"timedINPUT")
        os.write(fd, b"\x1b[8")
        self.read_until(fd, b"timedout=142\r\n")

    def test_interrupting_edited_read_preserves_its_destination(self):
        for shell in self.shells_for(5):
            fd = self.terminal_session(shell=shell)
            os.write(fd, b"a=old; read -e -p 'typed'INPUT a; echo still''RUNNING\n")
            self.read_until(fd, b"typedINPUT")
            os.write(fd, b"part\x03")
            self.assertNotIn(b"stillRUNNING", self.read_until(fd, PROMPT))
            os.write(fd, b'echo "status=$? value=$a"\n')
            self.assertIn(b"status=130 value=old\r\n", self.read_until(fd, PROMPT))

    def test_mapfile_termination_runs_exit_trap_with_completed_records(self):
        for shell in self.shells_for(5):
            for opts in ("", "-n5"):
                code = f"""trap 'echo "exit=$?:${{a[*]}}"' EXIT; echo ready; :
mapfile -t {opts} a; echo continued"""
                r, w = os.pipe()
                with os.fdopen(r, "rb") as reader, os.fdopen(w, "wb", buffering=0) as writer, subprocess.Popen(
                    [shell, "-c", code], env=self.env, stdin=reader, stdout=subprocess.PIPE, stderr=subprocess.PIPE
                ) as child:
                    try:
                        self.assertEqual(child.stdout.readline(), b"ready\n")
                        writer.write(b"saved\npartial")
                        self.wait_for_input_consumed(reader.fileno())
                        child.terminate()
                        child.wait(timeout=3)
                        out, err = child.communicate()
                        self.assertEqual((child.returncode, out, err), (-signal.SIGTERM, b"exit=0:saved\n", b""))
                    finally:
                        if child.poll() is None:
                            child.kill()

    def test_read_dispatches_a_trap_before_resuming_partial_input(self):
        cases = [
            ("read -r value", b"hel", b"1", b"hello"),
            ("read -r -t 4 value", b"hel", b"1", b"hello"),
            ("mapfile -t value", b"one\nhel", b"1", b"one hello"),
            ("mapfile -tn2 value", b"one\nhel", b"1", b"one hello"),
            ("mapfile -t -C : -c 1 value", b"one\nhel", b"0", b"one hello"),
        ]
        for sh in self.shells_for(minimum=5):
            for command, prefix, last, result in cases:
                source = f"""trap 'echo caught:$?' USR1; echo ready; false
{command}; printf '%s:%s\\n' "$?" "${{value[*]}}" """
                read_fd, write_fd = os.pipe()
                with os.fdopen(read_fd, "rb") as observe, os.fdopen(write_fd, "wb", buffering=0) as writer, subprocess.Popen(
                    [sh, "-c", source], env=self.env, stdin=observe, stdout=subprocess.PIPE, stderr=subprocess.PIPE
                ) as child:
                    try:
                        self.assertEqual(child.stdout.readline(), b"ready\n")
                        writer.write(prefix)
                        self.wait_for_input_consumed(observe.fileno())
                        child.send_signal(signal.SIGUSR1)
                        self.assertTrue(select.select([child.stdout], [], [], 2)[0])
                        self.assertEqual(child.stdout.readline(), b"caught:" + last + b"\n")
                        writer.write(b"lo\n")
                        writer.close()
                        out, err = child.communicate(timeout=3)
                        self.assertEqual((child.returncode, out, err), (0, b"0:" + result + b"\n", b""))
                    finally:
                        if child.poll() is None:
                            child.kill()

    def test_read_honors_integer_destinations_in_input_order(self):
        self.compare("""declare -i REPLY; read <<< '3+4'; echo "$REPLY"
declare -ia a=(99); declare -n ref=a; i=0
read -ra ref <<< '2 a[0]+3 i+=4 010 -7'
printf '<%s>' "${a[@]}" "$i" """, 0, b"7\n<2><5><4><8><-7><4>", minimum=5)

    def test_mapfile_honors_array_attributes_in_each_input_mode(self):
        self.compare("""for mode in bulk origin limit callback; do
    declare -au a=(before); declare -ia n=(99); declare -n ref=n
    case $mode in bulk) opts=();; origin) opts=(-O 1);; limit) opts=(-n 2);; callback) opts=(-C : -c 1);; esac
    mapfile -t "${opts[@]}" a <<< $'ab\\nCd'
    mapfile -t "${opts[@]}" ref <<< $'2\\nn[0]+3'
    printf '<%s>' "${a[@]}" "${n[@]}"; echo
    unset -n ref; unset a n
done""", 0, b"<AB><CD><2><5>\n<BEFORE><AB><CD><99><2><102>\n<AB><CD><2><5>\n<AB><CD><2><5>\n", minimum=5)

    def test_integer_input_error_preserves_assigned_prefix(self):
        for setup, command in [
            ("declare -ai a=(9)", "read -a a <<< '2 1/0 7'"),
            ("declare -ai a=(9)", "mapfile -t a <<< $'2\\n1/0\\n7'"),
            ("declare -ai a=(2)", "read 'a[0]' <<< '1/0'"),
        ]:
            for shell in self.shells_for(minimum=5):
                result = self.command(shell, setup + "; trap 'echo \"${a[*]}\"' EXIT; " + command + "; echo skipped")
                self.assertEqual((result.returncode, result.stdout), (1, b"2\n"))
                self.assertTrue(result.stderr)

    def test_invalid_utf8_mapfile_keeps_completed_records(self):
        for opts in ("", "-n 8", "-O 0", "-C : -c 1"):
            src = f"mapfile -t {opts} a; echo \"$?:${{a[*]}}\""
            result = subprocess.run([TONARI, "-c", src], env=self.env,
                                    input=b"ok\n\xff\n", capture_output=True, timeout=3)
            self.assertEqual((result.returncode, result.stdout), (0, b"1:ok\n"))
            self.assertTrue(result.stderr)

    def test_exact_pipe_read_keeps_buffered_bytes_across_traps(self):
        payload = b"abcdefgh" * 65536
        source = """hits=0; trap '((++hits))' USR1; echo ready
IFS= read -N "$1" value; status=$?
printf '%d %d\\n' "$status" "$hits"; printf '%s' "$value" """
        with subprocess.Popen([TONARI, "-c", source, "test", str(len(payload))], env=self.env,
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
            self.assertEqual(child.stdout.readline(), b"ready\n")
            done = threading.Event()

            def signal_reader():
                while not done.is_set() and child.poll() is None:
                    child.send_signal(signal.SIGUSR1)
                    done.wait(0.001)

            sender = threading.Thread(target=signal_reader)
            sender.start()
            try:
                out, err = child.communicate(input=payload, timeout=10)
            finally:
                done.set()
                sender.join()
                if child.poll() is None:
                    child.kill()
            status, data = out.split(b"\n", 1)
            read_status, hits = map(int, status.split())
            self.assertGreater(hits, 0)
            self.assertEqual((child.returncode, read_status, err), (0, 0, b""))
            self.assertEqual(data, payload)

    def test_mapfile_leaves_pipe_suffix_for_other_readers(self):
        source = """exec 3<&0; trap : USR1
callback() { IFS= read -ru3 line; printf 'callback=<%s>\\n' "$line"; }
mapfile -tu3 -s1 -n2 -C callback -c2 a
printf '%s:<%s>:<%s>\\n' "$?" "${a[0]}" "${a[1]}"
cat <&3"""
        for sh in self.shells_for(minimum=5):
            result = subprocess.run([sh, "-c", source], env=self.env, capture_output=True,
                                    input="omit\n雪\n🌸\nconsumed\nleft\n".encode(), timeout=3)
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, "callback=<consumed>\n0:<雪>:<🌸>\nleft\n".encode(), b""))
        for opts in ("", "-n4", "-C : -c2"):
            src = f'trap : USR1; mapfile -t {opts} a; echo "$?:${{a[*]}}"; cat'
            result = subprocess.run([TONARI, "-c", src], env=self.env, capture_output=True,
                                    input=b"ok\nA\xff\nkeep\nlast\n", timeout=3)
            self.assertEqual((result.returncode, result.stdout), (0, b"1:ok\nkeep\nlast\n"))
            self.assertTrue(result.stderr)

    def test_for_select_and_getopts_use_declared_integer_destinations(self):
        self.compare("""declare -i x; for x in 2+3 010 x+1; do echo "$x"; done
select x in 3+4; do echo "$x"; break; done 2>/dev/null <<< 1
a=12; declare -i opt OPTARG; getopts a: opt -a2+3; echo "$?:$opt:$OPTARG"
declare -n ref=x; for ref in a; do echo "$ref"; done
select ref in 2+4; do echo "$a"; break; done 2>/dev/null <<< 1""",
                     0, b"5\n8\n9\n7\n0:12:5\n12\n2+4\n", minimum=5)

    def test_arithmetic_error_in_loop_or_getopts_stops_the_command_list(self):
        for code in [
            "for x in 2 1/0; do :; done",
            "select x in 1/0; do break; done <<< 1",
            "OPTIND=1; getopts a: y -a1/0",
        ]:
            for shell in self.shells_for(minimum=5):
                result = self.command(shell, "declare -i x OPTARG; " + code + "; echo skipped")
                self.assertEqual((result.returncode, result.stdout), (1, b""))
                self.assertTrue(result.stderr)

    def test_read_preserves_quoted_space_and_trims_ifs_whitespace(self):
        for ws in ("\r", "\v", "\f"):
            code = 'IFS="$1"; read -r x <<< "$1' + 'a$1$1' + 'b$1"; printf "%s" "$x"'
            got = self.command(TONARI, code, "test", ws)
            self.assertEqual((got.returncode, got.stdout, got.stderr), (0, ("a" + ws * 2 + "b").encode(), b""))
        got = self.command(TONARI, r"""IFS=' '; read x <<< 'a \ '; printf '<%s>' "$x" """)
        self.assertEqual((got.returncode, got.stdout, got.stderr), (0, b"<a  >", b""))

        record = self.cwd / "record"
        for space in (b" ", b"\t"):
            record.write_bytes(b" a  b\\" + space + b"  \n")
            for opts, expected in [
                ("x", b"<a  b" + space + b"><>"),
                ("x y", b"<a><b" + space + b">"),
                ("-r x", b"<a  b\\><>"),
                ("-r x y", b"<a><b\\>"),
            ]:
                with self.subTest(space=space, opts=opts):
                    got = self.command(TONARI, f'unset y; read {opts} <record && printf "<%s><%s>" "$x" "$y"')
                    self.assertEqual((got.returncode, got.stdout, got.stderr), (0, expected, b""))

    def test_umask_permission_copies_see_preceding_clauses(self):
        got = self.command(TONARI, "umask 022; umask u=rw,go=u; umask; umask 077; umask g=u,o+g; umask")
        self.assertEqual((got.returncode, got.stdout, got.stderr), (0, b"0111\n0000\n", b""))

    def test_mapfile_here_callback_observes_remaining_stdin(self):
        self.compare("""hook() { read -r more; echo "$1:$2:$more"; }
mapfile -t -c 1 -C hook r <<'END'
one
two
three
four
END
printf '%s ' "${r[@]}"; echo""", 0, b"0:one:two\n1:three:four\none three \n", minimum=5)

    def test_posix_field_splitting_uses_ifs_after_word_expansion(self):
        source = """set -o posix
set -f -- a 'b c' d
unset IFS
printf '<%s>' ${*}${IFS=}${*}${IFS:=-}"${*}"
"""
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, b"<a><b c><da><b c><d><a-b c-d>", b""))

    def test_default_assignment_evaluates_the_selected_element_once(self):
        source = """i=0; declare -ia a; declare -A m
key(){ printf x >>hits; printf id; }
printf '<%s>' "${a[i++]:=2+3}" "$i" "${a[0]}" "${m[$(key)]:=one}" "${m[id]}"; cat hits"""
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, b"<5><1><5><one><one>x", b""))

    def shells_for(self, minimum=0):
        if minimum and bash_version()[0] < minimum:
            return (TONARI,)
        return (BASH, TONARI)

    def bash_before_53(self):
        return bash_version() < (5, 3)

    def compare(self, source, code, stdout=b"", minimum=0):
        for shell in self.shells_for(minimum):
            with self.subTest(shell=shell, source=source):
                proc = self.command(shell, source)
                self.assertEqual(proc.returncode, code, proc.stderr)
                self.assertEqual(proc.stdout, stdout)
                if code == 0 and "missing-tonari" not in source:
                    self.assertEqual(proc.stderr, b"")

    def test_nested_temporary_variable_scopes(self):
        self.compare("""v=outer
unlocal() { unset -v v; }
f() {
    local v
    echo "$v"
    v=second eval 'local v=third; unlocal; echo "$v"; unlocal; echo "$v"'
}
v=first f
echo "$v" """, 0, b"first\nfirst\nouter\nouter\n", minimum=5)

    def test_dynamic_integers_and_ordinary_shadow(self):
        self.compare("""RANDOM=1; echo "$RANDOM $RANDOM"
SECONDS=12; ((SECONDS >= 12 && SECONDS < 15)) || exit 1
((SRANDOM >= 0 && SRANDOM <= 4294967295)) || exit 1
f() { local RANDOM=abc; echo "$RANDOM"; }
f; unset SECONDS; SECONDS=9; declare -p SECONDS""", 0,
                     b'16807 10791\nabc\ndeclare -- SECONDS="9"\n', minimum=5)

    def test_compound_redirect_failure_and_tested_context(self):
        self.compare("""set -e
{ :; } <missing-tonari || echo handled
{ :; } <missing-tonari
echo bad""", 1, b"handled\n")

    def test_export_snapshots_dynamic_integers(self):
        self.compare('RANDOM=1; export RANDOM; printenv RANDOM; printenv RANDOM; echo "$RANDOM"',
                     0, b"16807\n16807\n10791\n", minimum=5)

    def test_unrelated_local_survives_prefixed_eval(self):
        self.compare("""f() {
    IFS= eval 'local v=keep'
    printf '<%s>' "$v"
}
f; echo "<${v-unset}>" """, 0, b"<keep><unset>\n")

    def test_indirect_readonly_subscript_forms(self):
        self.compare("""a=(zero one); b=1
for ref in 'a[$b]' 'a[${c:-1}]' 'a[$((3-2))]'; do echo "${!ref}"; done""",
                     0, b"one\none\none\n", minimum=5)

    def test_indirect_subscript_reads_dynamic_operand_once(self):
        self.compare('a=(even odd); RANDOM=1; ref="a[RANDOM%2]"; echo "${!ref} $RANDOM"',
                     0, b"odd 10791\n", minimum=5)

    def test_compound_redirect_reports_failing_operator_line(self):
        proc = self.command(TONARI, """trap 'echo line=$LINENO' ERR
{ true
  true
} >first \\
  <missing-tonari
echo end""")
        self.assertEqual((proc.returncode, proc.stdout), (0, b"line=5\nend\n"), proc.stderr)
        self.assertTrue((self.cwd / "first").exists())

    def test_generated_ascii_range_keeps_each_character(self):
        expected = b"".join(b"<"+bytes([n])+b">" for n in range(ord("Z"), ord("a")+1))
        proc = self.command(TONARI, "printf '%s' '<'{Z..a}'>'")
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, expected, b""))

    def test_subshell_loop_control_reports_failure(self):
        proc = self.command(TONARI, """for i in 1; do
    (continue; echo unreachable)
    echo child=$?
    (for j in 1; do continue; done; echo inner)
done""")
        self.assertEqual((proc.returncode, proc.stdout), (0, b"child=1\ninner\n"))
        self.assertIn(b"subshell", proc.stderr)

    def test_loop_control_usage_error_recovers_at_input_boundary(self):
        script = "for x in a b; do\n echo $x\n continue 1 2\ndone\necho next\n"
        path = self.cwd / "commands"
        path.write_text(script)
        for shell in self.shells_for(5):
            command = self.command(shell, script)
            self.assertEqual((command.returncode, command.stdout), (1, b"a\n"))
            for args, data in (([str(path)], b""), ([], script.encode())):
                run = subprocess.run([shell, *args], input=data, cwd=self.cwd, env=self.env,
                                     capture_output=True, timeout=5)
                self.assertEqual((run.returncode, run.stdout), (0, b"a\nnext\n"), run.stderr)

    def test_trace_expands_conditional_operands_once(self):
        script = """exec 4>trace; BASH_XTRACEFD=4; set -x
dir=/; if [[ -d $dir ]]; then (( a = 42 )); fi
[[ $(echo once >>calls; echo yes) = yes ]] || exit 1
set +x; cat calls; echo "$a" """
        for shell in self.shells_for(5):
            (self.cwd / "calls").write_text("")
            proc = self.command(shell, script)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"once\n42\n"), proc.stderr)
            trace = (self.cwd / "trace").read_bytes()
            self.assertIn(b"+ [[ -d / ]]\n", trace)
            self.assertIn(b" a = 42 ", trace)

    def test_alias_in_a_source_array_subscript(self):
        for posix in (False, True):
            for late in (False, True):
                options = ("set -o posix\n" if posix else "") + "shopt -s expand_aliases\n"
                alias = "alias pick='echo 2'\n"
                source = options + "pick() { echo 3; }\n" + ("" if late else alias)
                source += """f() {
    a[$(pick)]=simple
    b=([$(pick)]=compound)
    declare -A m
    m[$(pick)]=key
    echo "${!a[@]} ${!b[@]} ${!m[@]}"
}
""" + (alias if late else "") + "f"
                index = "3" if posix and late else "2"
                self.compare(source, 0, (" ".join([index] * 3) + "\n").encode(), minimum=5)

    def test_alias_in_subscript_after_posix_mode_switch(self):
        self.compare("""set -o posix
shopt -s expand_aliases
alias picked='echo 2'
f() { a[$(picked)]=A; a[$(later)]=B; }
set +o posix
shopt -s expand_aliases
alias picked='echo 4' later='echo 3'
f; echo "${!a[@]}" """, 0, b"2 3\n", minimum=5)

    def test_function_parentheses_from_trailing_space_alias(self):
        self.compare(r"""shopt -s expand_aliases
alias g='g( ' q=')'
g q
{ echo G; }
\g""", 0, b"G\n")

    def test_incomplete_arithmetic_at_end_of_heredoc(self):
        self.compare("echo $((cat <<EOF\n+(\nEOF\n))", 2)

    def test_negated_bracket_pattern_with_closing_bracket_member(self):
        proc = self.command(TONARI, """pat='[^]]'; s='ab^]cd^'
printf '%s' "${s//$pat/z}" """)
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"zzz]zzz", b""))

    def test_pipeline_status_with_open_and_closed_reader(self):
        self.compare("""{ echo hi; exit 55; } | { cat >/dev/null; false; }
echo "${PIPESTATUS[*]}"
{ while printf '%04096d\\n' 1; do :; done; } | false
echo "${PIPESTATUS[*]}" """, 0, b"55 1\n141 1\n")

    def test_unquoted_punctuation_in_function_names(self):
        self.compare("""%() { :; }; declare -F '%'; unset '%'; declare -F '%'; echo gone=$?
a,b%() { echo comma; }; 'a,b%'
f?() { echo question; }; 'f?'
""", 0, b"%\ngone=1\ncomma\nquestion\n", minimum=5)
        for shell in self.shells_for(5):
            run = self.command(shell, "a,b%() { echo child; }; export -f 'a,b%'; \"$1\" -c 'a,b%'",
                               "parent", str(shell))
            self.assertEqual((run.returncode, run.stdout, run.stderr), (0, b"child\n", b""))

    def test_posix_function_name_validation_at_definition(self):
        for shell in self.shells_for(5):
            for name in ("a,b%", "eval"):
                with self.subTest(shell=shell, name=name):
                    definition = name + "() { echo replaced; }"
                    for prefix in ("", "register() { " + definition + "; }; register; "):
                        proc = self.command(shell, prefix + "set -o posix; " +
                                            ("register" if prefix else definition) + "; echo after")
                        if name == "a,b%" and shell == BASH and not self.bash_before_53():
                            # Bash 5.3 accepts punctuation as a POSIX extension.
                            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                             (0, b"after\n", b""))
                        else:
                            self.assertEqual((proc.returncode, proc.stdout), (2, b""))
                            self.assertTrue(proc.stderr)
                    skipped = self.command(shell, "set -o posix; if false; then " + definition +
                                           "; fi; echo ok")
                    self.assertEqual((skipped.returncode, skipped.stdout, skipped.stderr),
                                     (0, b"ok\n", b""))

    def test_case_attributes_on_arrays_and_export(self):
        self.compare("""declare -u s=one; s+=two; export s+=three; echo "$s"
declare -la a=(AA Bb); a[1]+=CC; echo "${a[@]}"
declare -uA m=([x]=Hi); m[x]+=bye; echo "${m[x]}"
declare +u s; s=Mixed; echo "$s" """, 0,
                     b"ONETWOTHREE\naa bbcc\nHIBYE\nMixed\n", minimum=5)

    def test_prompt_names_job_number_and_terminal_fallback(self):
        self.compare(r"""p='\l'; echo "${p@P}"
p='\j'; sleep 2 & echo "${p@P}" | cat; kill $!; wait $! 2>/dev/null || :
echo "${p@P}" """, 0, b"tty\n1\n0\n", minimum=5)

    def test_nonwhite_ifs_empty_parameter_boundaries(self):
        self.compare("""set -- '' aa '' ''; IFS=x
printf '<%s>' $@; echo; printf '<%s>' pre$*post; echo""", 0,
                     b"<> <aa> <>".replace(b" ", b"") + b"\n<pre><aa><><post>\n")

    def test_explicit_interactive_history_file(self):
        self.env["HISTFILE"] = str(self.cwd / "entries")
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                path = self.cwd / "entries"
                path.write_text("older\n")
                out = subprocess.run([shell, "--noprofile", "--norc", "-i"], cwd=self.cwd,
                    env=self.env, input=b"echo current\nhistory\n", capture_output=True, timeout=5)
                self.assertEqual(out.returncode, 0, out.stderr)
                self.assertIn(b"older", out.stdout)
                self.assertEqual(path.read_text(), "older\necho current\nhistory\n")

    def test_timestamped_history_import_append_and_write(self):
        (self.cwd / "entries").write_bytes(b"#100\nfirst\nsecond\n#120\nthird\n")
        self.compare(r"""HISTTIMEFORMAT='%s '; history -c; history -r entries
history; history -w copy; cat copy
""", 0, b"    1  100 first\nsecond\n    2  120 third\n#100\nfirst\nsecond\n#120\nthird\n",
                     minimum=5)
        out = self.command(TONARI, r"""HISTTIMEFORMAT='%s '; history -r entries
printf '#130\n' >>entries; history -n entries
printf 'next\n' >>entries; history -n entries; history -n entries; history 1
HISTFILE=entries; HISTFILESIZE=2; cat entries""")
        self.assertEqual((out.returncode, out.stdout, out.stderr), (0,
            b"    3  130 next\n#120\nthird\n#130\nnext\n", b""))

    def test_history_limit_preserves_markers_at_block_boundaries(self):
        prefix = b"#100\nfirst\nline\n"
        for tail_size in (16382, 16383, 16384, 16385, 32768):
            with self.subTest(tail_size=tail_size):
                last = b"#200\n" + b"x" * tail_size + b"\nlast\n#300\n"
                (self.cwd / "entries").write_bytes(prefix + last)
                result = self.command(TONARI, "HISTFILE=entries; HISTFILESIZE=1")
                self.assertEqual((result.returncode, result.stderr), (0, b""))
                self.assertEqual((self.cwd / "entries").read_bytes(), last)

    def test_history_journal_retains_times_and_reads_older_rows(self):
        self.env.update(HISTFILE=str(self.cwd / ".bash_history"), HISTTIMEFORMAT="%s ",
                        XDG_STATE_HOME=str(self.cwd / "state"))
        journal = self.cwd / "state/tonari/history"
        journal.parent.mkdir(parents=True)
        journal.write_text("1\t42\t0\t/tmp\tlegacy\n2\t123\t43\t0\t/tmp\tremembered\n")
        result = self.command(TONARI, "history -r; history")
        self.assertEqual((result.returncode, result.stderr, result.stdout),
                         (0, b"", b"    1  ??legacy\n    2  123 remembered\n"))
        fd = self.terminal_session()
        os.write(fd, b"echo record-epoch >marker\r")
        self.read_until(fd, PROMPT)
        entry = [s for s in journal.read_text().splitlines() if s.endswith("echo record-epoch >marker")]
        self.assertEqual(len(entry), 1)
        fields = entry[0].split("\t")
        self.assertEqual(fields[0], "2")
        self.assertLess(abs(int(fields[1]) - time.time()), 30)
        result = self.command(TONARI, "history -r; history 1")
        self.assertIn((fields[1] + " echo record-epoch >marker\n").encode(), result.stdout)

    def test_printf_time_locale_zone_and_arguments(self):
        self.compare(r"""export TZ=UTC0
printf '[%*.*(%Y-%m-%d)T] %()T %((%Y))T\n' 7 4 0 0 0
printf -v t '%(%F %T %Z %z %s %%s)T;' 0 86400
echo "$t"
TZ=America/New_York printf '%(%F %T %Z %z)T\n' 0 1593604800
printf '%(%Z %z)T\n' 0
""", 0, b"[   1970] 00:00:00 (1970)\n1970-01-01 00:00:00 UTC +0000 0 %s;"
        b"1970-01-02 00:00:00 UTC +0000 86400 %s;\n1969-12-31 19:00:00 EST -0500\n"
        b"2020-07-01 08:00:00 EDT -0400\nUTC +0000\n", minimum=5)

    def test_printf_time_unexported_unset_and_clock(self):
        self.compare(r"""unset TZ
printf -v local '%(%F %T %z)T' 0
TZ=HST10
printf -v t '%(%F %T %z)T' 0
[[ $t == "$local" ]] || exit 1
export TZ
printf '%(%F %T %z)T\n' 0
unset TZ
printf -v t '%(%F %T %z)T' 0
[[ $t == "$local" ]] || exit 2
printf -v start '%(%s)T' -2
sleep 1
printf -v now '%(%s)T'
printf -v explicit '%(%s)T' -1
printf -v again '%(%s)T' -2
(( start == again && now > start && explicit >= now ))
""", 0, b"1969-12-31 14:00:00 -1000\n", minimum=5)

    def test_printf_regional_dates_and_locale_changes(self):
        available = subprocess.check_output(["locale", "-a"], text=True).lower().replace("-", "")
        locales = [name for name in ("de_DE", "ja_JP", "fr_FR", "pl_PL", "fa_IR")
                   if (name.lower() + ".utf8") in available.splitlines()]
        if not locales or len(self.shells_for(5)) < 2:
            self.skipTest("a named UTF-8 locale and GNU Bash 5 are required")
        script = r"""export TZ=UTC0
unset LC_ALL
for lang; do
  LC_TIME=$lang.UTF-8
  printf '%(%A|%B)T|%(%x)T|%(%X)T|%(%r)T|%(%%A|%Z|%z|%s)T\n' 0 0 0 0 0
  case $lang in de_DE|ja_JP) printf '%(%c)T\n' 0;; esac
  LC_ALL=C printf '%(%A|%B)T\n' 0
done
"""
        baseline = self.command(BASH, script, "_", *locales)
        self.assertEqual((baseline.returncode, baseline.stderr), (0, b""))
        if "de_DE" in locales:
            self.assertIn(b"Donnerstag|Januar|", baseline.stdout)
        if "ja_JP" in locales:
            self.assertIn("木曜日|1月|".encode(), baseline.stdout)
        result = self.command(TONARI, script, "_", *locales)
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, baseline.stdout, b""))
        if sys.platform.startswith("linux") and "ja_JP" in locales:
            self.env["LC_ALL"] = "de_DE.UTF-8" if "de_DE" in locales else "C"
            if "de_DE" in locales:
                self.compare("TZ=UTC0 printf '%(%^B|%10B|%#A)T' 0",
                             0, b"JANUAR|    Januar|DONNERSTAG", minimum=5)
            self.env["LC_ALL"] = "ja_JP.UTF-8"
            self.compare("export TZ=UTC0; printf '%(%EC|%Ey|%EY|%Od)T\\n' 0 1556668800 1700000000",
                         0, "昭和|45|昭和45年|一\n令和|01|令和元年|一\n令和|05|令和05年|十四\n".encode(),
                         minimum=5)

    @unittest.skipUnless(sys.platform.startswith("linux"), "GNU strftime flag semantics")
    def test_printf_time_flags_and_unknown_directives(self):
        self.compare("export TZ=UTC0; printf '%(%^B|%#A|%#p|%#Z|%10B|%05a|%-d|%_d|%0e|%10d|%k|%l|%OY|%Q|%%)T' 0",
                     0, b"JANUARY|THURSDAY|am|utc|   January|00Thu|1| 1|01|0000000001| 0|12|%OY|%Q|%", minimum=5)

    def test_printf_long_time_format(self):
        fmt = "%(" + "%Y" * 1000 + ")T"
        p = self.command(TONARI, 'TZ=UTC0 printf "$1" 0', '_', fmt)
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"1970" * 1000, b""))

    def test_assignment_tildes_and_brace_login_names(self):
        import pwd
        root = os.fsencode(pwd.getpwnam("root").pw_dir)
        self.env["HOME"] = "/home/test"
        self.compare(r"""
v=${u-~:${u-~:~}}
printf '%s\n' "$v" ${u-~:~} "${u-~:~}"
v=${u-a\:~:~}; echo "$v"
printf '%s\n' ~{ro,"ro"}ot {~root,~}/a ~{root,}/x
""", 0, b"/home/test:/home/test:/home/test\n/home/test:~\n~:~\na:~:/home/test\n"
        + root + b"\n~root\n" + root + b"/a\n/home/test/a\n" + root + b"/x\n/home/test/x\n")

    def test_array_assignment_and_argv_contexts(self):
        self.compare(r"""i=3
set -f
a=([1 + 2]=abc [i + 1]=def)
printf '<%s>\n' a[1 + 2]=xxx b[$i]=y "${a[@]}"
{ a[1 + 2]=x true; } 2>/dev/null
echo "$?:${a[3]}"
v=word
a=(hi "$v" [2]="$v" ~ *.txt {a,b}) env | grep '^a='
""", 0, b"<a[1>\n<+>\n<2]=xxx>\n<b[3]=y>\n<abc>\n<def>\n0:abc\n"
        b"a=(hi word [2]=word ~ *.txt {a,b})\n")

    def test_legacy_arithmetic_and_set_options(self):
        self.compare(r"""i=2; a[3]=7
echo "$[a[i+1] * $i]"
set -- old
set +
echo "$#:$*"
set -x -v - x + y
case $- in *x*|*v*) exit 1;; esac
echo "$#:$*"
set -a
f() { local TONARI_LOCAL=value; env | grep '^TONARI_LOCAL='; }
f
declare TONARI_GLOBAL=value
env | grep '^TONARI_GLOBAL='
""", 0, b"14\n1:old\n3:x + y\nTONARI_LOCAL=value\nTONARI_GLOBAL=value\n")

    def test_read_timeouts_on_files_and_invalid_options(self):
        self.compare(r"""read -t 0 </dev/null; echo $?
read -t 0.5 </dev/null; echo $?
for args in '-t -1' '-n x' '-N -1' '-u -3' '-u x' '-k'; do
    read $args </dev/null 2>/dev/null; echo $?
done
""", 0, b"0\n1\n1\n1\n1\n1\n1\n2\n", minimum=5)

    def test_set_prints_reusable_scalars_and_arrays(self):
        self.compare(r"""__plain=yes
__words='a b'
__new=$'a\nb'
__empty=''
__array=([3]=abc [9]='two words')
set | grep '^__' >vars
cat vars
unset __plain __words __new __empty __array
. ./vars
[[ $__plain == yes && $__words == 'a b' && $__new == $'a\nb' && -v __empty && ${__array[9]} == 'two words' ]]
""", 0, b'__array=([3]="abc" [9]="two words")\n__empty=\n__new=$\'a\\nb\'\n'
        b"__plain=yes\n__words='a b'\n", minimum=5)

    def test_shell_option_queries_echo_and_subscripts(self):
        self.compare(r"""test -o nounset; echo $?
set -u; [[ -o nounset ]]; echo $?
[ -z -a -a ]; echo $?
shopt -s xpg_echo
echo 'a\tb'
v=$(echo 'x\ty'); printf '<%s>\n' "$v"
set -o posix; echo -E 'a\tb'
""", 0, b"1\n0\n0\na\tb\n<x\ty>\n-E a\tb\n", minimum=5)
        self.compare(r"""a=(old)
{ a[]=oops; } 2>/dev/null
echo "$?:${a[0]}"
readonly 'b[2]=bad' 2>/dev/null; echo "$?:${#b[@]}"
""", 0, b"1:old\n1:0\n", minimum=5)

    def test_runtime_home_drives_tilde(self):
        p = self.command(TONARI, 'HOME=/changed; v=${u-~:~}; printf "%s" "$v"')
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"/changed:/changed", b""))

    def test_array_parameter_defaults_preserve_fields(self):
        self.compare(r"""show() { printf '%s' "$#"; for x; do printf '<%s>' "$x"; done; echo; }
a=()
show "${a[@]-empty}" "${a[*]:-empty}"
show "${a[@]+ignored}"
a=('' '' 'two words')
show "${a[@]:-other}" "${a[@]:+present}"
ref='a[@]'; show "${!ref:-other}"
IFS=; a=('' '')
show "${a[@]:-other}" "${a[*]:-other}"
IFS=:
a=('' y); show ${a[@]:-other}
declare -A indirect=([k]=v)
v=blue; show "${!indirect[@]#b}" "${!indirect[@]:-other}"
""", 0, b"2<empty><empty>\n0\n4<><><two words><present>\n3<><><two words>\n"
        b"3<><><other>\n2<><y>\n2<lue><blue>\n", minimum=5)

    def test_compgen_fallback_keywords_and_filters(self):
        (self.cwd / "sp-dir").mkdir()
        (self.cwd / "spam.py").touch()
        self.compare(r"""complete -F missing 2>/dev/null; echo $?
compgen -A command whil
compgen -k | grep '^time$'
compgen -o default sp | sort
compgen -o plusdirs -W 'space' sp | sort
compgen -o dirnames sp
""", 0, b"2\nwhile\ntime\nsp-dir\nspam.py\nsp-dir\nspace\nsp-dir\n", minimum=5)
        source = r"""shopt -s extglob; compgen -f -X "!*.@(py)" -- sp"""
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                p = subprocess.run([shell, "--norc", "-ic", source], cwd=self.cwd,
                                   env=self.env, capture_output=True, timeout=5)
                self.assertEqual((p.returncode, p.stdout), (0, b"spam.py\n"), p.stderr)

    def test_pipeline_last_argument_and_parent_scope(self):
        self.compare(""": before
f() { echo "inside:$_"; : changed; }
f arg final
echo "after:$_"
: old
printf x | cat >/dev/null
echo "pipe:$_"
shopt -s lastpipe
printf 'joined\\n' | read -r value
echo "$value:${PIPESTATUS[*]}"
f() { printf x | return 23; echo unreachable; }
f
echo $?
""", 0, b"inside:before\nafter:final\npipe:old\njoined:0 0\n23\n", minimum=5)

    def test_array_initialization_and_attributes(self):
        self.compare("""i=1
a=([100+i++]=$((i++)) [200+i++]=$((i++)))
echo "${!a[*]}:${a[*]}:$i"
a=([0]=3 [a[0]]=7 [a[3]]=new [7]+=er)
echo "${!a[*]}:${a[*]}"
declare -ai attr=([7]=123 [9]=321)
printf '<%s>' "${attr[@]@a}"
prefB=2 prefA=1; printf '<%s>' "${!pref@}"
""", 0, b"103 204:1 2:5\n0 3 7:3 7 newer\n<ai><ai><prefA><prefB>", minimum=5)
        # An element append uses the value already assigned to the same key.
        # Some Bash 5.2 builds lose this value in an associative initializer.
        proc = self.command(TONARI, 'declare -A a=([x]=first [x]+=second); echo "${a[x]}"')
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"firstsecond\n", b""))
        self.compare("""a=(x y); i=0; unset -v 'a[i++]'; echo "$i:${a[*]}"
declare v; unset -v 'v[2]' 2>/dev/null; echo "$?"
history -Z 2>/dev/null; echo "$?"
""", 0, b"1:y\n1\n2\n", minimum=5)

    def test_vi_operator_edits_submitted_argument(self):
        for shell in self.shells_for():
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"set -o vi; s(){ printf '<%s>' \"$@\" >vi-result; echo edit''OK; }; echo rea''dy\r")
                self.read_until(fd, b"ready\r\n")
                os.write(fd, b"s hello world")
                self.read_until(fd, b"world")
                os.write(fd, b"\x1b")
                time.sleep(0.65)
                os.write(fd, b"0wcwgoodbye\r")
                self.read_until(fd, b"editOK\r\n")
                self.assertEqual((self.cwd / "vi-result").read_bytes(), b"<goodbye><world>")

    def test_right_leaves_reverse_search_with_an_editable_match(self):
        for shell in self.shells_for():
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"echo editme >match; echo rea''dy\r")
                data = self.read_until(fd, b"ready\r\n")
                if PROMPT not in data.partition(b"ready\r\n")[2]:
                    self.read_until(fd, PROMPT)
                os.write(fd, b"\x12editme")
                self.read_until(fd, b">match")
                os.write(fd, b"\x1b[C\x05; echo changed >> match; echo done''OK\r")
                self.read_until(fd, b"doneOK\r\n")
                self.assertEqual((self.cwd / "match").read_text(), "editme\nchanged\n")
                (self.cwd / "match").unlink()

    def test_arithmetic_keeps_associative_keys_as_data(self):
        self.compare(r"""declare -A A
for key in '\' 'a]b' "x'y" '雪' '$anything'; do
  (( A["$key"] = 42 ))
  (( A["$key"] += 1 ))
  printf '<%s:%s>\n' "$key" "${A[$key]}"
done
i=0; ((0 && A["$((++i))"])); echo "$i"
(( A["$((i++))"] = $((i++)) )); echo "${A[1]}:$i"
""", 0, "<\\:43>\n<a]b:43>\n<x'y:43>\n<雪:43>\n<$anything:43>\n1\n2:3\n".encode(), minimum=5)

    def test_signal_listing_continues_past_invalid_operands(self):
        self.compare("kill -l HUP BAD INT 9999 129 2>/dev/null; echo $?", 0, b"1\n2\nHUP\n1\n")

    def test_regex_tilde_and_unquoted_operators(self):
        # Start both shells with this HOME. Some Bash builds cache tilde's
        # original value even after the HOME variable changes.
        self.env["HOME"] = str(self.cwd / "^a$")
        self.compare("""[[ $HOME =~ ~ ]]; echo "$?"
[[ "$HOME/a" =~ ~/[ab] ]]; echo "$?"
[[ '~' =~ "~" ]]; echo "$?"
[[ '&' =~ (&) ]]; echo "$?"
""", 0, b"0\n0\n0\n0\n")
        p = self.command(TONARI, 'HOME=/changed; printf "%s" ~; [[ "$HOME" =~ ~ ]]')
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"/changed", b""))
        for shell in self.shells_for():
            for operator in ("&", "<>"):
                proc = self.command(shell, "[[ x =~ " + operator + " ]]; echo unexpected")
                self.assertEqual((proc.returncode, proc.stdout), (2, b""))

    def test_unset_removes_a_posix_special_builtin_assignment(self):
        p = self.command(TONARI, """set -o posix
f() { x=42; x=tmp unset x; echo "${x-unset}"; }
f
x=42; x=tmp eval 'unset x'; echo "${x-unset}"
x=42; x=tmp :; echo "$x"
""")
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"unset\nunset\ntmp\n", b""))

    def test_umask_applies_symbolic_clauses_in_order(self):
        proc = self.command(TONARI, "umask 0124; umask a=,a=u && umask")
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"0777\n", b""))

    def test_quoted_array_declarations(self):
        self.compare("""false
declare -a 'arr=($? "two words")'
declare -A 'map=([a]=b [c]=d)'
printf '<%s>' "${arr[@]}" "${map[a]}" "${map[c]}"
declare 'arr[0]=(literal)'
echo "${arr[0]}"
""", 0, b"<1><two words><b><d>(literal)\n", minimum=5)

    def test_history_limits_and_incremental_import_after_copy(self):
        self.compare("""printf '%s\\n' one two three > first; cp first second; echo four >>second
history -c; HISTFILE=first history -r; HISTFILE=second history -n
history | awk '{print $NF}'
HISTSIZE=2; history | awk '{print $NF}'
HISTFILE=second; HISTFILESIZE=2; cat second
""", 0, b"one\ntwo\nthree\nfour\nthree\nfour\nthree\nfour\n", minimum=5)
        # Replacing a history file with unrelated content must keep its first line.
        proc = self.command(TONARI, "printf 'one\\n' >a; printf 'six\\nnext\\n' >b; "
                                    "history -r a; history -n b; history | awk '{print $NF}'")
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"one\nsix\nnext\n", b""))

    def test_completion_actions_survive_print_and_reload(self):
        actions = ("alias arrayvar binding builtin command directory disabled enabled export "
                   "file function group helptopic hostname job keyword running service setopt "
                   "shopt signal stopped user variable")
        source = """complete $(printf -- '-A %s ' """ + actions + """) probe
line=$(complete -p probe); complete -r probe; eval "$line"; complete -p probe"""
        reference = self.command(BASH, source)
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, reference.stdout, b""))
        self.assertIn(b"-A binding", result.stdout)

    def test_mapfile_callback_readonly_names_invoked_builtin(self):
        for name in ("mapfile", "readarray"):
            result = self.command(TONARI, "lock(){ readonly -a ary; }; "
                                  + name + " -tc1 -C lock ary <<<one")
            self.assertEqual((result.returncode, result.stderr),
                             (1, f"tonari: {name}: ary: readonly variable\n".encode()))

    def test_debug_wait_preserves_child_status(self):
        self.compare("trap ':' DEBUG; (exit 37) & wait $!; echo $?", 0, b"37\n")

    def test_reassign_history_limits(self):
        self.compare("""HISTSIZE=10; HISTFILESIZE=10; HISTFILE=hist
printf '%s\\n' one two three >hist; history -r hist
HISTSIZE=1; history | awk '{ print $NF }'
HISTFILESIZE=1; cat hist""", 0, b"three\nthree\n", minimum=5)

    def test_invoked_as_sh(self):
        sh = self.cwd / "sh"
        env = self.env | {"BASH_ENV": str(self.cwd / "bashenv"), "ENV": str(self.cwd / "env")}
        (self.cwd / "bashenv").write_text("echo bashenv\n")
        (self.cwd / "env").write_text('shopt -oq posix; echo "env:$?"\n')
        # Bash 3.2 on macOS exits on a failing "command ." in sh mode.
        for binary in self.shells_for(minimum=5):
            sh.unlink(missing_ok=True)
            sh.symlink_to(binary)
            def run(args, script=None):
                return subprocess.run([str(sh), *args], cwd=self.cwd, env=env, input=script,
                                      capture_output=True, timeout=5)
            with self.subTest(shell=binary):
                proc = run(["-c", "alias x='echo yes'\nx\nv=one :; z=two command :; echo \"$0/$v/${z-_}\"", "custom"])
                self.assertEqual((proc.returncode, proc.stdout), (0, b"yes\ncustom/one/_\n"))
                proc = run(["-i", "+m"], b"fi\necho good\nread -r line\nchild data\necho \"$line\"\n")
                self.assertEqual((proc.returncode, proc.stdout), (0, b"env:1\ngood\nchild data\n"))
                proc = run(["-c", "command . ./missing; echo ok; : <./missing; echo bad"])
                self.assertNotEqual(proc.returncode, 0)
                self.assertEqual(proc.stdout, b"ok\n")
                proc = run(["-c", "3</dev/null >&3"])
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"", b""))
                if binary == TONARI:
                    proc = run(["-c", 'unset v\n<<EOF\n${v=inner}\nEOF\necho "${v-unset}"'])
                    self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"unset\n", b""))
                self.assertNotEqual(run(["-c", "echo >&-"]).returncode, 0)
                self.assertEqual(run(["-c", "kill -s PIPE $$; echo bad"]).returncode, -signal.SIGPIPE)
                proc = run(["-c", "trap '' PIPE; \"$0\" -c 'kill -s PIPE $$; echo ok'"])
                self.assertEqual((proc.returncode, proc.stdout), (0, b"ok\n"))

                proc = run(["-c", "alias blank=; alias e=echo\n{ e first | blank\ncat; }\nf() { return -- 17; }; f; e $?; cd -P -e .; exit -- 56"])
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (56, b"first\n17\n", b""))
                proc = run(["-c", "alias e='echo '; alias a=aa; alias aa=inner\n"
                            "{\ne a world\ne test\n}\nalias both='e one\ne two'\nboth\n"])
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                 (0, b"inner world\ntest\none\ntwo\n", b""))
                proc = run(["-i", "+m", "-c", "shopt -oq posix"])
                self.assertEqual((proc.returncode, proc.stdout), (0, b"env:1\n"))
                proc = subprocess.run([str(sh), "-i", "+m", "-c",
                                       "trap - USR1; kill -s USR1 $$; echo bad"], env=env,
                                      preexec_fn=lambda: signal.signal(signal.SIGUSR1, signal.SIG_IGN),
                                      stdin=subprocess.DEVNULL, capture_output=True, timeout=5)
                self.assertEqual((proc.returncode, proc.stdout), (-signal.SIGUSR1, b"env:1\n"))

    def test_posix_redirections_precede_prefix_assignments(self):
        sh = self.cwd / "sh"
        sh.symlink_to(TONARI)
        script = r"""show() { printf '<%s>:%s\n' "$v" "$1"; }
v=outer
printf 'input\n' >source
v=$(cat <&5) 5<source show "$(test -e created || echo early)" >created
cat created
v=$(cat <&5) 5<source /usr/bin/env | grep '^v='
v=$(cat <&5) 5<source show bg >background & wait
cat background
v=$(printf z >&4) 4>once true
cat once; echo
v=$(echo wrong >side-effect) true <absent
echo "failed:$?"
test ! -e side-effect || exit 31
test ! -e readonly-file || exit 32
(readonly x; x=bad true >readonly-file)
test -e readonly-file || exit 33
echo "restored:$v"
"""
        p = self.command(sh, script)
        self.assertEqual((p.returncode, p.stdout), (0, b"<input>:early\nv=input\n<input>:bg\nz\nfailed:1\nrestored:outer\n"), p.stderr)
        self.assertIn(b"absent", p.stderr)
        self.assertIn(b"readonly", p.stderr)
        self.assertNotIn(b"invalid file descriptor", p.stderr)
        p = self.command(sh, 'unset red; x=ok /usr/bin/true >"${red:=external}"; '
                             'echo "${red-unset}"; x=ok true >"${red:=builtin}"; echo "$red"')
        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"unset\nbuiltin\n", b""))
        p = self.command(sh, 'unset red; x=ok true >"${red:?missing}"; echo unreachable')
        self.assertNotEqual(p.returncode, 0)
        self.assertEqual(p.stdout, b"")
        self.assertIn(b"missing", p.stderr)
        # Bash mode reads the right-hand side before truncating an existing file.
        for shell in (BASH, TONARI):
            p = self.command(shell, "printf old >same; take() { echo \"$v\"; }; v=$(cat same) 3>same take")
            self.assertEqual((p.returncode, p.stdout, p.stderr), (0, b"old\n", b""))

    def test_posix_test_tty_operands(self):
        sh = self.cwd / "sh"
        master, slave = pty.openpty()
        try:
            for shell in (BASH, TONARI):
                sh.unlink(missing_ok=True)
                sh.symlink_to(shell)
                script = """for value in '' x -10; do test -t "$value"; echo $?; done
test -t 0 0<&-; echo $?; test -t 0 </dev/null; echo $?; test -t 0; echo $?
test -t 5 5<&-; echo $?; test -t 5 5</dev/null; echo $?; test -t 5 5<&0; echo $?"""
                p = subprocess.run([sh, "-c", script], stdin=slave, cwd=self.cwd,
                                   env=self.env, capture_output=True, timeout=5)
                if shell == BASH and not self.bash_before_53() and p.stdout.startswith(b"2\n2\n"):
                    # Bash 5.3 diagnoses empty and nonnumeric operands. POSIX
                    # specifies false for an invalid file descriptor number.
                    self.assertEqual((p.returncode, p.stdout), (0, b"2\n2\n1\n1\n1\n0\n1\n1\n0\n"))
                    self.assertEqual(p.stderr.count(b"integer expected"), 2)
                    continue
                self.assertEqual((p.returncode, p.stdout, p.stderr),
                                 (0, b"1\n1\n1\n1\n1\n0\n1\n1\n0\n", b""))
        finally:
            os.close(slave)
            os.close(master)

    def test_seekable_input_assignment_and_child_offsets(self):
        script = self.cwd / "input"
        script.write_text("i=one\nx=old\nx=$i\nread -r line\npayload for read\n"
                          "i=two\nx=$i\nprintf '<%s><%s>\\n' \"$x\" \"$line\"\n"
                          "i=three\nx=$i\ncat\nremaining | bytes\n")
        for binary in (BASH, TONARI):
            sh = self.cwd / "sh"
            sh.unlink(missing_ok=True)
            sh.symlink_to(binary)
            with script.open("rb") as stdin:
                proc = subprocess.run([sh], stdin=stdin, cwd=self.cwd, env=self.env,
                                      capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, b"<two><payload for read>\nremaining | bytes\n", b""))

    def test_continuations_and_literal_command_cache(self):
        # Physical newlines work in parameter names and operators. Escaped
        # backslashes and quoted here-document bodies retain their bytes.
        script = "name=works\nprintf '<%s>\\n' \"$na\\\nme\" \"${na\\\nme:\\\n+yes}\" " \
                 "\"$name\\\\\nend\"\ncat <<'EOF'\nstring\\\ncontinued\nEOF\n"
        self.compare(script, 0, b"<works>\n<yes>\n<works\\\nend>\nstring\\\ncontinued\n")
        self.compare("foobar=value\nfoo=wrong\nprintf '%s' $foo\\\nbar\n", 0, b"value")
        script_file = self.cwd / "external"
        script_file.write_text("printf '%s/%s\\n' \"$setting\" \"$1\"\n")
        script_file.chmod(0o755)
        self.compare("export setting; for setting in one two; do " +
                     shlex.quote(str(script_file)) + " 'a b'; done",
                     0, b"one/a b\ntwo/a b\n")
        self.compare("set -n; while :; do echo wrong; done; echo wrong", 0)

    def test_posix_declarations_and_error_exit(self):
        for binary in self.shells_for(5):
            sh = self.cwd / "sh"
            sh.unlink(missing_ok=True)
            sh.symlink_to(binary)
            result = self.command(sh, "set -- 'one two'; command command export val=$1; "
                                  "printf '[%s]\\n' \"$val\"; readonly val; export val=other; echo wrong")
            self.assertEqual(result.stdout, b"[one two]\n")
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(result.stderr)

    def test_notification_wait_honors_prompt_timeout(self):
        for editing in (False, True):
            with self.subTest(editing=editing):
                pid, fd = self.terminal_session(editing=editing, with_pid=True)
                os.write(fd, b"set -b; TMOUT=1; sleep 2 &\n")
                self.read_until(fd, b"timed out waiting for input")
                self.read_until(fd, None)
                os.waitpid(pid, 0)

    def test_async_notifications_preserve_partial_input_and_wait_status(self):
        fd = self.terminal_session(editing=True, blocked=(signal.SIGCHLD,))
        os.write(fd, b"set -b; (sleep 0.3; exit 7) &\n")
        self.read_until(fd, PROMPT)
        os.write(fd, b"wait $!; echo status=")
        self.read_until(fd, b"Exit 7")
        os.write(fd, b"$?\n")
        self.assertIn(b"status=7", self.read_until(fd, b"status=7"))

    def test_repeated_function_definition_and_positional_frame(self):
        self.compare('set -- outer; f() { shift; printf "%s/%s/%s\\n" "$#" "$1" "${!#}"; }; '
                     'f first "two words" {1..1024}; echo "$#:$1"; '
                     'for a in 1 2; do g(){ echo inner; }; [ "$a" = 1 ] && set -a; done; '
                     'set +a; g; env | grep -q "BASH_FUNC_g%%="; echo "$?"',
                     0, b"1025/two words/1024\n1:outer\ninner\n0\n", minimum=5)

    def test_utf8_substring_endpoints(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare('v=猫😀abc; printf "<%s>\\n" "${v:1:2}" "${v: -2}" "${v:1:-1}" '
                     '"${v:5:0}" "${v:8:-20}" "${v: -9:2}" "${v:0:200}"',
                     0, "<😀a>\n<bc>\n<😀ab>\n<>\n<>\n<>\n<猫😀abc>\n".encode())
        self.compare('v=abc; printf "%s" "${v:2:-2}"', 1)

    def test_array_slice_at_greatest_representable_index(self):
        # GNU Bash 5.3 wraps the exclusive endpoint of this valid sparse array.
        source = ('a=([9223372036854775806]=before [9223372036854775807]=last); '
                  'declare -n ref=a; printf "<%s>\\n" "${a[@]: -1:1}" "${ref[@]: -2:2}"; '
                  'v=1 selected="${ref[*]: -1:1}"; printf "<%s>\\n" "$selected"')
        proc = self.command(TONARI, source)
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"<last>\n<before>\n<last>\n<last>\n", b""))

    def test_slices_of_dense_sparse_and_mapfile_arrays(self):
        self.compare(r"""show() { printf '[%d]' "$#"; printf '<%s>' "$@"; echo; }
a=(one "" "雪 🌸" four)
show "${a[@]:1:2}"
show "${a[@]: -1}"
show "${a[@]: -9:2}"
show "${a[@]:2:0}"
show "${a[@]:4:999999999999}"
a=([2]=low [7]="" [20]="雪 🌸" [90]=high)
show "${a[@]:3:2}"
show "${a[@]: -71:2}"
show "${a[@]: -100:5}"
declare -n ref=a
show "${ref[@]:7:2}"
mapfile -t lines < <(printf 'zero\n\n雪 🌸\nend\n')
show "${lines[@]:1:2}"
show "${lines[@]: -2:1}"
false | true
show "${PIPESTATUS[@]:0:2}"
declare -A assoc=([only]="kept value")
show "${assoc[@]:1:1}"
show "${a[*]:7:2}"
""", 0, """[2]<><雪 🌸>
[1]<four>
[0]<>
[0]<>
[0]<>
[2]<><雪 🌸>
[2]<雪 🌸><high>
[0]<>
[2]<><雪 🌸>
[2]<><雪 🌸>
[1]<雪 🌸>
[2]<1><0>
[1]<kept value>
[1]< 雪 🌸>
""".encode(), minimum=5)

    def test_length_and_slice_follow_live_bindings(self):
        self.compare('s=outside; f(){ local s=inside; v=$s n=${#v} sub=${v:1:3}; '
                     'echo "$n/$sub"; declare -n ref=s; echo "${#ref}/${ref:1:3}"; }; '
                     'f; a=(zero one); echo "${#a}/${a:1:2}/${#s}"; '
                     'set -- abcdef; echo "${#1}/${1:2:2}"', 0,
                     b"6/nsi\n6/nsi\n4/er/7\n6/cd\n", minimum=5)

    def test_arithmetic_fast_paths_keep_numeric_rules(self):
        self.compare('a[8]=octal; a[10]=decimal; v=010; '
                     'echo "${a[010]}:${a[10]}:$((v++)):$v"; '
                     'declare -n ref=v; let ref++; echo "$v"; '
                     'v=9223372036854775807; ((v++)); echo "$v"', 0,
                     b"octal:decimal:8:9\n10\n-9223372036854775808\n", minimum=5)

    def test_readonly_repeated_definition(self):
        self.compare('for i in 1 2; do g(){ :; }; rc=$?; readonly -f g; done; exit "$rc"', 1)

    def test_captured_printf_appends_and_counts_from_each_format(self):
        self.compare(r"""v=$(printf x; printf '%s' 'a b' '' c; printf '%s\n' q r; printf '%s%n:%d' hi n 4; printf '!'); printf '<%s>\n' "$v" """,
                     0, b"<xa bcq\nr\nhi:4!>\n")

    def test_printf_lists_with_count_conversions_and_literal_n(self):
        self.compare(r"""n=outer; v=$(printf '%s\n' one two; printf 'name:%02d' 7); echo "$v:$n"
v=$(printf 'xy%ln' n; printf 'seen:%s\n' "$n"); echo "$v:$n"
v=$(printf '%%n:%s\n' ok; printf end); echo "$v:$n" """,
                     0, b"one\ntwo\nname:07:outer\nxyseen:2:outer\n%n:ok\nend:outer\n")

    def test_capture_search_preserves_parent_hash_and_path_order(self):
        self.compare('mkdir -p a b; for d in b a; do printf "echo %s\\n" "$d" >"$d/tool"; '
                     'chmod +x "$d/tool"; done; PATH=a:b:/usr/bin:/bin; '
                     'echo "$(tool)"; hash -t tool 2>/dev/null; echo "$?"; '
                     'rm a/tool; echo "$(tool)"; hash -p b/tool tool; echo "$(tool)"',
                     0, b"a\n1\nb\nb\n", minimum=4)

    def test_capture_quoted_executable_preserves_path_status_and_function_scope(self):
        (self.cwd / "my shell").symlink_to("/bin/sh")
        self.compare(r"""tool="$PWD/my shell"; set -- "$tool"
a=$("$1" -c 'printf "%s\n" "$1"; exit 19' sh 'two words'); echo "$?:$a"
b=$("$tool" -c 'printf scalar; exit 7'); echo "$?:$b"
function /bin/echo { marker=inner; printf override; }
marker=outer; tool=/bin/echo; b=$("$tool" ignored); echo "$b:$marker"
export -f /bin/echo 2>/dev/null; echo "export:$?"
command /bin/echo real
set -- /usr/bin/printf; tool=/usr/bin/printf
x=$("$1" '%s' "$BASH_SUBSHELL"); echo "$x"
y=$("$tool" '%s' "$(printf nested)"); echo "$y" """,
                     0, b"19:two words\n7:scalar\noverride:outer\nexport:1\nreal\n1\nnested\n")
        for shell in self.shells_for(minimum=4):
            proc = subprocess.run([shell, "-c",
                                   'set -a; /bin/echo() { printf override; }; '
                                   '"$1" -c "/bin/echo child"', "sh", shell],
                                  env=self.env, cwd=self.cwd, capture_output=True, timeout=8)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"child\n"))
            self.assertIn(b"error importing function definition", proc.stderr)

    def test_large_script_read_ahead_preserves_execution_boundaries(self):
        script = self.cwd / "large.sh"
        prefix = "# a comment\n" * 12000 + "if true; then\n  echo before\nfi\n"
        for shell in self.shells_for():
            script.write_text(prefix + "echo after\n")
            check = subprocess.run([shell, "-n", script], env=self.env, capture_output=True)
            self.assertEqual((check.returncode, check.stdout, check.stderr), (0, b"", b""))
            script.write_text(prefix + "if true; then\n")
            check = subprocess.run([shell, "-n", script], env=self.env, capture_output=True)
            self.assertEqual((check.returncode, check.stdout), (2, b""))
            run = subprocess.run([shell, script], env=self.env, capture_output=True)
            self.assertEqual((run.returncode, run.stdout), (2, b"before\n"))

    def test_syntax_check_releases_only_complete_and_or_lists(self):
        script = self.cwd / "syntax.sh"
        prefix = "message='start\n" + "quoted value\n" * 800 + "'\n"
        chains = "false || true && echo side_effect &\nwait\n" * 20
        for suffix, status in [("", 0), ("true &&\n", 2), ("if true; then\n", 2)]:
            script.write_text(prefix + chains + suffix)
            for shell in self.shells_for():
                run = subprocess.run([shell, "-n", script], env=self.env, capture_output=True)
                self.assertEqual((run.returncode, run.stdout), (status, b""))
                self.assertEqual(bool(run.stderr), bool(status))

    def test_case_lookahead_preserves_alias_and_line_number(self):
        script = self.cwd / "case.sh"
        source = ("shopt -s expand_aliases\ncase x in\nx) : <<'BODY'\n"
                  "case text in\ncase text in\nBODY\nalias report=echo ;;\nesac\n"
                  "report line-$LINENO\ncase z in\nz) report end ;;\nesac\n")
        for prefix in ("", "#" + "x" * (128 * 1024 - 70) + "\n"):
            script.write_text(prefix + source)
            for shell in self.shells_for():
                run = subprocess.run([shell, script], env=self.env, capture_output=True)
                line = 10 if prefix else 9
                self.assertEqual((run.returncode, run.stdout, run.stderr),
                                 (0, f"line-{line}\nend\n".encode(), b""))

    def test_large_standalone_unquoted_field_boundaries(self):
        pad = "木🌱" * 96
        for shell in self.shells_for():
            def fields(code, value):
                p = self.command(shell, code + '; printf "<%s>" "$@"', "sh", value)
                self.assertEqual((p.returncode, p.stderr), (0, b""))
                return p.stdout.decode()

            v = pad + "  \t\nleft\rside\vstill\fhere  right "
            self.assertEqual(fields("v=$1; set -- $v", v),
                             "<" + pad + "><left\rside\vstill\fhere><right>")
            self.assertEqual(fields('v=$1; set -- "$v"', v), "<" + v + ">")
            self.assertEqual(fields('v=$1; set -- before $v after', ' \t\n' * 100),
                             "<before><after>")
            self.assertEqual(fields('v=$1; set -- "pre"$v"post"', pad + " middle end"),
                             "<pre" + pad + "><middle><endpost>")
            self.assertEqual(fields("IFS=:; v=$1; set -- $v", pad + ":x y::z"),
                             "<" + pad + "><x y><><z>")
            (self.cwd / "match.txt").touch()
            self.assertEqual(fields('v=$1; set -- $v', pad + " ?.txt *.txt"),
                             "<" + pad + "><?.txt><match.txt>")
            self.assertEqual(fields('shopt -s extglob; v=$1; set -- $v', pad + " @(match).txt"),
                             "<" + pad + "><match.txt>")

    def test_named_tilde_in_arguments_and_assignments(self):
        import pwd
        home = os.fsencode(pwd.getpwnam("root").pw_dir)
        self.compare("""path=~root/bin
printf '<%s>' ~root "$path" '~root/bin' ~ro"ot" ~tonari_missing_user_0432/bin""",
                     0, b"<" + home + b"><" + home + b"/bin><~root/bin><~root><~tonari_missing_user_0432/bin>")

    def test_nested_output_with_redefined_builtins(self):
        self.compare("""v=$(echo $(echo $(echo deep))); printf '[%s]' "$v"
echo() { builtin echo "fn:$1"; }; v=$(echo $(echo value)); printf '[%s]' "$v"
unset -f echo; f() { echo -en 'abc'; }; v=$(f); printf '[%s]' "$v"
v=$(echo $(echo "$BASH_SUBSHELL")); printf '[%s]' "$v" """,
                     0, b"[deep][fn:fn:value][abc][2]")

    def test_self_prefixed_assignments_preserve_the_old_value(self):
        self.compare("""s=abc; x=XYZ; s=$s$x; s="${s}:$s"; printf '<%s>' "$s"
i=0; a=(A B); declare -n next='a[i++]'; i=$i$next; printf '<%s>' "$i"
unset s; s=$s'!'; printf '<%s>' "$s"
declare -i n=3; n=$n+1; printf '<%s>' "$n"
a=(a b); a=$a/$a; printf '<%s>' "${a[@]}"
v='*.rs'; [[ $v == '*.rs' ]] && printf literal; [[ xxx.rs == *.rs ]] && printf glob
shopt -s nocasematch; [[ xyz == XYZ ]] && printf folded""",
                     0, b"<abcXYZ:abcXYZ><0A><!><4><a/a><b>literalglobfolded", minimum=5)

    def test_append_and_array_count_through_references(self):
        self.compare("""v=ab; v+=cd; f() { local v=local; v+=x; printf '<%s>' "$v"; }; f
declare -n ref=v; ref+=ef; printf '<%s>' "$v"
a=([2]="" [75]=tail); declare -n r=a; declare -A m=([one]=x [two]="")
printf '<%s>' "${#a[@]}" "${#r[@]}" "${#m[*]}"; unset 'a[75]'; printf '<%s>' "${#r[@]}" """,
                     0, b"<localx><abcdef><2><2><2><1>", minimum=5)

    def test_export_visibility_after_repeated_child_launches(self):
        self.env["FROM_PARENT"] = "parent"
        self.compare("""probe() { /bin/sh -c 'echo "${FROM_PARENT-missing}/${NEW_NAME-missing}"'; }
probe
export -n FROM_PARENT
NEW_NAME=local
probe; probe
export FROM_PARENT=outer NEW_NAME=new
inside() { local FROM_PARENT=inner; export -n NEW_NAME; probe; }
inside
probe
export NEW_NAME
unset FROM_PARENT
probe""", 0, b"parent/missing\nmissing/missing\nmissing/missing\ninner/missing\nouter/missing\nmissing/new\n")

    def test_external_can_duplicate_inherited_log_descriptor(self):
        self.compare("""exec 5>log
/bin/sh -c 'echo err >&2; echo out' >&5 2>&1
/bin/sh -c 'echo visible; echo logged >&2' 2>&5
/bin/cat log
exec 5>&-
{ /bin/echo wrong >&5; } 2>/dev/null || echo closed""",
                     0, b"visible\nerr\nout\nlogged\nclosed\n")

    def test_missing_command_is_rechecked_and_redirects_take_effect(self):
        self.compare("""PATH=.:/usr/bin:/bin
{ later; echo missing=$?; } 2>err
echo 'echo found' >later; chmod +x later; later
chmod -x later; hash -r; { later; echo denied=$?; } 2>err
{ another >made; echo redirected=$?; } 2>err
[ -e made ] && echo created; rm -f later made""",
                     0, b"missing=127\nfound\ndenied=126\nredirected=127\ncreated\n")

    def test_indexed_assignment_uses_the_live_subscript_once(self):
        self.compare("""x=1; y=2; a=(one two)
a[$x$y]=twelve
i=0; a[i++]+=!
a[-1]+=!
a[07]=seven
idx=(0 3); a[${idx[1]}]=three
printf '%s ' "$i" "${a[@]}"; printf '%s' "${a[12]}"
""", 0, b"1 one! two three seven twelve! twelve!", minimum=4)

    def test_empty_fc_and_top_level_return(self):
        self.compare('fc -lnr; echo "list=$?"; fc -s 2>/dev/null; echo "repeat=$?"; '
                     'return 0 2>/dev/null; echo "return=$?"',
                     0, b"list=0\nrepeat=1\nreturn=2\n", minimum=5)
        self.compare('return 1 2 2>/dev/null', 1, minimum=5)

    def test_arithmetic_error_discards_current_input_but_nounset_exits(self):
        for shell in self.shells_for(5):
            for script, status, output in [
                (b"echo $((42x)); echo wrong\necho recovered\n", 0, b"recovered\n"),
                (b"set -u\necho $((absent + 1))\necho wrong\n", 1, b""),
                (b"echo $((02#0110))\n", 1, b""),
            ]:
                with self.subTest(shell=shell, script=script):
                    p = subprocess.run([shell], input=script, capture_output=True, env=self.env, cwd=self.cwd)
                    self.assertEqual((p.returncode, p.stdout), (status, output))
                    self.assertTrue(p.stderr)

    def test_compound_append_after_shift_and_sparse_index(self):
        self.compare('set -- a b "c d"; shift; f() { shift; a+=( "$@" ); }; '
                     'a=(start); f skip "$@"; unset "a[1]"; a+=(end); '
                     'a+=([1]=rest); printf "<%s>" "${a[@]}"; echo "/$#/$1"',
                     0, b"<start><rest><c d><end>/2/b\n")

    def test_quoted_whole_array_preserves_fields_after_mutation(self):
        self.compare('a=("" "a b" "*" "雪"); f() { printf "<%s>" "$@"; echo; }; '
                     'f "${a[@]}"; unset "a[1]"; f "${a[@]}"; '
                     '''mapfile -t a <<< $'x y\\n*'; f "${a[@]}"; '''
                     'a=(); set -- "${a[@]}"; echo "$#"',
                     0, "<><a b><*><雪>\n<><*><雪>\n<x y><*>\n0\n".encode(), minimum=4)

    def test_literal_assignment_tracks_attributes_and_integer_syntax(self):
        self.compare(r"""
n=one
n+=02
printf '%s ' "$n"
unset n
declare -i n=07
n+=010
n+=16#f
n+=-3
n+=2
printf '%s ' "$n"
declare -n ref=n
ref+=2
printf '%s ' "$n"
declare -a a=(start end)
a+=8
printf '%s %s ' "${a[0]}" "${a[1]}"
set -a
n=9
sh -c 'printf "%s" "$n"'
""", 0, b"one02 29 31 start8 end 9", minimum=4)

    def test_nocasematch_across_shell_contexts(self):
        self.env["LC_ALL"] = "en_US.UTF-8"
        script = r"""shopt -s extglob
f() {
  case "$1" in
    "Ab*" ) printf 'literal ' ;;
    @(GiT|SVN)-[A-F] ) printf 'group ' ;;
    $pat ) printf 'variable ' ;;
    * ) printf 'no ' ;;
  esac
}
pat='abc*'
shopt -s nocasematch
f 'aB*'; f 'git-f'; f 'ABCz'
v='aB雪Ab'; printf '<%s> <%s> <%s> ' "${v//ab/[&]}" "${v/#AB/X}" "${v/%aB/X}"
[[ 'ab' == [[:upper:]]* ]] || printf 'lower '
[[ 'Ab' == [[:upper:]]* ]] && printf 'upper '
[[ 'aB' =~ ^(A)(b)$ ]] && printf '%s,%s ' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
printf '<%s> ' "${v#A}"
compgen -W 'git Git tag' -X 'G*' | tr '\n' ' '
shopt -u nocasematch
f 'git-f'; [[ 'aB' =~ ^(A)(b)$ ]] || printf 'strict'
"""
        self.compare(script, 0, "literal group variable <[aB]雪[Ab]> <X雪Ab> <aB雪X> "
                     "lower upper a,B <aB雪Ab> tag no strict".encode(), minimum=5)
        self.compare(r"""shopt -s extglob nocasematch
v=aB; p='?(q)'; printf '<%s>' "${v//$p/x}" "${v/%$p/x}"
v=a; p='!(A|B)'; printf '<%s>' "${v//$p/x}" "${v/%$p/x}"
v=''; printf '<%s>' "${v//$p/x}" "${v/%$p/x}" """,
                     0, b"<xaxB><aBx><xa><a><><x>", minimum=5)
        self.compare('shopt -s nocasematch; touch FILE; printf "%s " fi*; '
                     '[[ "É😃" == é? ]] && [[ "Σ" == σ ]] && [ A != a ] && printf yes',
                     0, b"fi* yes", minimum=5)

    def test_redirect_expansion_observes_each_previous_open(self):
        for shell in self.shells_for(5):
            for command in ("printf hi", "/usr/bin/printf hi", "v=assigned", ":", ""):
                for suffix in ("", " | /bin/cat"):
                    with self.subTest(shell=shell, command=command, suffix=suffix):
                        for name in ("first", "after", "before"):
                            (self.cwd / name).unlink(missing_ok=True)
                        line = command + ' >first >"$(test -f first && printf after || printf before)"' + suffix
                        result = self.command(shell, line + '; echo "end:$?"')
                        self.assertEqual((result.returncode, result.stdout, result.stderr),
                                         (0, b"end:0\n", b""))
                        self.assertEqual((self.cwd / "first").read_bytes(), b"")
                        self.assertFalse((self.cwd / "before").exists())
                        data = b"hi" if "printf" in command else b""
                        self.assertEqual((self.cwd / "after").read_bytes(), data)
            script = """unset copied; >first >"${copied:=after}"; echo "$copied"
unset copied; >first <first >"${copied:=after}"; echo "${copied-unset}"
unset copied; /usr/bin/true >first >"${copied:=after}"; echo "${copied-unset}"
x=$(exit 3) >first >"$(printf after; exit 2)"; echo "$?"
exec 4>four; printf 'ok' 3>&4 >"$(printf 'on-fd' >&3; printf after)"; cat four
"""
            result = self.command(shell, script)
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, b"after\nunset\nunset\n2\non-fd", b""))

            for setup, command, expected in (
                    ("set -o posix", ":", (1, b"")),
                    ("set -e", "{ :; }", (1, b"")),
                    ("trap 'echo ERR' ERR", "{ :; }", (0, b"ERR\nnext\n"))):
                script = setup + "; " + command + ' >absent/file >"$(echo later)"; echo next'
                result = self.command(shell, script)
                self.assertEqual((result.returncode, result.stdout), expected)
                self.assertTrue(result.stderr)
                self.assertFalse((self.cwd / "later").exists())

    def test_descriptor_move_uses_the_shell_spelling(self):
        script = """name='メモ-'; printf o >&"$name"
name='3-'; printf a >&"$name"; printf b >&'3-'
exec 6>six; fd=6; exec 4>&$fd-; printf m >&4; exec 4>&-
test -e /dev/fd/6; echo $?
exec 6<six; fd=6; exec 5<&"$fd"-; read -r -n1 -u5 got; echo "$got"
test -e /dev/fd/6; echo $?; exec 5<&-
fd='5-'; : 4<&"$fd" 2>/dev/null; echo $?
cat メモ- 3- six
"""
        for shell in self.shells_for(5):
            result = self.command(shell, script)
            self.assertEqual((result.returncode, result.stdout), (0, b"1\nm\n1\n1\nobm"))
            self.assertTrue(result.stderr)

    def test_named_descriptors_and_ordered_expansion(self):
        self.compare('exec {out}>file 5>&$out; printf "雪" >&5; exec {out}>&-; '
                     'test -e /dev/fd/$out; printf "%d\\n" "$?"; cat file; '
                     'f(){ read -r -u "$input" value; printf "<%s>" "$value"; }; '
                     'f {input}<<<"🌸 x"; exec {input}<&-; '
                     '/usr/bin/true {child}>file; echo "${child-unset}"', 0,
                     "1\n雪<🌸 x>unset\n".encode(), minimum=4)
        self.compare('shopt -s varredir_close; : {tmp}>out; test -e /dev/fd/$tmp; echo "$?"; '
                     'exec {keep}>out; test -e /dev/fd/$keep; echo "$?"; exec {keep}>&-', 0,
                     b"1\n0\n", minimum=5)

        for shell in self.shells_for(minimum=5):
            for name, expected in (("true", (127, b"")), ("/usr/bin/true", (0, b"after\n"))):
                result = self.command(shell, name + ' {out}>"${missing:?unset}"; echo after')
                self.assertEqual((result.returncode, result.stdout), expected)
                self.assertIn(b"unset", result.stderr)

    def test_invalid_expansion_in_unselected_shell_branch(self):
        self.compare('if false; then echo ${(M)${(k)parameters[@]}:#git_*}; fi; echo ready',
                     0, b"ready\n")
        for shell in self.shells_for():
            result = self.command(shell, 'echo ${(M)name}; echo unreachable')
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b"")
            self.assertIn(b"bad substitution", result.stderr)

    def test_buffered_profile_preserves_parse_boundaries(self):
        self.env["BASH_ENV"] = str(self.cwd / "startup")
        (self.cwd / "startup").write_text(
            "shopt -s expand_aliases\nif true; then\n  if true; then alias greet='echo ok'; fi\nfi\n"
            'greet\nf(){\n echo "$LINENO"\n}\nf\n')
        self.compare(":", 0, b"ok\n7\n")

    def test_large_startup_function_preserves_alias_and_heredoc_boundaries(self):
        profile = self.cwd / "startup"
        self.env["BASH_ENV"] = str(profile)
        body = ("shopt -s expand_aliases\ninstall_alias() {\n"
                ": <<'END'\n}\nfi\nEND\n" + "  : padding-in-function-body\n" * 600
                + "  alias greet='echo ready'\n}\ninstall_alias\ngreet\n")
        profile.write_text(body + 'echo "$LINENO"\nreturn 0\n)\n')
        self.compare("echo running", 0, f"ready\n{len(body.splitlines()) + 1}\nrunning\n".encode())

    def test_select_menu_reply_and_eof(self):
        source = """PS3='> '; set -- first '雪 x'; select item; do
printf '<%s:%s>\\n' "$item" "$REPLY"; done"""
        for shell in self.shells_for():
            p = subprocess.run([shell, "-c", source], cwd=self.cwd, env=self.env,
                               input=b"\n 02 \ninvalid\n", capture_output=True, timeout=2)
            menu = "1) first\n2) 雪 x\n> ".encode()
            self.assertEqual((p.returncode, p.stdout, p.stderr),
                             (1, "<雪 x: 02 >\n<:invalid>\n\n".encode(), menu + menu + b"> > "))

    def test_timed_pipelines_and_coprocess_descriptors(self):
        self.compare('TIMEFORMAT="timed %%"; { time false | true; time ! true; } 2>clock; '
                     'echo "$?"; cat clock', 0, b"1\ntimed %\ntimed %\n")
        self.compare('coproc WORK { read -r x; printf "<%s>\\n" "$x"; exit 7; }; '
                     'p=$WORK_PID; exec 7<&"${WORK[0]}"; '
                     '( [[ -e /dev/fd/${WORK[0]} ]]; echo "inherited=$?" ); '
                     'printf "雪 x\\n" >&"${WORK[1]}"; wait "$p"; echo "$?"; '
                     'cat <&7; exec 7<&-; echo "${WORK[0]-gone}"', 0,
                     "inherited=1\n7\n<雪 x>\ngone\n".encode(), minimum=4)

    def test_arithmetic_scalars_keep_bash_number_syntax(self):
        self.compare('a=017; b=16#ff; c="a + b"; n=-31; array=(12); declare -n ref=n; '
                     'printf "%s\\n" "$((a))" "$((b))" "$((c))" "$((ref))" "$((array))"; '
                     'x=8; ((x <<= 2)); ((x >>= 1)); ((x ^= 3)); echo "$x"', 0,
                     b"15\n255\n270\n-31\n12\n19\n", minimum=4)

    def test_function_exports_across_shells(self):
        source = r'''
f(){ printf '%s\n' "a:$1"; }
export -f f
"$1" -c 'f "雪 x"'
f(){ printf '%s\n' "b:$1"; }
"$1" -c 'f "🌸 y"'
export -fn f
"$1" -c 'type -t f || echo absent'
export -f f; unset -f f
"$1" -c 'type -t f || echo gone'
'''
        for parent in self.shells_for(4):
            for child in self.shells_for(4):
                result = self.command(parent, source, "shell", str(child))
                self.assertEqual((result.returncode, result.stdout, result.stderr),
                                 (0, "a:雪 x\nb:🌸 y\nabsent\ngone\n".encode(), b""), (parent, child))

    def test_active_function_keeps_its_source_across_redefinition(self):
        (self.cwd / "first").write_text(
            'f() { source ./second; f; printf "original %s\\n" "${BASH_SOURCE[0]}"; }\n')
        (self.cwd / "second").write_text(
            '\nf() { printf "new %s\\n" "${BASH_SOURCE[0]}"; unset -f f; }\n')
        self.compare('source ./first; f; type -t f || echo gone', 0,
                     b"new ./second\noriginal ./first\ngone\n")

    def test_function_wrapper_redefinition_preserves_read_time_context(self):
        # Alias expansion happens when a function is defined. Redirections and
        # positional arguments are evaluated at the call, even after a replacement.
        source = r"""shopt -s expand_aliases
alias action='printf "old:%s\n"'
f() { action "$@"; }
alias action='printf "new:%s\n"'
f '雪 x'
f() { action "$@"; }
unalias action
f 'two words'
f() { printf '%s\n' "$@"; } >"$out"
out=redirected
f a b
cat "$out"
function f {
  printf '%s:%s\n' "$LINENO" "$1"
}
f done
"""
        self.compare(source, 0, "old:雪 x\nnew:two words\na\nb\n14:done\n".encode(), minimum=4)
        for malformed in ("function f{ echo x; }", "f(){ echo x\n ; }"):
            self.compare(malformed, 2)

    def test_reserved_words_as_function_arguments_patterns_and_operands(self):
        source = r"""f() {
  local -a data=(if then else fi)
  printf '%s|' "${data[@]}"
  for x in for do done; do printf '%s|' "$x"; done
  [[ if = if ]] && printf 'ok|'
  case then in then) printf '%s\n' "$(case x in x) echo done;; esac)";; esac
}
f"""
        self.compare(source, 0, b"if|then|else|fi|for|do|done|ok|done\n", minimum=4)

    def test_caller_lines_and_debug_inheritance(self):
        script = self.cwd / "calls.sh"
        script.write_text('inner(){\n  caller; caller 0; caller 1; echo "${BASH_LINENO[*]}:$LINENO"\n}\n'
                          'outer(){\n  inner\n}\nouter\n')
        expected = f"5 {script}\n5 outer {script}\n7 main {script}\n5 7 0:2\n".encode()
        for shell in self.shells_for(4):
            result = subprocess.run([shell, script], env=self.env, capture_output=True, timeout=2)
            self.assertEqual((result.stdout, result.stderr, result.returncode), (expected, b"", 0))
        self.compare('f(){ : inner; }; trap \'echo "$BASH_COMMAND:$?"\' DEBUG; '
                     'false; f; set -T; f; trap - DEBUG', 0,
                     b'false:0\nf:1\nset -T:0\nf:0\nf:0\n: inner:0\ntrap - DEBUG:0\n', minimum=4)

    def test_builtin_control_and_suspension(self):
        self.compare('enable -n echo; type -t echo; builtin echo no 2>/dev/null; printf "%d\\n" "$?"; '
                     'enable echo; type -t echo; help -s printf', 0,
                     b'file\n1\nbuiltin\nprintf: printf [-v var] format [arguments]\n', minimum=4)
        for shell in self.shells_for(4):
            self.check_suspension(shell)

    def check_suspension(self, shell):
        # A separate session lets both implementations suspend their entire group safely.
        with subprocess.Popen([shell, "-c", "suspend -f; printf resumed"], start_new_session=True,
                              cwd=self.cwd, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
            try:
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    pid, status = os.waitpid(proc.pid, os.WUNTRACED | os.WNOHANG)
                    if pid:
                        self.assertTrue(os.WIFSTOPPED(status), (shell, status))
                        break
                    time.sleep(.005)
                else:
                    self.fail(f"{shell} did not stop")
                os.kill(proc.pid, signal.SIGCONT)
                self.assertEqual(proc.communicate(timeout=2), (b"resumed", b""))
                self.assertEqual(proc.returncode, 0)
            finally:
                if proc.poll() is None:
                    proc.kill()

    def test_editor_inputrc_macro_and_shell_binding(self):
        self.env.update(LC_ALL="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8",
                        INPUTRC=str(self.cwd / "inputrc"))
        (self.cwd / "inputrc").write_text('$if Bash\n"\\C-xm": "ZZ"\n$endif\n')
        for shell in self.shells_for(5):
            capture = self.cwd / "capture"
            result = self.cwd / "result"
            capture.unlink(missing_ok=True)
            result.unlink(missing_ok=True)
            fd = self.terminal_session(editing=True, shell=shell)
            os.write(fd, b"""f(){ printf '%s\\n' "$READLINE_LINE" "$READLINE_POINT" "$READLINE_MARK" >capture; READLINE_LINE='echo ready >result'; READLINE_POINT=4; }; bind -x '"\\C-xr":f'\r""")
            os.write(fd, '猫🌸'.encode() + b'\x18m\x02\x18r\r')
            deadline = time.monotonic()+3
            while not result.exists() and time.monotonic()<deadline:
                if select.select([fd], [], [], .025)[0]:
                    os.read(fd, 8192)
            self.assertTrue(result.exists(), (shell, capture.read_bytes() if capture.exists() else None))
            self.assertEqual(result.read_text(), "ready\n")
            self.assertEqual(capture.read_text().splitlines(), ["猫🌸ZZ", "3", "0"])

    def test_child_environment_with_many_private_variables(self):
        code = """for ((i=0;i<256;i++)); do printf -v "private_$i" %d "$i"; done
export shared=first
/usr/bin/printenv shared
private_7=changed; shared=second; /usr/bin/printenv shared
/usr/bin/printenv private_7 || echo private
set -a; private_7+=promoted; set +a; /usr/bin/printenv private_7
f() { local shared=local; /usr/bin/printenv shared; }; f
/usr/bin/printenv shared
export -n LC_ALL; /usr/bin/printenv LC_ALL || echo masked
export -n shared; /usr/bin/printenv shared || echo hidden
private_8=prefix /usr/bin/printenv private_8
/usr/bin/printenv private_8 || echo restored
unset private_7; /usr/bin/printenv private_7 || echo unset
"""
        self.compare(code, 0, b"first\nsecond\nprivate\nchangedpromoted\nlocal\nsecond\n"
                              b"masked\nhidden\nprefix\nrestored\nunset\n")

    def test_fixed_width_parameter_patterns(self):
        code = r"""s=ab-12:cd-34
p='??-??'; printf '%s\n' "${s/$p/X}" "${s//$p/(&)}" "${s/#$p/H}" "${s/%$p/T}"
p='[a-z][a-z]-[[:digit:]][[:digit:]]'; printf '%s\n' "${s##$p}" "${s%%$p}"
s='a?b?'; p='?\?'; printf '%s\n' "${s//$p/X}"
"""
        self.compare(code, 0, b"X:cd-34\n(ab-12):(cd-34)\nH:cd-34\nab-12:T\n:cd-34\nab-12:\nXX\n", minimum=5)
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare("""s='雪😀-ab:xy-12'; p='??-??'
printf '%s\\n' "${s//$p/X}" "${s##$p}" "${s%%$p}"
""", 0, "X:X\n:xy-12\n雪😀-ab:\n".encode(), minimum=5)

    def test_uncached_launch_honors_execignore_on_inherited_path(self):
        first, second = self.cwd / "first", self.cwd / "second"
        first.mkdir()
        second.mkdir()
        for directory, value in ((first, "first"), (second, "second")):
            program = directory / "pick"
            program.write_text(f'#!/bin/sh\nprintf "%s" {value}\n')
            program.chmod(0o755)
        self.env["PATH"] = f"{first}:{second}:/usr/bin:/bin"
        self.env["PICK_ROOT"] = str(self.cwd)
        self.compare('''set +h; EXECIGNORE="$PICK_ROOT/first/*"
pick; echo ":$?"
pick |& cat; echo ":${PIPESTATUS[*]}"
EXECIGNORE="$PICK_ROOT/*"; pick |& cat >/dev/null; echo "${PIPESTATUS[*]}"''',
                     0, b"second:0\nsecond:0 0\n127 0\n", minimum=5)

    def test_executable_text_without_shebang(self):
        script = self.cwd / "plain"
        script.write_text('args=("two words" "$1"); printf "<%s><%s>\\n" "$0" "${args[*]}"; exit 7\n')
        script.chmod(0o700)
        code = 'PATH=.:$PATH; plain "x y" | cat; echo "${PIPESTATUS[*]}"; ./plain z'
        self.compare(code, 7, b"<./plain><two words x y>\n7 0\n<./plain><two words z>\n")
        self.compare('value=$(./plain z); echo "$?:$value"', 0, b"7:<./plain><two words z>\n")

        script.write_text('printf "<%s>\\n" "$#" "$@"\n')
        for launch in ('./plain', 'exec -a renamed ./plain'):
            self.compare(launch + ' "" "a b" 日本語 ""', 0, "<4>\n<>\n<a b>\n<日本語>\n<>\n".encode())

        directory = self.cwd / "commands"
        directory.mkdir()
        helper = directory / "helper"
        helper.write_text("echo found\n")
        helper.chmod(0o700)
        script.write_text('if command -v helper >/dev/null; then helper; else echo default; fi\n')
        self.compare('PATH=commands:/bin; ./plain; export -n PATH; ./plain; echo "$PATH"', 0,
                     b"found\ndefault\ncommands:/bin\n")

        (self.cwd / "linked").symlink_to(directory, target_is_directory=True)
        helper.write_text('printf "<%s><%s>\\n" "${PWD##*/}" "${OLDPWD##*/}"\n')
        self.compare("export OLDPWD; cd linked; PATH=.:$PATH; helper; unset OLDPWD; ./helper",
                     0, f"<linked><{self.cwd.name}>\n<linked><>\n".encode())

    def test_executable_text_with_shared_input_and_inherited_descriptor(self):
        outer = self.cwd / "outer"
        outer.write_text("""for fd in /dev/fd/*; do
    if [ "$fd" != /dev/fd/0 ] && [ "$fd" -ef /dev/stdin ]; then echo "extra input: $fd"; fi
done
IFS= read -r line || exit 2
./inner "$line"
""")
        inner = self.cwd / "inner"
        inner.write_text('printf "<%s>\\n" "$1" >&9; exit 7\n')
        for file in (outer, inner):
            file.chmod(0o700)
        script = self.cwd / "input"
        script.write_text("exec 9>record\n./outer\nfirst line\necho $?\n"
                          "./outer\nsecond line\necho $?\nexec 9>&-\ncat record\n")
        for shell in self.shells_for():
            with self.subTest(shell=shell), script.open("rb") as stdin:
                result = subprocess.run([shell], cwd=self.cwd, env=self.env, stdin=stdin,
                                        capture_output=True, timeout=5)
                self.assertEqual((result.returncode, result.stdout, result.stderr),
                                 (0, b"7\n7\n<first line>\n<second line>\n", b""))

    def test_history_expansion_in_script_and_at_prompt(self):
        source = """set -o history
set -H
history -s 'echo "two words" tail'
printf '<%s>\\n' !!:^ !$
"""
        for shell in self.shells_for(4):
            proc = self.command(shell, source)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"<two words>\n<tail>\n"), proc.stderr)
            self.assertIn(b'"two words" tail', proc.stderr)
            fd = self.terminal_session(shell=shell)
            os.write(fd, b"echo 'original text' >first\n")
            self.read_until(fd, PROMPT)
            os.write(fd, b"!!:s/first/second/\n")
            self.read_until(fd, PROMPT)
            self.assertEqual((self.cwd / "second").read_bytes(), b"original text\n")
            (self.cwd / "second").unlink()
            os.write(fd, b"!!:p\n")
            self.read_until(fd, PROMPT)
            self.assertFalse((self.cwd / "second").exists())

    def test_debug_for_headers_and_trace_fd(self):
        self.compare("trap 'echo \"$BASH_COMMAND\"' DEBUG; for x in a b; do :; done; "
                     "case x in x) :;; esac; for ((i=0; i<1; i++)); do :; done; trap - DEBUG",
                     0, b"for x in a b\n:\nfor x in a b\n:\ncase x in \n:\n"
                     b"((i=0))\n((i<1))\n:\n((i++))\n((i<1))\ntrap - DEBUG\n", minimum=4)
        self.compare("exec 7>trace; BASH_XTRACEFD=7; PS4='TRACE '; set -ax; v='two words'; "
                     "echo \"$v\"; set +x; /bin/sh -c 'test \"$v\" = \"two words\"' && cat trace", 0,
                     b"two words\nTRACE v='two words'\nTRACE echo 'two words'\nTRACE set +x\n", minimum=4)

    def test_readonly_scopes_attributes_and_write_paths(self):
        self.compare("""exec 2>/dev/null
readonly g=outer
f(){ local g=inner; echo "$?:$g"; local -ar a=(one 'two words'); nested; declare -p a; }
nested(){ local a=inner; echo "$?:$a"; }
f
export g; export g=changed; echo "$?:$g"; declare -p g
unset g; echo "$?:$g"; read g <<<new; echo "$?:$g"; printf -v g new; echo "$?:$g"
readonly -a data=(one two); mapfile -t data <<<new; echo "$?:${data[*]}"
readonly -f f; unset -f f; echo $?; f(){ :; }; echo $?; declare -pF f
""", 0, b'1:outer\n0:inner\ndeclare -ar a=([0]="one" [1]="two words")\n'
             b'1:outer\ndeclare -rx g="outer"\n1:outer\n1:outer\n1:outer\n1:one two\n1\n1\ndeclare -fr f\n', minimum=5)

    def test_readonly_recovery_in_command_file_and_stdin(self):
        script = """exec 2>/dev/null
readonly x=1; x=2; echo wrong
echo after:$?:$x; ((x++)); echo arithmetic:$?
echo "$((x++))"; echo wrong
echo expanded:$?; x=2 y=ok /bin/sh -c 'echo prefix:$y'
f(){ x=9; echo wrong; }; f; echo wrong
echo function:$?; eval 'x=8; echo wrong'; echo eval:$?
readonly a=(one two); readonly a+=(bad); echo wrong
declare -p a; (x=3); echo subshell:$?
"""
        expected = b'after:1:1\narithmetic:1\nexpanded:1\nprefix:ok\nfunction:1\neval:1\ndeclare -ar a=([0]="one" [1]="two")\nsubshell:1\n'
        (self.cwd / "script").write_text(script)
        for shell in self.shells_for(5):
            for args, stdin in [(["-c", script], None), (["script"], None), ([], script.encode())]:
                with self.subTest(shell=shell, args=args[:1]):
                    result = subprocess.run([shell, *args], cwd=self.cwd, env=self.env,
                                            input=stdin, capture_output=True, timeout=5)
                    self.assertEqual((result.returncode, result.stdout, result.stderr), (0, expected, b""))

    def test_err_trap_inheritance_and_tested_commands(self):
        self.compare("""trap 'echo err:$?' ERR
false || :; ! true; if false; then :; fi; { false && :; }; echo group:$?
f(){ false; }; f; echo outer:$?
set -E; f; echo inherited:$?
v=$(false; echo done); printf '<%s>\n' "$v"; (false); echo sub:$?
trap - ERR; false; echo last:$?
""", 0, b'group:1\nerr:1\nouter:1\nerr:1\nerr:1\ninherited:1\n<err:1\ndone>\nerr:1\nerr:1\nsub:1\nlast:1\n', minimum=5)
        self.compare("set -e; { false && :; }; echo survived", 0, b"survived\n")
        self.compare("set -e; trap 'echo err:$?' ERR; false; echo wrong", 1, b"err:1\n")
        self.compare("set -e; shopt -s inherit_errexit; v=$(false; echo wrong); echo wrong", 1, minimum=5)

    def test_return_trap_function_and_source_boundary(self):
        (self.cwd / "ret.sh").write_text("false; return 6\n")
        self.compare("""trap 'echo leaving:$?' RETURN
f(){ false; return 7; }; f; echo untraced:$?
. ./ret.sh; echo sourced:$?
set -T; f; echo traced:$?
g(){ :; }; g; trap - RETURN; g; echo done
""", 0, b"untraced:7\nleaving:1\nsourced:6\nleaving:1\ntraced:7\nleaving:0\ndone\n", minimum=5)

    def test_readonly_shell_variables_and_exported_options(self):
        self.compare("""exec 2>/dev/null
[[ $EUID == "$(id -u)" && $UID == "$(id -ru)" && $PPID -gt 0 ]] && echo ids
parent=$PPID; (test "$parent" = "$PPID" && echo inherited)
unset PPID; echo readonly:$?; declare -p UID | cut -d= -f1
export SHELLOPTS BASHOPTS; set -o pipefail; shopt -s nullglob
"$BASH" -c '[[ :$SHELLOPTS: == *:pipefail:* && :$BASHOPTS: == *:nullglob:* ]] && echo child; printf "<%s>" missing-*.absent'
shopt -u nullglob; set +o pipefail
"$BASH" -c '[[ :$SHELLOPTS: != *:pipefail:* && :$BASHOPTS: != *:nullglob:* ]] && echo reset'
""", 0, b"ids\ninherited\nreadonly:1\ndeclare -ir UID\nchild\n<>reset\n", minimum=5)

    def test_failglob_and_full_path_globignore(self):
        script = """exec 2>/dev/null
shopt -s nullglob failglob; echo no-match-*; echo wrong
echo recovered:$?; /bin/echo no >missing-*.file; echo redirected:$?
shopt -u failglob nullglob; shopt -s extglob
mkdir sub; touch '++' ';' keep sub/file
GLOBIGNORE='+([^[:alnum:]])'; printf '<%s>' ./*; echo
GLOBIGNORE='@(sub/file|./keep)'; printf '<%s>' ./* sub/*; echo
"""
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                version = self.command(BASH, 'echo "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"')
                newer = tuple(map(int, version.stdout.strip().split(b"."))) >= (5, 3)
                prefix = b"<./++><./;>" if shell != BASH or newer else b""
                expected = b"recovered:1\nredirected:1\n" + prefix + b"<./keep><./sub>\n<./++><./;><./sub><sub/*>\n"
                result = self.command(shell, script)
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, expected, b""))

    def test_associative_keys_scope_and_expansion(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare(r"""declare -A a=([foo]=one ["x ] 雪🌸"]="two words" ["*"]=literal)
k='x ] 雪🌸'; a["$k"]+='!'
printf '<%s>/<%s>/%s\n' "${a[$k]}" "${a["*"]}" "${#a[@]}"
for k in "${!a[@]}"; do printf '[%s]=%s\n' "$k" "${a[$k]}"; done | LC_ALL=C sort
f(){ local -A a=(red first blue second spare); printf '%s:%s:%s\n' "${a[red]}" "${a[blue]}" "${#a[@]}"; }; f
k='x ] 雪🌸'; printf -v 'a[$k]' '%s' updated; [[ -v 'a[$k]' ]] && echo "${a[$k]}"
unset 'a[$k]'; echo "${#a[@]}"; readonly -A a; (a[foo]=no) 2>/dev/null; echo "$?:${a[foo]}"
declare -Ai count=([ok]=1+2); count[ok]+=4; declare count[ok]+=2; declare -p count
""", 0, ("<two words!>/<literal>/3\n[*]=literal\n[foo]=one\n"
             "[x ] 雪🌸]=two words!\nfirst:second:3\nupdated\n2\n1:one\n"
             'declare -Ai count=([ok]="9" )\n').encode(), minimum=5)

    def test_parameter_transformations_preserve_fields(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        self.compare(r"""v="héLLo🌸"; printf '<%s>\n' "${v^^}" "${v,,}" "${v^}" "${v,,[L]}"
unset v; printf '<%s>' "${v@Q}"; v=''; printf '<%s>' "${v@Q}"; echo
a=('one two' "can't" $'a\nb'); printf '[%s]\n' "${a[@]@Q}"
v='one\ntwo'; printf '<%s>\n' "${v@E}"
v=aBc; echo "${v@U} ${v@u} ${v@L}"
v=résumé; echo "${v^^[a-z]}"
v=éßa; LC_ALL=C; echo "${v^^}"; LC_ALL=C.UTF-8; echo "${v^^}"
""", 0, ("<HÉLLO🌸>\n<héllo🌸>\n<HéLLo🌸>\n<héllo🌸>\n<><''>\n"
             "['one two']\n['can'\\''t']\n[$'a\\nb']\n<one\ntwo>\nABC ABc abc\nRéSUMé\néßA\nÉßA\n").encode(), minimum=5)

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
        self.compare('set -o pipefail; yes | head -n 1; printf "%s:%s" "$?" "${PIPESTATUS[*]}"',
                     0, b"y\n141:141 0")
        self.compare("exit 42 | cat; printf ok", 0, b"ok")
        self.compare("missing-tonari-command | cat", 0)
        self.compare("printf abc | missing-tonari-command", 127)
        self.compare("printf abc | exit 42", 42)
        self.compare("''", 127)
        tool = self.cwd / "not-executable"
        tool.write_text("plain file")
        self.compare("./not-executable", 126)
        self.compare("cat < absent", 1)

    @unittest.skipUnless(sys.platform.startswith("linux") and hasattr(fcntl, "F_GETPIPE_SZ"),
                         "Linux pipe capacity")
    def test_pipeline_capacity_respects_kernel_limits(self):
        reader, writer = os.pipe()
        try:
            default = fcntl.fcntl(reader, fcntl.F_GETPIPE_SZ)
            expected = default
            if 65536 <= default < 131072:
                try:
                    expected = fcntl.fcntl(reader, fcntl.F_SETPIPE_SZ, 131072)
                except OSError:
                    pass  # The kernel can deny an increase for this user.
        finally:
            os.close(reader)
            os.close(writer)
        command = (f"/bin/true | {shlex.quote(sys.executable)} -c "
                   "'import fcntl; print(fcntl.fcntl(0, fcntl.F_GETPIPE_SZ))'")
        for shell, size in ((BASH, default), (TONARI, expected)):
            with self.subTest(shell=shell):
                p = self.command(shell, command)
                self.assertEqual((p.returncode, p.stdout, p.stderr), (0, f"{size}\n".encode(), b""))

    def test_parenthesized_pipeline_stage_is_one_subshell(self):
        self.compare(r"""parent=$BASHPID
printf 'data\n' | ( read -r line; [[ $BASHPID != "$parent" ]] || exit 40
  printf '%s:%s ' "$BASH_SUBSHELL" "$line"; (printf 'nested=%s ' "$BASH_SUBSHELL")
  trap 'printf exit' EXIT; exit 7 )
printf ':%s\n' "${PIPESTATUS[*]}"
printf x | ( printf 'redirect=%s' "$BASH_SUBSHELL" ) >"$(printf dest)"
cat dest; (printf ' standalone=%s' "$BASH_SUBSHELL"); echo
v=$(printf 'z\n' | ( read -r x; printf '%s:%s' "$BASH_SUBSHELL" "$x" )); echo "$v"
""", 0, b"1:data nested=2 exit:0 7\nredirect=1 standalone=1\n2:z\n", minimum=4)

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

    def test_external_null_redirect_with_pipeline_and_script_fallback(self):
        script = self.cwd / "show"
        script.write_text("echo stdout; echo stderr >&2\n")
        script.chmod(0o755)
        self.compare("""./show 2>/dev/null | /bin/cat
/bin/cat </dev/null | /bin/cat
./show >/dev/null 2>&1
./missing 2>/dev/null; echo $?
/bin/echo hidden >/dev/null | /bin/cat
/bin/echo done""", 0, b"stdout\n127\ndone\n")

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
                # The direct pwd writer reserves stdout before opening the file.
                # One free fd must fail without destroying the previous contents.
                file.write_bytes(b"keep")
                proc = subprocess.run(
                    [TONARI, "-c", 'pwd >data; r=$?; exit "$r"'], cwd=self.cwd, env=self.env,
                    preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_NOFILE, (limit, limit)),
                    capture_output=True, timeout=5,
                )
                self.assertEqual(proc.returncode, status, proc.stderr)
                expected = b"keep" if status else os.fsencode(self.cwd.resolve()) + b"\n"
                self.assertEqual(file.read_bytes(), expected)

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

    def test_startup_exports_the_validated_working_directory(self):
        real = self.cwd.resolve() / "real"
        real.mkdir()
        link = real.with_name("logical")
        link.symlink_to(real)
        witness = 'exec /usr/bin/printenv PWD'
        for inherited, expected in ((None, real), ("", real), (".", real),
                                    (real.parent, real), (real / "missing", real),
                                    (real, real), (link, link)):
            env = dict(self.env)
            if inherited is not None:
                env["PWD"] = str(inherited)
            shells = (TONARI,) if inherited == "." and self.bash_before_53() else self.shells_for()
            for shell in shells:
                for source, output in (
                    (witness, [expected]),
                    (r'printf "%s\n" "$PWD"; pwd -L; /usr/bin/printenv PWD | cat; '
                     r'printf "%s\n" "$(/usr/bin/printenv PWD)"', [expected] * 4),
                ):
                    with self.subTest(shell=shell, inherited=inherited, source=source):
                        proc = subprocess.run([shell, "-c", source], cwd=real, env=env,
                                              capture_output=True, timeout=5)
                        self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                         (0, b"".join(os.fsencode(v) + b"\n" for v in output), b""))

        # File, stdin, and executable text without a shebang all reach fresh shell setup.
        script = real / "script"
        script.write_text(witness + "\n")
        script.chmod(0o700)
        env = dict(self.env, PWD=str(real / "missing"))
        for shell in self.shells_for():
            for args, data in (([str(script)], b""), (["-s"], witness.encode()),
                               (["-c", "./script"], b"")):
                proc = subprocess.run([shell, *args], input=data, cwd=real, env=env,
                                      capture_output=True, timeout=5)
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                 (0, os.fsencode(real) + b"\n", b""))
            proc = subprocess.run([shell, "-c", witness], cwd="/", env=env,
                                  capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"/\n", b""))

    def test_startup_pwd_and_logical_cache_after_rename(self):
        real = self.cwd.resolve() / "real"
        link = real.with_name("link")
        link.symlink_to(real)
        for shell in self.shells_for():
            real.mkdir()
            env = dict(self.env, PWD=str(link))
            proc = subprocess.run([shell, "-c", 'mv ../real ../renamed; pwd -L; '
                                   'set -o posix; pwd -L; echo "$PWD"; /usr/bin/printenv PWD'],
                                  cwd=real, env=env, capture_output=True, timeout=5)
            renamed = real.with_name("renamed")
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, f"{link}\n{renamed}\n{link}\n{link}\n".encode(), b""))
            renamed.rmdir()

        # POSIX requires the physical path when PWD contains a dot or dot-dot component.
        real.mkdir()
        for inherited in (f"{link}/.", f"{link}/../link"):
            env = dict(self.env, PWD=inherited)
            proc = subprocess.run([TONARI, "-c", 'pwd; echo "$PWD"; /usr/bin/printenv PWD'],
                                  cwd=real, env=env, capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, (os.fsencode(real) + b"\n") * 3, b""))

    def test_pwd_export_edits_after_startup_repair(self):
        self.env["PWD"] = str(self.cwd / "missing")
        self.compare(r"""PWD=user; /usr/bin/printenv PWD
PWD=prefix /usr/bin/printenv PWD
export -n PWD; /usr/bin/printenv PWD || echo private
cd /; echo "$PWD"; /usr/bin/printenv PWD || echo private
export PWD; /usr/bin/printenv PWD
unset PWD; /usr/bin/printenv PWD || echo absent
PWD=local; /usr/bin/printenv PWD || echo private; echo "$PWD" """,
                     0, b"user\nprefix\nprivate\n/\nprivate\n/\nabsent\nprivate\nlocal\n")

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

    def test_eval_multiline_compounds_execute_in_order(self):
        # Closing words inside quotes and here-documents must stay in the body.
        hook = """# setup
shopt -s expand_aliases
if true; then
    alias say='printf %s'
fi
show() {
    say 'inside
}
quote'
    cat <<'END'
}
fi
雪
END
}
show
"""
        self.compare("eval " + shlex.quote(hook), 0, "inside\n}\nquote}\nfi\n雪\n".encode())
        for ending in ("", "\n"):
            self.compare("eval " + shlex.quote("if true; then\n  false; fi" + ending), 1)
        self.compare("eval " + shlex.quote("f() {\n echo seen\n}\nf\nif )"), 2, b"seen\n")
        source = "eval " + shlex.quote("# comment\n{\n echo $LINENO\n}\necho $LINENO")
        for shell in self.shells_for():
            # Bash 5.2 counts newlines in the quoted argument twice. Tonari
            # and Bash 5.3 start at the line containing the eval invocation.
            expected = b"7\n9\n" if shell == BASH and self.bash_before_53() else b"3\n5\n"
            p = self.command(shell, source)
            self.assertEqual((p.returncode, p.stdout, p.stderr), (0, expected, b""))

    def test_multiline_single_quote_keeps_input_boundaries(self):
        text = "雪\n" + "fi\n" * 700 + "😀"
        code = "v='" + text + "'\nprintf '%s' \"$v\"\n"
        expected = text.encode()
        self.compare(code, 0, expected)
        for shell in self.shells_for():
            with self.subTest(shell=shell):
                proc = subprocess.run([shell], input=code.encode(), capture_output=True,
                                      cwd=self.cwd, env=self.env, timeout=5)
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, expected, b""))
        for suffix in (b"\xff\n", b"\x00\n", b""):
            proc = subprocess.run([TONARI], input=b"printf first\nv='open\nsecond\n" + suffix,
                                  capture_output=True, cwd=self.cwd, env=self.env, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout), (2, b"first"))
            self.assertTrue(proc.stderr)

    def test_eval_large_profile_preserves_command_boundaries(self):
        # Read the text as an rcfile generator would. The actual function close
        # is beyond 8 KiB, with plausible terminators inside its body.
        common = "# settings\n" + "  : ${padding}\n" * 620
        hook = ("shopt -s expand_aliases\nsetup() {\n" + common
                + "  cat <<'END'\n}\nEND\n  alias configured='printf ready'\n}\n"
                + "setup\nconfigured\nprintf ':%s' \"$LINENO\"")
        (self.cwd / "config").write_text(hook)
        self.compare('eval "$(<config)"', 0, f"}}\nready:{len(hook.splitlines())}".encode())
        (self.cwd / "config").write_text(hook + "\nif )")
        self.compare('eval "$(<config)"', 2, f"}}\nready:{len(hook.splitlines())}".encode())

    def test_eval_repeated_code_keeps_runtime_state(self):
        self.compare(r"""set -- 'two words'
for n in 1 2 3 4; do false; eval 'printf "<%s|%s|%s>" "$?" "$n" "$1"'; done
for n in 1 2 3; do eval "i=$n"; done
declare -i j; eval 'j=07'; readonly keep=yes; eval 'keep=no' 2>/dev/null
printf ':%s:%s:%s:%s' "$i" "$j" "$?" "$keep"
f() { eval 'return 19'; echo missed; }; f; printf ':%s' "$?"
for n in a b; do eval 'break'; echo missed; done
eval -- 'printf' '%s' '":雪"'
""", 0, ("".join(f"<1|{i}|two words>" for i in range(1, 5)) + ":3:7:1:yes:19:雪").encode())
        self.compare("eval 'exit 17'; echo missed", 17)

    def test_eval_nested_boundaries_are_validated_by_the_parser(self):
        source = ("shopt -s expand_aliases\nif true; then\n"
                  + "if true; then\n :\n  fi\n" * 300
                  + "cat <<'END'\nif true; then\nEND\n"
                  + "alias loaded='echo ok'\n  fi\nloaded\n")
        (self.cwd / "config").write_text(source)
        self.compare('eval "$(<config)"', 0, b"if true; then\nok\n")

    def test_sourced_and_generated_functions_with_separate_braces(self):
        source = ("setup ()\n{\n" + " : padding\n" * 500
                  + """ printf '%s\\n' 'script /pattern/{' '}'\n}\nsetup\nreturn 23\n""")
        (self.cwd / "config").write_text(source)
        self.compare('. ./config; echo "$?"', 0, b"script /pattern/{\n}\n23\n")
        self.compare('f() { eval "$(<config)"; echo skipped; }; f; echo "$?"',
                     0, b"script /pattern/{\n}\n23\n")
        (self.cwd / "config").write_text("echo before\nif )\n")
        self.compare(". ./config", 2, b"before\n")
        self.assertIn(b"./config: command starting at line 2:",
                      self.command(TONARI, ". ./config").stderr)

    def test_eval_rechecks_parser_options_aliases_and_lines(self):
        self.compare(r"""run() { eval "$code"; }
code='echo "$n"'
for n in one two three; do run; done
code='case b in @(a|b)) echo match;; esac'
shopt -s extglob; run; run; run
shopt -u extglob; run 2>/dev/null; echo "off:$?"
shopt -s expand_aliases
code='say'; alias say='echo first'; run; run; run
alias say='echo second'; run
unalias say
say() { echo function; }; run
for n in one two three; do eval 'echo "$LINENO"'; done
eval 'echo "$LINENO"'
""", 0, b"one\ntwo\nthree\nmatch\nmatch\nmatch\noff:2\nfirst\nfirst\nfirst\nsecond\nfunction\n12\n12\n12\n13\n")
        for shell in self.shells_for():
            probe = self.command(shell, """for i in 1 2 3; do eval ': <<END'; done""")
            self.assertEqual(probe.returncode, 0)
            self.assertEqual(probe.stderr.count(b"warning:"), 3, probe.stderr)

    def test_eval_options_and_temporary_bindings(self):
        self.compare(r"""value=outer
value=inner eval; printf '%s:%s ' "$?" "$value"
value=inner eval --; printf '%s:%s ' "$?" "$value"
value=inner eval -z 2>/dev/null; printf '%s:%s ' "$?" "$value"
eval -- -z 2>/dev/null; printf '%s ' "$?"
eval --help >/dev/null; echo "$?"
""", 0, b"0:outer 0:outer 2:outer 127 2\n")

    def test_eval_patterns_after_a_continued_line(self):
        test = r"""[[ x17 =~ \
            ^(x|y)([0-9]+)$ ]] && echo "${BASH_REMATCH[2]}"
[[ xyz == \
           @(xyz|no) ]] && echo glob"""
        self.compare("code=" + shlex.quote(test) + '\nfor i in 1 2 3; do eval "$code"; done',
                     0, b"17\nglob\n" * 3, minimum=5)
        for ending in (chr(92), chr(92) + "\n"):
            for prefix in ("echo a", "echo a "):
                source = prefix + ending
                literal = (prefix[5:] + chr(92) + "\n").encode() if ending == chr(92) else b"a\n"
                self.compare("eval " + shlex.quote(source), 0, literal)
                path = self.cwd / "tail"
                path.write_text(source)
                self.compare(". ./tail", 0, literal)
                for shell in self.shells_for():
                    expected = b"a\n" if shell == BASH and self.bash_before_53() else literal
                    for argv, data in (([shell, path], None), ([shell], source.encode())):
                        p = subprocess.run(argv, input=data, capture_output=True, env=self.env, cwd=self.cwd)
                        self.assertEqual((p.returncode, p.stdout, p.stderr), (0, expected, b""))

    def test_eval_reads_complete_compounds_with_quoted_terminators(self):
        bodies = ["printf '%s\\n' 'fi\ndone\nesac\n}'", "cat <<'END'\nfi\ndone\nesac\n}\nEND"]
        wrappers = [("if true; then", "fi"), ("for i in 1; do", "done"),
                    ("case ok in\nok)", ";;\nesac"), ("{", "}"),
                    ("run ()\n{", "}\nrun"), ("if true; then\nwhile :; do", "break\ndone\nfi")]
        commands = ["eval " + shlex.quote(f"{header}\n{body}\n{end}\necho next")
                    for header, end in wrappers for body in bodies]
        self.compare("\n".join(commands), 0, b"fi\ndone\nesac\n}\nnext\n" * len(commands))

    def test_cdable_vars_uses_a_variable_after_directory_search(self):
        self.compare(r"""mkdir -p search/mark real both
DEST='real'; mark=real; both=real
shopt -q cdable_vars; echo "off:$?"
cd DEST 2>/dev/null; echo "missing:$?"
shopt -s cdable_vars; CDPATH=search
cd mark >result; echo "path:${PWD##*/}"; cd "$HOME"
cd both; echo "direct:${PWD##*/}"; cd "$HOME"
cd DEST; echo "var:${PWD##*/}"; cd "$HOME"
cd -P DEST; echo "physical:${PWD##*/}"; cd "$HOME"
shopt -u cdable_vars; cd DEST 2>/dev/null; echo "off:$?"
""", 0, b"off:1\nmissing:1\npath:mark\ndirect:both\nreal\nvar:real\nreal\nphysical:real\noff:1\n")

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

    def test_utf8_command_input_and_binary_streams(self):
        invalid = b"printf '<%s>' '\xff'\n"
        (self.cwd / "invalid.sh").write_bytes(invalid)
        for argv, stdin in ((["-c", invalid], b""), (["invalid.sh"], b""), ([], invalid)):
            with self.subTest(argv=argv):
                result = subprocess.run([TONARI, "--noprofile", "--norc", *argv],
                                        input=stdin, cwd=self.cwd, env=self.env,
                                        capture_output=True, timeout=5)
                self.assertEqual((result.returncode, result.stdout), (2, b""))
                self.assertIn(b"UTF-8", result.stderr)
        payload = bytes(range(256))
        (self.cwd / "input.bin").write_bytes(payload)
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                result = self.command(shell, "cat <input.bin | cat >output.bin")
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"", b""))
                self.assertEqual((self.cwd / "output.bin").read_bytes(), payload)
                (self.cwd / "output.bin").unlink()

    def test_utf8_length_and_slicing_in_the_c_locale(self):
        self.env["LC_ALL"] = "C"
        source = 'v=雪🌸; printf "%s/<%s>" "${#v}" "${v:1:1}"'
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, "2/<🌸>".encode(), b""))

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
            if shell == TONARI or not self.bash_before_53():
                self.assertIn(b"regular expression", proc.stderr)
            else:
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

    def test_review_static_case_and_conditional_patterns(self):
        utf8 = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        source = r"""shopt -s extglob
for v in 'b]' ':]' 'a]' 'l]' a b; do [[ $v == [a-[:alpha:]] ]]; printf %s $?; done
[[ b == [[=a=]-c] ]]; echo "/$?"
[[ y == @(x[|y) ]]; echo "$?"
v='aaa\'; for p in 'a\' '?\' '??\' '[a]\'; do printf '<%s>' "${v/%$p/R}"; done; echo
f() { case "$1" in @(foo|bar)) echo y;; *) echo n;; esac; }
f FOO; shopt -s nocasematch; f FOO; shopt -u nocasematch; f FOO
unset LC_ALL; LC_COLLATE=$locale; LC_CTYPE=C; shopt -u globasciiranges
touch éb; [[ éb == ??[a-c] ]]; echo "$?"; echo ??[a-c]
"""
        self.env["locale"] = utf8
        expected = b"100011/1\n1\n<aaR><aaa\\><aaa\\><aaR>\nn\ny\nn\n"
        expected += "0\néb\n".encode()
        for shell in (TONARI, BASH):
            if shell == BASH and bash_version() < (5, 2):
                continue
            p = self.command(shell, source)
            self.assertEqual((p.returncode, p.stdout, p.stderr), (0, expected, b""))

    def test_review_command_lookup_and_redirect_diagnostics(self):
        source = """hash -p /bin/echo cached-name; type -P cached-name; echo "$?"
hash -p /no/such/program stale; stale; echo "$?"
mkdir -p path/directory; touch path/locked; PATH=$PWD/path
directory; echo "$?"; locked; echo "$?"; ./path/directory; echo "$?"
echo x >& ""; echo x 3>&99999999999; echo x >&out-
readonly v=5; exec {v}>file"""
        for shell in (TONARI, BASH):
            if shell == BASH and bash_version() < (5, 3):
                continue
            p = self.command(shell, source)
            self.assertEqual((p.returncode, p.stdout), (1, b"/bin/echo\n0\n127\n127\n126\n126\n"))
            for text in [b"/no/such/program: No such file", b"directory: command not found",
                         b"/path/locked: Permission denied", b"./path/directory: Is a directory",
                         b'"": Bad file descriptor', b"3: Bad file descriptor",
                         b"out: ambiguous redirect", b"v: readonly variable", b"v: cannot assign fd"]:
                self.assertIn(text, p.stderr)
            self.assertNotIn(b"(os error", p.stderr)

    def test_review_trace_and_control_operands(self):
        for shell in (TONARI, BASH):
            if shell == BASH and bash_version() < (5, 3):
                continue
            p = self.command(shell, "BASH_XTRACEFD='1 '; set -x; : one; BASH_XTRACEFD=-5; : two")
            self.assertEqual((p.returncode, p.stdout), (0, b""))
            for text in [b"1 : invalid value", b"-5: invalid value", b"+ : one", b"+ : two"]:
                self.assertIn(text, p.stderr)
            p = self.command(shell, 'exit invalid; echo $?; for i in 1; do continue invalid; done')
            self.assertEqual((p.returncode, p.stdout), (2, b"2\n"))
            self.assertIn(b"exit: invalid: numeric argument required", p.stderr)
            self.assertIn(b"continue: invalid: numeric argument required", p.stderr)
            p = self.command(shell, 'p="("; [[ a =~ $p ]]; echo $?; set -u; [[ a == "$missing" ]]')
            self.assertEqual((p.returncode, p.stdout), (127, b"2\n"))
            self.assertIn(b"invalid regular expression `(':", p.stderr)
            self.assertIn(b"missing: unbound variable", p.stderr)
            self.assertNotIn(b"[[: missing:", p.stderr)

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
                # Tonari and Bash 5.3 dequote the collating or equivalence operand.
                members = selected
                if (shell == TONARI or tuple(map(int, version.split(b"."))) >= (5, 3)) and pattern in (r'[[="a"=]]', r'[[."a".]]'):
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

    def test_large_glob_required_extension(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for i in range(256):
            mark = ("a", "雪", "🐟")[i % 3]
            suffix = ("log", "LOG", "txt", "v1")[i % 4]
            (self.cwd / f"entry-{i:03}-{mark}.{suffix}").touch()
        for name in (".hidden.log", "x?.log", "xa.t.xt", "xb.日本語", "xq.lo[g"):
            (self.cwd / name).touch()
        (self.cwd / "d1.box").mkdir()
        (self.cwd / "d1.box" / "f雪.txt").touch()
        source = r"""
show() { printf '%s\0' "$#" "$@"; }
shopt -s nullglob
show entry-???-?.log
show entry-???-?.zip
show entry-???-?.*
show *?.v1
show x\?.log x?.t.xt x?.日本語 x?.lo[g
show entry-[[:digit:]]??-?.log d?.box/f?.txt
shopt -s dotglob
show *?.log
shopt -s nocaseglob
show entry-???-?.LOG
shopt -u nocaseglob
shopt -s extglob
show @(entry-???-?.log|x?.t.xt)
"""
        reference = self.command(BASH, source)
        self.assertEqual((reference.returncode, reference.stderr), (0, b""))
        self.assertTrue(reference.stdout.startswith(b"64\0entry-000-a.log\0"))
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, reference.stdout, b""))

    def test_large_glob_preserves_locale_order(self):
        # Several byte-ordered runs change places in a named locale.
        parts = ["A", "a", "é", "Z", "z", "あ", "雪", "B"]
        for i in range(4200):
            (self.cwd / f"{parts[i % len(parts)]}-{i:05}.txt").touch()
        source = r'printf "%s\0" *.txt'
        for locale in (["en_US.UTF-8", "sv_SE.UTF-8"] if sys.platform == "darwin" else ["C"]):
            self.env["LC_ALL"] = locale
            expected = self.command(BASH, source)
            self.assertEqual(expected.returncode, 0)
            self.assertEqual(expected.stdout.count(b"\0"), 4200)
            self.compare(source, 0, expected.stdout)

    def test_large_glob_with_unrelated_collation_and_byte_order(self):
        rng = random.Random(42)
        for i in range(4200):
            stem = "".join(rng.choices("AaBbZzÊéÉöäÅßØôn.-0129ΣσЖЯあキ漢字🚀", k=12))
            (self.cwd / f"p{stem}-{i}.txt").touch()
        source = r'printf "%s\0" *.txt'
        self.env["LC_ALL"] = "ja_JP.UTF-8" if sys.platform == "darwin" else "C"
        expected = self.command(BASH, source)
        self.assertEqual(expected.returncode, 0)
        self.assertEqual(expected.stdout.count(b"\0"), 4200)
        self.compare(source, 0, expected.stdout)

    def test_numeric_glob_sorts_large_decimal_filenames(self):
        names = ["10", "8", "008", "90000000000000000000", "100000000000000000000",
                 "+7", "90000000000000000000z"]
        for name in names:
            (self.cwd / name).touch()
        # Equal numeric keys retain the filename collation order. A non-digit
        # anywhere in a filename makes it an ordinary string.
        ascending = [names[i] for i in (2, 1, 0, 3, 4, 5, 6)]
        script = 'for GLOBSORT in numeric -numeric; do printf "%s\\n" *; done'
        result = self.command(TONARI, script)
        self.assertEqual((result.returncode, result.stdout.decode().splitlines(), result.stderr),
                         (0, ascending + ascending[::-1], b""))

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

        if sys.platform == "darwin":
            # Darwin collates these names equally. The tie-break compares unsigned bytes.
            sub = self.cwd / "ties"
            sub.mkdir()
            for name in ("🌱a", "a🌱"):
                (sub / name).touch()
            self.env["LC_ALL"] = "en_US.UTF-8"
            result = self.command(TONARI, "cd ties && printf '<%s>' *")
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, "<a🌱><🌱a>".encode(), b""))

    def test_nocaseglob_path_components_and_ignore(self):
        (self.cwd / "Sub").mkdir()
        (self.cwd / "bytes").mkdir()
        for name in ("A.ONE", "b.TWO", ".Hidden.ONE", "Ä.TXT", "雪.ONE", "🌱.ONE",
                     "Sub/K.TXT", "Sub/L*.TXT", "Sub/z.txt", "bytes/a🌱"):
            (self.cwd / name).touch()
        self.env.pop("LC_ALL")
        self.env.update(LC_COLLATE="C", LC_CTYPE="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8")
        script = r"""shopt -s extglob nocaseglob
printf '<%s>\n' a.one *.one .h*.one ä.* s*/k*.txt s*/l\**.txt s*/[k-l].txt
printf '<%s>\n' @([a-b]|no).@(one|two) [[:upper:]].one [[:lower:]].one
printf '<%s>\n' s*/K.TXT s*/k.txt
GLOBIGNORE='s*/k.txt:*.txt'
printf '<%s>\n' s*/*.txt
GLOBIGNORE='s*'
set -- s*/*.txt; printf '<%s>\n' "$@"
unset GLOBIGNORE
shopt -u nocaseglob
shopt -s nocasematch
printf '<%s>\n' *.one
shopt -s nocaseglob
printf '<%s>\n' ?.one
LC_ALL=C
printf '<%s>\n' ä.* ?.one ???.one ????.one
shopt -u nocaseglob
shopt -s nullglob
set -- bytes/@(a|xy)*[!a]; printf '<%s>\n' "$#"
GLOBIGNORE='bytes/*[!a]'; set -- bytes/*; printf '<%s>\n' "$#"
"""
        literal = "Sub/k.txt" if (self.cwd / "Sub/k.txt").exists() else "s*/k.txt"
        names = ("a.one", "A.ONE", "雪.ONE", "🌱.ONE", ".Hidden.ONE", "Ä.TXT", "Sub/K.TXT",
                 "Sub/L*.TXT", "Sub/K.TXT", "A.ONE", "b.TWO", "A.ONE", "[[:lower:]].one",
                 "Sub/K.TXT", literal, "Sub/L*.TXT", "Sub/z.txt", "s*/*.txt", "*.one",
                 "A.ONE", "雪.ONE", "🌱.ONE", "ä.*", "A.ONE", "雪.ONE", "🌱.ONE", "1", "0")
        self.compare(script, 0, "".join(f"<{name}>\n" for name in names).encode(), minimum=5)

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

    def test_inherited_trap_listing_and_restore(self):
        self.compare("""trap 'echo caught' USR1
save=$(trap -p USR1); printf '<%s>\\n' "$save"
( :; trap -p USR1; trap '' TERM; trap -p USR1 TERM )
trap - USR1; eval "$save"; trap -p USR1""", 0,
                     b"<trap -- 'echo caught' SIGUSR1>\ntrap -- 'echo caught' SIGUSR1\n"
                     b"trap -- '' SIGTERM\ntrap -- 'echo caught' SIGUSR1\n", minimum=5)

    def test_external_capture_and_environment(self):
        self.compare(r"""printf() { echo hidden; }; command() { echo shadow; }
echo "$(builtin printf '%s' real):$(command printf '%s' other)"
unset -f command
v=$(command /bin/sh -c 'echo out; echo silent >&2; exit 17' 2>/dev/null)
echo "$v:$?"
f() { builtin printf '\033x'; }; v=$(f); builtin printf '%s' "$v"
f() { builtin printf 'changed'; }; echo "$(f)"
""", 0, b"real:shadow\nout:17\n\x1bxchanged\n")
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

    def test_medium_here_inputs_preserve_every_byte_and_record_boundary(self):
        for size in (511, 512, 4095, 4096, 8193, 65535, 65536, 65537):
            data = b"0" * size + b"\n"
            script = f"printf -v text '%0*d' {size} 0\n"
            self.compare(script + 'cat <<EOF > out\n$text\nEOF\ncat out', 0, data)
            self.compare(script + 'v=$(cat <<<"$text"); printf "%s" "$v"', 0, data[:-1])
        # The known delimiter is beyond the 4 KiB read buffer. Escapes and a
        # later reader sharing the group input must still see the correct bytes.
        self.compare('printf -v a "%10000s" x; IFS= read -r -d : v <<< "$a:tail"; '
                     '[[ $v == "$a" ]] && echo ok', 0, b"ok\n")
        self.compare(r"""printf -v a '%6000s' x; v="$a"'\:t:tail'
{ IFS= read -d : x; [[ $x == "$a:t" ]] && cat; } <<<"$v" """, 0, b"tail\n")

    def test_counted_read_preserves_pipe_input_for_the_next_reader(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        text = "a" * 8191 + "雪🍵" + "b" * 917
        tail = b"\nrest\x00tail"
        for value, raw in [(text, text.encode()), ("🍵雪", b"\x00" + "🍵".encode() + b"\x00" + "雪".encode()),
                           ("", b"")]:
            (self.cwd / "data").write_bytes(raw + tail)
            group = f'{{ IFS= read -r -N {len(value)} value; printf "%s|%s" "$?" "$value"; /bin/cat; }}'
            for code in (f"/bin/cat data | {group}", f"{group} <data"):
                self.compare(code, 0, b"0|" + value.encode() + tail)

    def test_literal_file_capture_and_compound_redirect(self):
        (self.cwd / "data").write_bytes("雪\n🍵\n\n".encode())
        self.compare('v=$(<data); printf "<%s>:%s" "$v" "$?"', 0, "<雪\n🍵>:0".encode())
        self.compare('v=$({ printf group; } < data); printf "%s:%s" "$v" "$?"', 0, b"group:0")
        (self.cwd / "empty").touch()
        self.compare('v=$(<empty); echo "[$v]:$?"', 0, b"[]:0\n")
        (self.cwd / "nul").write_bytes(b"abc\0def\n\n")
        for shell in (BASH, TONARI):
            result = self.command(shell, 'v=$(<nul); printf "<%s>:%s" "$v" "$?"')
            self.assertEqual((result.returncode, result.stdout), (0, b"<abcdef>:0"))

    def test_literal_file_capture_reads_to_eof(self):
        # Preserve multibyte text after a long ASCII prefix and trim trailing newlines.
        value = b'x' * (8 * 1024 * 1024 - 1) + "🍵\nlast".encode()
        (self.cwd / "input").write_bytes(value + b"\n\n")
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                result = subprocess.run([shell, "-c", 'v=$(<input) && printf %s "$v" >out'],
                                        cwd=self.cwd, env=self.env, capture_output=True, timeout=30)
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b"", b""))
                self.assertEqual((self.cwd / "out").read_bytes(), value)
                (self.cwd / "out").unlink()
        os.mkfifo(self.cwd / "feed")
        self.compare('printf "from fifo\\n" >feed & v=$(<feed); rc=$?; wait; printf "%s:%s" "$v" "$rc"',
                     0, b"from fifo:0")
        if sys.platform.startswith("linux"):
            self.compare('v=$(</proc/sys/kernel/ostype); printf "%s:%s" "$v" "$?"', 0, b"Linux:0")
            (self.cwd / "self-stat").symlink_to("/proc/self/stat")
            self.compare('v=$(<self-stat); test "${v%% *}" = "$$"', 0)

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

    def test_braces_can_extend_an_unquoted_parameter_name(self):
        self.compare(r"""v=base; vx=X; vy=Y; vxz=XZ; v1=one; v2=two
printf '<%s>' $v{x,y} "${v}"{x,y} ${v}{x,y} $v{x,'y'} $v{1..2}; echo
printf '[%s]' {$v,q}x $v{x,y}z; echo
set +B; echo $v{x,y}""", 0, b"<X><Y><basex><basey><basex><basey><X><basey><one><two>\n"
                     b"[X][qx][XZ]\nbase{x,y}\n")

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
        script = """echo first >file; set -C
echo lost 2>/dev/null >file; echo "blocked:$?"; cat file
echo allowed >|file; cat file; : >/dev/null; echo "device:$?"
exec 3<>file; IFS= read -ru3 line; echo "read:$line"; printf 'more' >&3; exec 3>&-
exec 6<file; exec 7<&6-; read -ru7 line; printf 'moved:%s' "$line"
exec 8>|output; printf out 9>&8- >&9; { printf ok >&8; } 2>/dev/null; echo "/$?"; cat output"""
        for shell in self.shells_for():
            result = self.command(shell, script)
            ending = b"/0\noutok" if shell == TONARI else b"/1\nout"
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (0, b"blocked:1\nfirst\nallowed\ndevice:0\nread:allowed\nmoved:allowed" + ending, b""))

    def test_arithmetic_for_quotes_nested_separators_and_errors(self):
        self.compare("""for ((i=0; "i < 3"; i++)); do printf %s "$i"; done; echo
for ((i=$(printf 'a;' >/dev/null; echo 2); i>0; i--)); do printf %s "$i"; done; echo
{ for ((i=1; i < 4; 7++)); do echo "body:$i"; done; } 2>/dev/null; echo "step:$?"
{ for ((i=1; 4/0; i++)); do echo unexpected; done; } 2>/dev/null; echo "test:$?" """,
                     0, b"012\n21\nbody:1\nstep:1\ntest:1\n", minimum=5)

    def test_attached_kill_signal_and_monitor_switch(self):
        self.compare("""sleep 1 & p=$!; set -m; fg %1 2>/dev/null; echo "fg:$?"
kill -n9 "$p"; wait "$p" 2>/dev/null; echo "killed:$?"
set +m; sleep 1 & p=$!; kill -sHUP "$p"; wait "$p" 2>/dev/null; echo "hup:$?" """,
                     0, b"fg:1\nkilled:137\nhup:129\n", minimum=5)

    def test_monitor_mode_in_scripts(self):
        self.compare("""rm -f gate ack; mkfifo gate ack; set -m
reader() { IFS= read -r line <gate; }; reader & p=$!
g=$(python3 -c 'import os,sys; print(os.getpgid(int(sys.argv[1])))' "$p"); test "$g" = "$p" && echo group
(sleep .12; echo go >gate; IFS= read -r line <ack) & sender=$!
fg %1 >/dev/null; echo "fg:$?"; set +m; echo done >ack; wait "$sender"
shopt -qo monitor; echo "disabled:$?" """,
                     0, b"group\nfg:0\ndisabled:1\n", minimum=5)

    def test_logical_directory_after_external_rename(self):
        self.compare("""mkdir -p outer; cd outer; pwd -L >/dev/null; mv ../outer ../renamed
a=$(pwd -L); b=$(pwd -P); echo "${a##*/}/${b##*/}"; cd .. && mv renamed outer""",
                     0, b"outer/renamed\n")

    def test_physical_pwd_rechecks_replaced_and_unlinked_directory(self):
        proc = self.command(TONARI, """mkdir old; cd old; mv ../old ../new; mkdir ../old
actual=$(pwd -P); echo "${actual##*/}"; rmdir ../new
pwd -P >/dev/null 2>&1 && exit 1
echo unlinked""")
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"new\nunlinked\n", b""))

    def test_extended_patterns_in_conditionals_files_and_parameters(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for name in ("lib.rs", "lib.c", "lib.txt", ".hidden"):
            (self.cwd / name).touch()
        source = r"""shopt -u extglob
[[ 123 == +([[:digit:]]) ]] && echo digits
shopt -s extglob
for s in '' a aa ab b abb x; do case "$s" in +(a|b)) printf '<%s>' "$s";; esac; done; echo
printf '%s\n' lib.@(rs|c) @(.hidden|lib.txt)
s=abc123xyz; echo "${s##+(a|b|c)}" "${s//+([[:digit:]])/_}"
for s in a 'b|c' xyxx b 🍃; do [[ $s == @(a|"b|c"|+(x|y)|🍃) ]]; echo "$s:$?"; done
for s in '' a aa aab x; do [[ $s == !(a|aa)a ]]; echo "<$s:$?>"; done
p='!(lib.c)'; printf '<%s>' $p; echo; shopt -u extglob; printf '<%s>\n' $p"""
        self.compare(source, 0, ("digits\n<a><aa><ab><b><abb>\nlib.c\nlib.rs\n.hidden\nlib.txt\n"
                                 "123xyz abc_xyz\na:0\nb|c:0\nxyxx:0\nb:1\n🍃:0\n"
                                 "<:1>\n<a:0>\n<aa:1>\n<aab:1>\n<x:1>\n"
                                 "<lib.rs><lib.txt>\n<!(lib.c)>\n").encode(), minimum=5)

    def test_globignore_and_explicit_dot_entries(self):
        for name in ("one.tmp", "two", ".hidden", "a:b", "++"):
            (self.cwd / name).touch()
        (self.cwd / "dir").mkdir()
        (self.cwd / "dir" / "one.tmp").touch()
        self.compare(r"""shopt -u globskipdots
printf '[%s]' .*; echo
GLOBIGNORE='*.tmp:a\:b'; printf '[%s]' *; echo; printf '[%s]' dir/*; echo
GLOBIGNORE=; shopt -p dotglob; unset GLOBIGNORE; shopt -p dotglob
shopt -s extglob
GLOBIGNORE='+([^[:alnum:]])'; printf '[%s]' *; echo
shopt -u dotglob; list() { local GLOBIGNORE=two; printf '[%s]' *; echo; }; list; echo "$GLOBIGNORE"
printf '[%s]' !([*)* +(a|b[)*; echo
case '*(a|b[)' in *(a|b[)) echo literal;; esac""", 0,
                     b"[.][..][.hidden]\n[++][.hidden][dir][two]\n[dir/one.tmp]\n"
                     b"shopt -s dotglob\nshopt -u dotglob\n[.hidden][a:b][dir][one.tmp][two]\n"
                     b"[++][.hidden][a:b][dir][one.tmp]\n+([^[:alnum:]])\n"
                     b"[!([*)*][+(a|b[)*]\nliteral\n", minimum=5)

    def test_plain_star_followed_by_nullable_extended_group(self):
        script = r"""for p in '*!(a)' '*!(*)b' '*@(a|)' '?*@([ab]|*)'; do
for v in '' a b x aa; do [[ $v == $p ]] && printf 1 || printf 0; done; echo; done"""
        for shell in self.shells_for(5):
            # Bash accepts an empty subject despite the required trailing b.
            negative = b"10000" if shell == BASH else b"00000"
            proc = self.command(shell, script)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                             (0, b"10111\n" + negative + b"\n01001\n00001\n", b""))

    def test_generated_extended_patterns_against_bash(self):
        if BASH not in self.shells_for(5):
            self.skipTest("uses the Bash 5.x pattern matcher")
        rng = random.Random(42)
        def pattern(depth):
            def atom():
                if depth and rng.random() < .33:
                    return rng.choice("@!?+*") + "(" + "|".join(
                        pattern(depth - 1) for _ in range(rng.randint(1, 3))) + ")"
                return rng.choice(("a", "b", "?", "*", "[ab]", "[[:digit:]]"))
            return "".join(atom() for _ in range(rng.randrange(4)))
        patterns = ["x" + pattern(2) for _ in range(150)] + [pattern(2) for _ in range(250)]
        script = ("for p in " + shlex.join(patterns) + "; do "
                  "for v in '' a b aa ab abc abb 12 xa x1; do [[ $v == $p ]] && printf 1 || printf 0; done; echo; done")
        ref, proc = (self.command(shell, script) for shell in (BASH, TONARI))
        self.assertEqual((proc.returncode, proc.stderr, ref.returncode, ref.stderr), (0, b"", 0, b""))
        self.assertEqual(len(proc.stdout.splitlines()), len(patterns))
        for p, expected, actual in zip(patterns, ref.stdout.splitlines(), proc.stdout.splitlines(), strict=True):
            # None can accept empty: b and ? require input, and the middle
            # negation in the second pattern explicitly excludes empty.
            if p in ("*!(*)b", "*!(|@(|b)[ab][[:digit:]])*", "*!(b[ab])?"):
                expected = b"0" + expected[1:]
            self.assertEqual(actual, expected, p)

    def test_extglob_at_parse_time_and_invocation(self):
        for shell in (BASH, TONARI):
            proc = subprocess.run([shell, "-O", "extglob", "-c", "case b in @(a|b)) echo yes;; esac"],
                                  cwd=self.cwd, env=self.env, capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"yes\n", b""))
            self.assertEqual(self.command(shell, 'shopt -s extglob; case b in @(b)) :;; esac').returncode, 2)
            self.assertEqual(self.command(shell, 'shopt -s extglob\n( case b in @(a|b)) :;; esac )').returncode, 0)

    def test_directory_stack_rotation_and_no_cd(self):
        self.compare("""HOME=$PWD; mkdir -p a b; pushd a; pushd ../b; dirs -v
pushd +1; popd -0; popd; dirs -c; pushd -n c; echo "${DIRSTACK[1]}:${#DIRSTACK[@]}"
DIRSTACK[1]=changed; dirs +1; pushd no-such-path 2>/dev/null; echo "$?"; dirs
popd -n; dirs -p""", 0, b"~/a ~\n~/b ~/a ~\n 0  ~/b\n 1  ~/a\n 2  ~\n~/a ~ ~/b\n~/a ~\n~\n~ c\nc:2\nchanged\n1\n~ changed\n~\n~\n")

    def test_directory_stack_tildes_use_saved_entries_and_pwd(self):
        self.compare(r"""pushd -n 'saved path' >/dev/null
unset DIRSTACK; DIRSTACK=(fake 'fake path')
PWD=user; printf '<%s>\n' ~0 ~+1/file ~-0 ~-1 ~+2 ~--1 ~+99999999999999999999999
printf '<%s>\n' "${DIRSTACK[@]}"
unset PWD; printf '<%s>\n' ~0 ~1 ~-1
PWD=prefix v=~0; echo "$v"
dirs -c; printf '<%s>\n' ~-0 ~1 """, 0,
                     b"<user>\n<saved path/file>\n<saved path>\n<user>\n<~+2>\n"
                     b"<~--1>\n<~+99999999999999999999999>\n<fake>\n<fake path>\n"
                     b"<~0>\n<saved path>\n<~-1>\nprefix\n<prefix>\n<~1>\n")

    def test_directory_rotation_no_cd_and_failure_preserve_state(self):
        script = r"""HOME=$PWD; OLDPWD=original
pushd -n a >/dev/null; pushd -n b >/dev/null; pushd -n c >/dev/null
pushd -n +0; pushd -n -3; echo "$OLDPWD"; dirs
pushd -n +2; dirs; pushd -n -0; dirs
pushd +0 >/dev/null; [[ $OLDPWD = "$PWD" ]] && echo refreshed"""
        self.compare(script, 0, b"original\n~ c b a\n~ a ~ c\n~ ~ a ~\nrefreshed\n")
        # A nonexistent destination leaves both the cwd and every saved entry recoverable.
        proc = self.command(TONARI, "HOME=$PWD; pushd -n missing >/dev/null; "
                            "pushd -n other >/dev/null; pushd +2 2>/dev/null; echo $?; dirs")
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"1\n~ other missing\n", b""))

    def test_stack_physical_mode_and_failed_cd_status_output(self):
        (self.cwd / "real" / "child").mkdir(parents=True)
        (self.cwd / "link").symlink_to("real/child")
        self.compare(r"""start=$PWD; set -P; pushd link/.. >/dev/null || exit 9
[[ $PWD = "$start/real" && $OLDPWD = "$start" ]] || exit 8; popd >/dev/null
pushd -n link/.. >/dev/null; popd >/dev/null; [[ $PWD = "$start/real" ]] || exit 7
cd "$start"; shopt -s cdable_vars; destination=real; cd destination >&- 2>/dev/null
echo "$?"; [[ $PWD = "$start/real" && $OLDPWD = "$start" && $(pwd -P) = "$PWD" ]] && echo entered""",
                     0, b"1\nentered\n", minimum=5)

    def test_second_review_startup_status_and_script_search(self):
        first = self.cwd / "first"
        second = self.cwd / "second"
        first.mkdir()
        second.mkdir()
        (first / "sample").mkdir()
        (second / "sample").write_text('echo selected\n')
        self.env["PATH"] = f"{first}:{second}:/usr/bin:/bin"
        self.env["BASH_ENV"] = str(self.cwd / "startup")
        (self.cwd / "startup").write_text("false; return 7\n")
        (self.cwd / "fallback").mkdir()
        for shell in (BASH, TONARI):
            proc = subprocess.run([shell, "sample"], cwd=self.cwd, env=self.env,
                                  capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"selected\n"), proc.stderr)
            self.assertEqual(self.command(shell, 'echo "$?"').stdout, b"1\n")
            env = dict(self.env, BASH_ENV=str(self.cwd / "fallback"))
            proc = subprocess.run([shell, "-c", 'echo "$?"'], env=env, cwd=self.cwd,
                                  capture_output=True, timeout=5)
            self.assertEqual((proc.returncode, proc.stdout), (0, b"0\n"))
            self.assertIn(b"directory", proc.stderr.lower())

    def test_second_review_forced_interactive_history(self):
        hist = self.cwd / "entries"
        self.env["HISTFILE"] = str(hist)
        (self.cwd / "script").write_text("echo current\nhistory -s added\n")
        (self.cwd / "rc").write_text("history -s startup\nexit 3\n")
        for shell in ((BASH, TONARI) if bash_version() >= (5, 0) else (TONARI,)):
            for args, status, expected in [
                (["--norc", "-ic", "history -s added"], 0, "old\nadded\n"),
                (["--norc", "-i", "script"], 0, "old\necho current\nadded\n"),
                (["--rcfile", "rc", "-ic", ":"], 3, "old\nstartup\n"),
                (["--norc", "-i", "+o", "history", "script"], 0, "old\n"),
            ]:
                hist.write_text("old\n")
                proc = subprocess.run([shell, *args], cwd=self.cwd, env=self.env,
                                      input=b"", capture_output=True, timeout=5)
                self.assertEqual((proc.returncode, hist.read_text()), (status, expected), (shell, args, proc.stderr))

    def test_second_review_read_directory_and_format_operands(self):
        for shell in ((BASH, TONARI) if bash_version() >= (5, 0) else (TONARI,)):
            proc = self.command(shell, 'printf "data\\n" | { read -t 99999999999 v; echo "$?/$v"; }; '
                                'read -t 1e0 v <<<text; echo "$?"; '
                                'mkdir gone; cd gone; rmdir ../gone; cd .; echo "$?"')
            self.assertEqual((proc.returncode, proc.stdout), (0, b"0/data\n1\n0\n"), proc.stderr)
            self.assertIn(b"invalid timeout", proc.stderr)
            self.assertIn(b"retrieving current directory", proc.stderr)
        proc = self.command(TONARI, 'printf "%2147483648d|" 1; echo "$?"; '
                            'TZ=UTC0 printf "[%(%Y)T]/[%(%Y)T]" 9223372036854775807 0')
        self.assertEqual((proc.returncode, proc.stdout), (0, b"1\n[1970]/[1970]"), proc.stderr)

    def test_second_review_option_listing_and_directory_operands(self):
        controls = (BASH, TONARI) if bash_version() >= (5, 3) else (TONARI,)
        for shell in controls:
            for switch, expected in [("-o", b"braceexpand    \ton\n"), ("+o", b"set -o braceexpand\n")]:
                out = subprocess.run([shell, switch], cwd=self.cwd, env=self.env,
                                     input=b"echo body\n", capture_output=True, timeout=5)
                self.assertEqual(out.returncode, 0, out.stderr)
                self.assertIn(expected, out.stdout)
                self.assertTrue(out.stdout.endswith(b"body\n"))
            result = subprocess.run([shell, "+O", "nullglob", "-c", "shopt -p nullglob"],
                                    cwd=self.cwd, env=dict(self.env, BASHOPTS="nullglob"),
                                    capture_output=True, timeout=5)
            self.assertEqual((result.returncode, result.stdout), (0, b"shopt -s nullglob\n"))
            result = self.command(shell, "shopt extglob; shopt -o vi; dirs +999999999999999999999; "
                                  "echo $?; umask 7777; umask; umask u=r; umask")
            mask = b"7777" if sys.platform == "darwin" else b"0777"
            self.assertEqual(result.stdout, b"extglob             \toff\nvi                  \toff\n2\n"
                             + mask + b"\n0377\n")
            self.assertIn(b"invalid number", result.stderr)
            result = self.command(shell, """declare -A a=([a~]=1); declare -p a
unset a; declare -A a=([é]=2); declare -p a""")
            self.assertEqual(result.stdout, b'declare -A a=([a~]="1" )\n'
                             + b"declare -A a=([$'\\303\\251']=\"2\" )\n")

    def test_physical_cd_without_a_directory_name(self):
        for flags, status in (("-P", 0), ("-Pe", 1)):
            for mode in ("", "set -o posix; "):
                proc = self.command(TONARI, mode + 'mkdir gone; cd gone; rmdir ../gone; '
                                    + f'cd {flags} . 2>/dev/null; ' +
                                    'echo "$?/${PWD-absent}/${OLDPWD##*/}"; '
                                    '/usr/bin/printenv PWD || echo absent; '
                                    'cd "$HOME" && [[ $(/usr/bin/printenv PWD) = "$PWD" ]] && echo recovered')
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                 (0, f"{status}/absent/gone\nabsent\nrecovered\n".encode(), b""))

    def test_local_inheritance_copies_attributes_and_sparse_indices(self):
        self.compare("""declare -i count=9; array=([2]=two [7]=seven)
f() { local -I count; local -I array; count+=2; unset 'array[2]'; echo "$count:${!array[*]}"; }
f; echo "$count:${!array[*]}"
shopt -s localvar_inherit; g() { local array; array[0]=zero; echo "${!array[*]}"; }
g; echo "${!array[*]}" """, 0, b"11:7\n9:2 7\n0 2 7\n2 7\n", minimum=5)

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
        self.compare(r"""mapfile -d '' -t names < <(printf '%s\0' '日 報' $'line\nbreak' $'tab\t🐟' | cat)
printf '%s\0' "${names[@]}" """, 0, "日 報\0line\nbreak\0tab\t🐟\0".encode())

    def test_background_subshell_pid_and_stage_depth(self):
        self.compare("""(printf '%s:%s\\n' "$BASH_SUBSHELL" "$BASHPID" >result; exit 13) &
p=$!; wait "$p"; code=$?
IFS=: read -r depth pid <result
printf '%s:%s:%s/' "$code" "$depth" "$((p==pid))"
printf '%s' "$BASH_SUBSHELL" | cat; echo
( (printf '%s' "$BASH_SUBSHELL") ) & wait
""", 0, b"13:1:1/0\n2", minimum=4)

    def test_loop_rechecks_literal_builtin_resolution(self):
        self.compare("""for i in 1 2 3 4 5 6; do
    case $i in
        3) function echo { printf fn; };;
        4) unset -f echo;;
        5) set -x;;
        6) set +x;;
    esac
    echo -n X
done 2>trace
grep -q 'echo -n X' trace && printf '/traced/'
for i in 1 2 3 4; do false; echo -n "$?"; done
""", 0, b"XXfnXXX/traced/1111")

    def test_small_here_inputs_preserve_read_boundaries(self):
        self.compare(r"""IFS= read -d : x <<< 'one\:two:three'; printf '<%s>' "$x"
IFS= read -r -d : x <<< 'one\:two:three'; printf '<%s>' "$x"
IFS= read -r -n 1 x <<< 'abc'; printf '<%s>' "$x"
exec 7<<<'outside'; IFS= read -r -u 7 x <<< 'inside'; printf '<%s>' "$x"
for i in 1 2 3 4; do cat <<<'雪'; done""", 0, "<one:two><one\\><a><outside>雪\n雪\n雪\n雪\n".encode())

    def test_repeated_redirects_follow_shell_state(self):
        self.compare("""rm -f out
for i in 1 2 3 4 5; do
    case $i in 4) set -C;; 5) set +C;; esac
    { printf data >out; } 2>/dev/null
    printf %s "$?"
    exec 7<out; exec 6<&7-; IFS= read -r line <&6; exec 6<&-
    printf '<%s>' "$line"
done""", 0, b"0<data>0<data>0<data>1<data>0<data>")

    def test_positional_storage_and_live_loop_operands(self):
        self.compare("""set -- before 'two words'
f(){ set -- "$2" "$1" ""; printf '<%s>' "$@"; }; f a b; printf '<%s>' "$@"
for ((i=0; i<16#12; ++i)); do
    case $i in 0) i=010;; 9) i='target+1'; target=14;; 16) i=17;; esac
    printf ':%s' "$i"
done
echo ":$i"
declare -a v=([0]=1); for ((;v<4;v++)); do :; done; echo "${v[*]}"
declare -n ref=v; for ((;ref<6;ref++)); do :; done; echo "${v[*]}"; set --; echo "$#"
""", 0, b"<b><a><><before><two words>:010:target+1:17:18\n4\n6\n0\n", minimum=5)

    def test_output_only_pipeline_and_redirect_fallbacks(self):
        self.compare("""set -o pipefail; printf '%s\\n' a b |& cat; echo "${PIPESTATUS[*]}"
printf '%02000d' 0 | wc -c | tr -d ' '; printf -v name set | cat; echo "${name-unset}"
echo -ne 'a\\0b' | wc -c | tr -d ' '
printf '%s%%' one >f; printf '<%s>' two >>f; cat f
printf '%n' n >f; echo ":$n"; echo end
""", 0, b"a\nb\n0 0\n2000\nunset\n3\none%<two>:0\nend\n")

    def test_onecmd_string_and_script_boundaries(self):
        source = "set -t; printf one\nprintf two\n"
        self.compare(source, 0, b"onetwo")
        script = self.cwd / "onecmd"
        script.write_text(source)
        self.compare(". ./onecmd; printf three", 0, b"onetwothree")
        for binary in (BASH, TONARI):
            proc = subprocess.run([binary, script], cwd=self.cwd, env=self.env, capture_output=True)
            self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, b"one", b""))

    def test_raw_read_preserves_shared_offsets(self):
        (self.cwd / "input").write_bytes(b"x" * 8192 + b"\0" + "雪".encode() + b"|tail\nlast")
        self.env["LC_ALL"] = "en_US.UTF-8"
        self.compare("""exec 3<input
IFS= read -r -u3 -d '|' a; printf '%s/' "${#a}"
IFS= read -ru3 a; printf '%s/' "$a"; IFS= read -ru3 a; printf '%s:%s' "$?" "$a" """,
                     0, b"8193/tail/1:last")

    def test_mapfile_chunk_boundaries_and_sparse_edits(self):
        self.env["LC_ALL"] = "en_US.UTF-8"
        if not self.command(BASH, 'type -t mapfile').stdout:
            shells = (TONARI,)
        else:
            shells = (BASH, TONARI)
        script = """mapfile -t v <chunked; printf '%s/' "${#v[@]}" "${#v[0]}" "${#v[1]}" "${v[2]}" "${v[3]}"
v[1]=changed; unset 'v[0]'; v[20]=tail; printf '[%s]' "${!v[@]}" "${v[-1]}" """
        for block in (8192, 65536):
            # Place a delimiter at the end of one block and split 雪 across the next boundary.
            (self.cwd / "chunked").write_bytes(
                b"a" * (block - 1) + b"\n" + b"b" * (block - 1) + "雪\n".encode()
                + b"\nlast\0discarded")
            expected = f"4/{block - 1}/{block}//last/[1][2][3][20][tail]".encode()
            for shell in shells:
                with self.subTest(shell=shell, block=block):
                    proc = self.command(shell, script)
                    self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, expected, b""))

    def test_close_with_no_spare_file_descriptor(self):
        self.compare("""(ulimit -n 16; for fd in {3..15}; do eval "exec $fd>/dev/null" || exit 9; done
exec 6>&-; echo "close:$?"; exec 6>&7; echo "dup:$?"; exec 6>&7-; echo "move:$?") 2>/dev/null""",
                     0, b"close:1\ndup:0\nmove:1\n", minimum=5)

    def test_resource_limits_and_child_inheritance(self):
        src = """ulimit -Sn64; ulimit -Sn; ulimit -Sn 48 -n
python3 -c 'import resource; print(resource.getrlimit(resource.RLIMIT_NOFILE)[0])'
(ulimit -Sn 24; ulimit -n); ulimit -n
ulimit -Sn invalid 2>/dev/null; echo "invalid:$?"
ulimit -p 1 2>/dev/null; echo "pipe:$?" """
        self.compare(src, 0, b"64\nopen files                          (-n) 48\n48\n24\n48\ninvalid:1\npipe:1\n", minimum=5)
        # Resource values depend on the host and the Python test runner's limits.
        for mode in ("-a", "-Ha", "-Sn -Hn", "-p", "-v"):
            cmd = f"ulimit {mode}"
            ref = self.command(BASH, cmd)
            proc = self.command(TONARI, cmd)
            if BASH in self.shells_for(5):
                self.assertEqual((proc.returncode, proc.stdout, proc.stderr),
                                 (ref.returncode, ref.stdout, ref.stderr), mode)

    def test_times_includes_reaped_children(self):
        src = """python3 -c 'import time; start=time.process_time()
while time.process_time() - start < 0.035: pass'
times --"""
        for shell in (BASH, TONARI):
            proc = self.command(shell, src)
            self.assertEqual((proc.returncode, proc.stderr), (0, b""))
            rows = proc.stdout.splitlines()
            self.assertEqual(len(rows), 2)
            for row in rows:
                self.assertRegex(row, rb"^\d+m\d+\.\d{3}s \d+m\d+\.\d{3}s$")
            self.assertNotEqual(rows[1], b"0m0.000s 0m0.000s")
            self.assertEqual(self.command(shell, "times -z").returncode, 2)

    def test_umask_and_job_filters(self):
        self.compare("""umask 022; (umask 077; umask); umask -p
umask u=rwx,g=r,o=; umask -pS; : > formed; ls -l formed | cut -c 1-10""",
                     0, b"0077\numask 0022\numask -S u=rwx,g=r,o=\n-rw-r-----\n")
        self.compare("""sleep 0.12 & p=$!; jobs -s; jobs -r %1 >listing; awk '{$1=$1; print}' listing
test "$(jobs -p %1)" = "$p" && echo selected
jobs -l | grep -q "$p" && echo long; jobs -n; wait "$p" """,
                     0, b"[1]+ Running sleep 0.12 &\nselected\nlong\n")

    def test_executable_text_starts_with_fresh_shell_state(self):
        child = self.cwd / "plain script"
        child.write_text("""printf '%s/%s/%s/%s/%s\n' "$0" "$1" "$#" "${hidden-unset}" "${passed-unset}"
read -r word <&8; echo "$word"; echo message >&9
echo "$SHLVL"; exit 17
""")
        child.chmod(0o755)
        (self.cwd / "input").write_text("record\n")
        body = """hidden=local; exec 8<input 9>out
"./plain script" "a b"; echo "$?"; cat out"""
        self.compare(body, 0, b"./plain script/a b/1/unset/unset\nrecord\n2\n17\nmessage\n")
        self.compare("export passed=text; " + body, 0,
                     b"./plain script/a b/1/unset/text\nrecord\n2\n17\nmessage\n")
        self.compare("export -n SHLVL; " + body, 0,
                     b"./plain script/a b/1/unset/unset\nrecord\n1\n17\nmessage\n")

    def test_nested_executable_text_at_end_of_command(self):
        chain = self.cwd / "chain"
        chain.write_text("""if (( $1 )); then ./chain "$(($1 - 1))"; else
printf '%s/%s/%s' "$0" "${private-unset}" "$2"; exit 19; fi
""")
        chain.chmod(0o755)
        self.compare('private=parent; ./chain 40', 19, b"./chain/unset/")
        self.compare('private=parent; ./chain 0 "two words"', 19, b"./chain/unset/two words")

    def test_mapfile_origin_keeps_surrounding_and_sparse_indices(self):
        self.compare("""a=([0]=keep [2]=previous [4]=tail [81]=sparse)
mapfile -tO 2 a <<<'one
two'
printf '<%s>' "${!a[@]}" "${a[@]}"
mapfile -tO 400 empty </dev/null; declare -p empty""",
                     0, b'<0><2><3><4><81><keep><one><two><tail><sparse>declare -a empty=()\n', minimum=4)

    def test_current_shell_capture_with_closed_stdout_and_restrictive_umask(self):
        source = r"""umask 777
value=${ printf '<'; : 10>&-; printf '%131072s' x; printf '>\n\n'; return 7; }
printf '%s:%s:%s:%s\n' "$?" "${#value}" "${value:0:1}" "${value: -2}" >&8"""
        participants = (TONARI,) if self.bash_before_53() else (BASH, TONARI)
        for shell in participants:
            with self.subTest(shell=shell):
                result = self.command(shell, "exec 8>&1\n" + source)
                self.assertEqual((result.returncode, result.stdout, result.stderr),
                                 (0, b"7:131074:<:x>\n", b""))
        # Tonari also restores a previously closed stdout after capturing.
        result = self.command(TONARI, "exec 8>&1 1>&-\n" + source + "\n[[ ! -e /dev/fd/1 ]]")
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (0, b"7:131074:<:x>\n", b""))

    def test_command_substitution_capture_boundaries(self):
        self.compare("""x=outside; v=$(printf 'a'; echo -ne 'b\\nc'; printf ':'); echo "$v"
v=$(printf '%s' "$x"; false); echo "$?:$v"
v=$(printf -v x changed; echo "$x"); echo "$x:$v"
echo() { printf 'function:%s' "$1"; }; v=$(echo shadow); printf '%s' "$v" """,
                     0, b"ab\nc:\n1:outside\noutside:changed\nfunction:shadow")
        self.compare("""n=8; v=$(printf 'xy%n' n; printf '%s' "$n"); echo "$v:$n"
v=$(false; printf '%s' "$?"); echo "$v" """, 0, b"xy2:8\n1\n")

    def test_capturing_job_list_and_external_array_arguments(self):
        self.compare(r"""v=$(jobs -p); printf 'empty:%s:<%s>\n' "$?" "$v"
a=([2]="a b" [4]='' [9]='*.rs')
v=$(/usr/bin/printf '<%s>' "${a[@]}"); echo "$v"
declare -n ref=a; v=$(/usr/bin/printf '[%s]' "${ref[@]}"); echo "$v"
jobs() { printf '%s' "shadow:$1"; }; v=$(jobs -p); echo "$v" """,
                     0, b"empty:0:<>\n<a b><><*.rs>\n[a b][][*.rs]\nshadow:-p\n", minimum=5)

    def test_source_consecutive_function_boundaries(self):
        def definitions(names, body):
            return "".join(f"{name}() {{\n{body}\n}}\n" for name in names)

        names = ["first", "second", "third", "fourth", "fifth"]
        script = self.cwd / "defs"
        # Syntax errors later in the file must preserve earlier definitions.
        script.write_text(definitions(names, ":\n :") + "broken() { if; }\n")
        for shell in self.shells_for(5):
            p = self.command(shell, ". ./defs; declare -F fifth >/dev/null && echo registered")
            self.assertEqual(p.stdout, b"registered\n")
            self.assertTrue(p.stderr)

        script.write_text(definitions(names, ':\necho "$FUNCNAME:$LINENO"') +
                          "alias marker='echo expanded'\n" + definitions(["after"], "marker"))
        self.compare("shopt -s expand_aliases; . ./defs; first; fifth; after", 0,
                     b"first:3\nfifth:19\nexpanded\n", minimum=5)
        # Header-shaped text in a here-document is part of a single body.
        script.write_text("doc() {\ncat <<'END'\n}\n" +
                          definitions(names, "fake") + "END\n}\n" + definitions(names, "echo ok"))
        self.compare(". ./defs; fifth; doc | head -1", 0, b"ok\n}\n")

    def test_capture_readonly_expansions_and_helper_arguments(self):
        self.compare(r"""set -- 'a b' '' c
emit() { builtin printf '<%s>' "$@"; }
first() { echo "[$1:$#]"; }
path=a/b/c; val=日a; i=4
printf '%s\n' "$(emit "$@" "${path##*/}" "${missing:-ok}" "$((i+1))")"
printf '%s\n' "$(first)" "$(first "$@")" "$(emit "${val//a/b}" "${path:2:1}")"
echo "$#:$1:$i" """, 0, "<a b><><c><c><ok><5>\n[:0]\n[a b:3]\n<日b><b>\n3:a b:4\n".encode(),
                     minimum=4)

    def test_capture_keeps_arithmetic_writes_and_function_frames_in_child(self):
        self.compare(r"""set -- outer
i=3; expr='i++'; declare -n ref=i
frame() { printf '%s' "$FUNCNAME:$1"; }
inc() { echo "$((ref++))"; }
alt() { echo "${missing:=child}"; }
idx() { echo "$((a[i++]))"; }; a[3]=seven
printf '%s\n' "$(frame inner)" "$(inc)" "$(echo "$((expr))")" "$(alt)" "$(idx)"
printf '%s\n' "$i:${missing-unset}:$1"
""", 0, b"frame:inner\n3\n3\nchild\n0\n3:unset:outer\n", minimum=5)

    def test_capture_reports_prior_diagnostic_once_on_expansion_error(self):
        for shell in self.shells_for(5):
            for setup, word in [("", '"$((1/0))"'), ("shopt -s failglob; z='absent*';", "$z")]:
                with self.subTest(shell=shell, word=word):
                    proc = self.command(shell, setup + ' v=$(printf "%d" invalid; echo '
                                        + word + '; echo unreachable); echo "$?:$v"')
                    self.assertEqual(proc.returncode, 0)
                    self.assertEqual(proc.stdout, b"1:0\n")
                    self.assertEqual(sum(b"printf:" in line for line in proc.stderr.splitlines()), 1)

    def test_capture_with_child_trap(self):
        self.compare("""n=0; trap '((n++))' CHLD; set -- ok
v=$(echo "$1"); echo "$v:$n" """, 0, b"ok:1\n", minimum=5)

    def test_captured_string_formatter_with_stderr_redirect(self):
        self.compare(r"""set -- 'a b' ''; IFS=:
printf() { echo shadow; }
f() { command printf '%s\n' "$*" 2>/dev/null; }
g() { builtin printf '%d' invalid 2>/dev/null; }
v=$(f "$@" end); echo "$?:<$v>"
v=$(g); echo "$?:<$v>"
set -C; v=$(f again); echo "$?:<$v>"
exec 2>&-; v=$(f last); echo "$?:<$v>"
""", 0, b"0:<a b::end>\n1:<0>\n0:<again>\n0:<last>\n")

    def test_null_redirects_have_independent_open_file_descriptions(self):
        mutate = shlex.quote("import fcntl, os; fcntl.fcntl(1, fcntl.F_SETFL, os.O_NONBLOCK)")
        verify = shlex.quote("import fcntl, os, sys; flags=fcntl.fcntl(1, fcntl.F_GETFL); "
                             "sys.exit(bool(flags & (os.O_NONBLOCK | os.O_APPEND)))")
        python = shlex.quote(sys.executable)
        self.compare(f"{python} -c {mutate} >>/dev/null && {python} -c {verify} >/dev/null && echo ok",
                     0, b"ok\n")
        self.compare(f"{python} -c {mutate} >/dev/null && {python} -c {verify} >/dev/null && echo ok",
                     0, b"ok\n")

    def test_parent_traps_during_direct_captures(self):
        self.compare("""trap 'echo exit' EXIT; trap 'echo signal' USR1
v=$(echo hello | /bin/cat); echo "$v"
v=$(/bin/cat <<< "$v"); echo "$v"
f() { echo inner; }; v=$(f); echo "$v"
export parent=$$; v=$(echo value | /bin/sh -c 'kill -USR1 "$parent"; cat'); echo "$v" """,
                     0, b"hello\nhello\ninner\nsignal\nvalue\nexit\n")

    def test_captured_pipeline_and_here_input(self):
        self.compare("""v='two words'
a=$(printf '%s' "$v" | /bin/cat); echo "$a:$?"
a=$(echo -n | /bin/sh -c 'test -p /dev/stdin && echo pipe'); echo "$a"
a=$(/bin/cat <<< "$v"); echo "$a"
a=$(/bin/cat <<EOF
$v
EOF
); echo "$a"
v=before
a=$(/bin/cat <<< "${other:=inside}"); echo "$a/${other-unset}"
a=$(/bin/cat <<< "$BASH_SUBSHELL"); echo "$a"
set -o pipefail
a=$(echo one | /bin/sh -c 'cat >/dev/null; exit 7'); echo "$?"
echo() { builtin echo override; }
a=$(echo original | /bin/cat); builtin echo "$a" """,
                     0, b"two words:0\npipe\ntwo words\ntwo words\ninside/unset\n1\n7\noverride\n")

    def test_external_capture_observes_subshell_state(self):
        self.compare(r"""args=(value); declare -n depth=BASH_SUBSHELL; set -e
v=$(/usr/bin/printf '<%s>' "$depth" "${args[@]}"); echo "$v"
v=$(/usr/bin/printf '%s' "$BASH_COMMAND" "${args[@]}"); case "$v" in /usr/bin/printf*) echo command;; esac
export SHELLOPTS; args=(SHELLOPTS); v=$(/usr/bin/printenv "${args[@]}")
case ":$v:" in *:errexit:*) echo leaked;; *) echo cleared;; esac""",
                     0, b"<1><value>\ncommand\ncleared\n", minimum=5)

    def test_newline_filter_and_bracket_substitutions(self):
        self.env["LC_ALL"] = "en_US.UTF-8"
        self.compare(r"""v=$'🌸abc\na-71a\n雪'; printf '<%s>\n' "${v//[!$'\n']}"
v='a-71Bc]'; printf '<%s>\n' "${v//[a-c]}" "${v//[[:digit:]]}" "${v//[]-]}"
v=aba; printf '<%s>\n' "${v//[a]b}" "${v/#[a]}" "${v//[a]/x}" """,
                     0, b"<\n\n>\n<-71B]>\n<a-Bc]>\n<a71Bc>\n<a>\n<ba>\n<xbx>\n")

    def test_external_at_end_of_captured_function(self):
        self.compare(r"""f() { /bin/sh -c 'echo "$PPID"'; }
p=$(f); test "$p" = "$$" && echo direct
f() { local -x name="$1"; /bin/sh -c 'printf %s "$name"; exit 19'; }
name=outer; v=$(f 'two words'); echo "$?:$v:$name"
f() { /bin/echo before; echo after; }; v=$(f); echo "$v"
f() { trap 'echo leaving' RETURN; /bin/echo body; :; }; v=$(f); echo "$v" """,
                     0, b"direct\n19:two words:outer\nbefore\nafter\nbody\nleaving\n", minimum=5)

    def test_literal_external_wrapper_capture_and_fallback(self):
        self.compare(r"""f() { /bin/sh -c 'printf wrapped; exit 23'; }
v=$(f); printf '<%s:%s>' "$?" "$v"
v=$(f >/dev/null); printf '<%s:%s>' "$?" "$v"
f() { /usr/bin/printf '%s:%s' "$#" "$BASH_SUBSHELL"; }; v=$(f a b); printf '<%s>' "$v"
f() { /usr/bin/printf before; }; v=$(f); printf '<%s>' "$v"
f() { /usr/bin/printf after; }; v=$(f); printf '<%s>' "$v"
f() { /usr/bin/printenv SHELLOPTS; }; set -e; export SHELLOPTS; v=$(f)
case ":$v:" in *:errexit:*) echo leaked;; *) echo cleared;; esac""",
                     0, b"<23:wrapped><23:><2:1><before><after>cleared\n", minimum=5)

    def test_prompt_width_markers_and_literal_backslashes(self):
        self.compare(r"""p='日\[x\]\[y\]本'; printf '<%s>\n' "${p@P}"
p='\\[literal]$((3+4))'; printf '<%s>\n' "${p@P}" """,
                     0, "<日xy本>\n<\\[literal]7>\n".encode(), minimum=5)

    def test_styled_prompt_echo_and_midline_edit(self):
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, br"PS1='top\n\[\e[32m\]ready> \[\e[0m\]'" + b"\r")
                end = b"ready> \x1b[0m"
                self.read_until(fd, end)
                os.write(fd, b"echo ZX")
                echoed = self.read_until(fd, b"X")
                self.assertNotIn(end, echoed)
                os.write(fd, b"\x02Y\r")
                self.read_until(fd, b"ZYX\r\n")

    def test_parent_job_visibility_without_job_ownership(self):
        self.compare("""sleep 0.15 & p=$!
f() { jobs -p; }
test "$(jobs -p)" = "$p" && echo direct
test "$(true; f)" = "$p" && echo nested
test "$(jobs -p | cat)" = "$p" && echo pipe
(jobs -p); wait "$p"; echo "$?" """, 0, b"direct\nnested\npipe\n0\n")

    def test_posix_wait_reports_status_without_job_notice(self):
        sh = self.cwd / "sh"
        sh.symlink_to(TONARI)
        script = """/bin/sh -c 'kill -HUP $$' & p=$!
wait "$p"; code=$?; kill -l "$code"; exit "$code" """
        for argv, notice in (([TONARI], True), ([TONARI, "--posix"], False), ([sh], False)):
            with self.subTest(argv=argv):
                proc = subprocess.run([*argv, "-c", script], cwd=self.cwd, env=self.env,
                                      capture_output=True, timeout=5)
                self.assertEqual((proc.returncode, proc.stdout), (129, b"HUP\n"))
                if notice:
                    self.assertIn(b"Hangup", proc.stderr)
                else:
                    self.assertEqual(proc.stderr, b"")
        proc = self.command(sh, "wait 99999999")
        self.assertEqual(proc.returncode, 127)
        proc = self.command(sh, "wait %missing")
        self.assertEqual(proc.returncode, 127)
        self.assertIn(b"%missing: no such job", proc.stderr)

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
        gate = self.cwd / "release-wait-child"
        os.mkfifo(gate)
        source = """trap 'seen=yes' USR1
read -r line <release-wait-child & slow=$!; result=old; echo ready
wait -np result "$slow"; rc=$?
test "$seen" = yes && test "$rc" -gt 128 && echo "interrupted:${result-unset}"
wait "$slow"; echo "$?" """
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell), gate.open("r+b", buffering=0) as release:
                with subprocess.Popen([shell, "-c", source], cwd=self.cwd, env=self.env,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
                    try:
                        self.assertTrue(select.select([proc.stdout], [], [], 5)[0])
                        self.assertEqual(proc.stdout.readline(), b"ready\n")
                        # Keep the selected child blocked until wait has actually returned.
                        # A signal arriving between echo and wait may run its trap early.
                        deadline = time.monotonic() + 5
                        while time.monotonic() < deadline and proc.poll() is None:
                            proc.send_signal(signal.SIGUSR1)
                            if select.select([proc.stdout], [], [], 0.05)[0]:
                                break
                    finally:
                        release.write(b"go\n")
                    out, err = proc.communicate(timeout=5)
                    self.assertEqual((proc.returncode, out, err), (0, b"interrupted:unset\n0\n", b""))


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
        self.compare("alias ll='ls -alF'; type -t ll; shopt -s expand_aliases; "
                     "type -t ll; command -v ll; command -V ll", 0,
                     b"alias\nalias ll='ls -alF'\nll is aliased to `ls -alF'\n")
        self.compare("shopt -s expand_aliases\nalias a='printf yes | cat'\n"
                     "say() { a; }; unalias a; say; echo .", 0, b"yes.\n")
        self.compare("shopt -s expand_aliases\nalias headword='echo '; alias arg=expanded\n"
                     "headword arg; headword 'arg'", 0, b"expanded\narg\n")
        self.compare("shopt -s expand_aliases\nalias f='printf nested'\necho \"$(f)\"",
                     0, b"nested\n")

    def test_bash_combined_output_redirection_in_profiles(self):
        self.compare('f() { echo out; echo err >&2; }; if false; then true >&/dev/null; fi; '
                     'p="log file"; f >&"$p"; cat "$p"; f 1>&file; cat file; '
                     'f >log 2>&1; cat log', 0, b"out\nerr\n" * 3)
        self.compare('if false; then : 7>&not-a-number; fi; printf ok', 0, b"ok")

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

    def test_login_profile_precedence_and_logout(self):
        # The user's selected profile may source .bashrc itself.
        (self.cwd / ".bashrc").write_text("echo rc >> starts\n")
        (self.cwd / ".bash_logout").write_text("echo logout-$? >> ends\nfalse\n")
        for name in (".bash_profile", ".bash_login", ".profile"):
            (self.cwd / name).write_text(f'echo {name} >> starts\n'
                                         'if [ -n "$BASH_VERSION" ]; then . "$HOME/.bashrc"; fi\n')
        ends = self.cwd / "ends"
        starts = self.cwd / "starts"
        cmd = 'shopt -q login_shell || exit 8; trap \'echo trap-$? >> ends\' EXIT; false; logout 23'
        for name in (".bash_profile", ".bash_login", ".profile"):
            for shell in (BASH, TONARI):
                proc = subprocess.run([shell, "--login", "-c", cmd], cwd=self.cwd,
                                      env=self.env, capture_output=True, timeout=5)
                self.assertEqual(proc.returncode, 23, proc.stderr)
                self.assertEqual(starts.read_text().splitlines(), [name, "rc"])
                self.assertEqual(ends.read_text().splitlines(), ["logout-1", "trap-23"])
                starts.unlink()
                ends.unlink()
            (self.cwd / name).unlink()
        for shell in (BASH, TONARI):
            proc = self.command(shell, 'logout 2>/dev/null; echo $?')
            self.assertEqual((proc.stdout, proc.returncode), (b"1\n", 0))
            self.assertFalse(ends.exists())

    def test_distribution_login_profile(self):
        skel = Path("/etc/skel")
        if not Path("/etc/debian_version").exists() or not all(
                (skel / name).is_file() for name in (".bashrc", ".profile")):
            self.skipTest("Debian or Ubuntu distribution profiles required")
        for name in (".bashrc", ".profile"):
            (self.cwd / name).write_bytes((skel / name).read_bytes())
        profiles = []
        for shell in (BASH, TONARI):
            proc = subprocess.run([shell, "--login", "-ic",
                                   'echo "$SHLVL/$HISTCONTROL/$HISTSIZE"; type -t ll; :'],
                                  cwd=self.cwd, env=dict(self.env, TERM="dumb"),
                                  capture_output=True, timeout=5)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            if shell == TONARI:
                self.assertEqual(proc.stderr, b"")
            profiles.append(proc.stdout)
        self.assertEqual(profiles[0], profiles[1])
        self.assertIn(b"ignoreboth", profiles[0])

    def test_login_flags_and_plain_command_startup(self):
        (self.cwd / ".bash_profile").write_text("profile=on\n")
        (self.cwd / ".bashrc").write_text("rc=on\n")
        envfile = self.cwd / "envfile"
        envfile.write_text("envfile=on\n")
        env = dict(self.env, BASH_ENV=str(envfile))
        cmd = 'printf "%s/%s/%s:" "${profile-off}" "${rc-off}" "${envfile-off}"; shopt -q login_shell'
        for args, expected, code in ((["-c"], b"off/off/on:", 1),
                                     (["--login", "-c"], b"on/off/on:", 0),
                                     (["--noprofile", "--login", "-c"], b"off/off/on:", 0),
                                     (["-ic"], b"off/on/off:", 1)):
            for shell in (BASH, TONARI):
                with self.subTest(args=args, shell=shell):
                    p = subprocess.run([shell, *args, cmd], cwd=self.cwd, env=env,
                                       capture_output=True, timeout=5)
                    self.assertEqual((p.stdout, p.returncode), (expected, code), p.stderr)

    def test_terminal_login_argv0_and_control_d(self):
        (self.cwd / ".bash_profile").write_text(
            'shopt -q login_shell && echo "$0 $SHLVL" >> login-seen\nPS1="tonari$ "\n')
        (self.cwd / ".bash_logout").write_text("echo end >> logged-out\n")
        for shell in (BASH, TONARI):
            fd = self.terminal_session(shell=shell, login=True)
            os.write(fd, b"\x04")
            ready, _, _ = select.select([fd], [], [], 5)
            self.assertTrue(ready, "shell did not finish")
            while select.select([fd], [], [], 5)[0]:
                try:
                    if not os.read(fd, 4096):
                        break
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
            else:
                self.fail("login shell did not close its terminal")
        self.assertEqual((self.cwd / "login-seen").read_text().splitlines(), ["-bash 1"] * 2)
        self.assertEqual((self.cwd / "logged-out").read_text().splitlines(), ["end"] * 2)

    def test_interactive_history_defaults_follow_input_mode(self):
        source = 'printf "<%s/%s>" "$HISTSIZE" "$HISTFILESIZE"'
        for shell in (BASH, TONARI):
            for override, expected in [({}, b"500/500"), ({"HISTSIZE": "37"}, b"37/37"),
                                       ({"HISTSIZE": "37", "HISTFILESIZE": "8"}, b"37/8")]:
                env = dict(self.env, **override)
                proc = subprocess.run([shell, "--norc", "-is"], env=env, cwd=self.cwd,
                                      input=(source + "\n").encode(), capture_output=True, timeout=5)
                self.assertEqual((proc.stdout, proc.returncode), (b"<" + expected + b">", 0))
            proc = subprocess.run([shell, "--norc", "-ic", source], env=self.env, cwd=self.cwd,
                                  capture_output=True, timeout=5)
            self.assertEqual((proc.stdout, proc.returncode), (b"</>", 0))

    def test_secondary_prompt_eof_saves_history_and_runs_logout(self):
        (self.cwd / ".bash_profile").write_text(
            'PS1="tonari$ "; PS2="MORE> "; HISTFILE=$PWD/saved-history\n')
        (self.cwd / ".bash_logout").write_text("echo end >> logged-out\n")
        # Apple Bash 3.2 returns to input at this secondary-prompt EOF.
        for shell in ((BASH, TONARI) if bash_version() >= (5, 0) else (TONARI,)):
            fd = self.terminal_session(shell=shell, login=True)
            os.write(fd, b"if true; then\r")
            self.read_until(fd, b"MORE> ")
            os.write(fd, b"\x04")
            while select.select([fd], [], [], 5)[0]:
                try:
                    if not os.read(fd, 4096):
                        break
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
            else:
                self.fail("login shell did not close its terminal after PS2 EOF")
            self.assertEqual((self.cwd / "logged-out").read_text(), "end\n")
            self.assertIn("if true; then", (self.cwd / "saved-history").read_text().splitlines())
            (self.cwd / "logged-out").unlink()
            (self.cwd / "saved-history").unlink()

    def test_profile_version_gate_and_nested_shell_level(self):
        self.compare('test -n "$BASH_VERSION" && (( BASH_VERSINFO[0] >= 3 )) && '
                     'test "${#BASH_VERSINFO[@]}" = 6 && '
                     'test "${BASH_VERSINFO[5]}" = "$MACHTYPE" && printf available',
                     0, b"available")
        self.env["SHLVL"] = "4"
        self.compare('printf "%s " "$SHLVL"; "$BASH" --noprofile -c \'printf %s "$SHLVL"\'; :',
                     0, b"5 6")

    def test_completion_helper_scope_and_indexed_arithmetic(self):
        self.compare('g() { unset -v x; x=written; }; f() { local x=inside; g; echo "$x"; }; '
                     'x=outside; f; echo "$x"; x=outside; shopt -s localvar_unset; f; echo "$x"',
                     0, b"written\nwritten\nwritten\noutside\n", minimum=5)
        self.compare('eval -- "a=(4 8)"; i=0; ((a[i++] += 3)); ((0 && a[i++])); '
                     'echo "${a[*]}/$i/$((a[i-1]))"', 0, b"7 8/1/7\n")
        for shell in (BASH, TONARI):
            p = self.command(shell, "set -v\necho seen\nset +v\necho quiet")
            self.assertEqual((p.returncode, p.stdout, p.stderr),
                             (0, b"seen\nquiet\n", b"echo seen\nset +v\n"))
        self.compare('compgen -A shopt histapp; compgen -A setopt verb; '
                     'shopt -uq hostcomplete; shopt -q hostcomplete; echo $?',
                     0, b"histappend\nverbose\n1\n")


    def test_completion_name_references(self):
        self.compare('''v=old
        f() { local -n dest=$1; dest+=!; printf '<%s:%s>' "${!dest}" "$dest"; }
        f v; declare -gn ref=v; declare ref=new; g() { local ref=inner; echo "$ref/$v"; }; g
        echo "$ref/$v"; declare -n list=items; list=(first second); list[4]=last
        declare -n slot='items[1]'; printf -v slot %s changed; unset -n slot
        [[ -R list ]] && printf '<%s>' "${list[@]}" "${!list[@]}"
        unset ref; printf '|%s' "${v-gone}"''', 0,
                     b"<v:old!>inner/new\nnew/new\n<first><changed><last><0><1><4>|gone", minimum=5)

    def test_history_fc_and_explicit_file_roundtrip(self):
        self.compare("history -s first; history -s last; fc -lnr -2147483648; "
                     "history -d -2147483648 2>/dev/null; echo $?", 0,
                     b"\t last\n\t first\n1\n")
        self.compare('history -c; history -s echo first; history -s echo second; '
                     'history -p "!!:$"; fc -ln -2 -1; fc -s second=changed -1 2>/dev/null; '
                     'history -w hist; history -c; history -r hist; '
                     'history 1 | sed "s/^ *[0-9]* *//"', 0,
                     b"second\n\t echo first\n\t echo second\nchanged\necho changed\n")

    def terminal_session(self, editing=False, shell=TONARI, login=False, blocked=(), with_pid=False, ignored=()):
        pid, fd = pty.fork()
        if pid == 0:
            try:
                # A driver itself launched as an asynchronous shell command can
                # inherit SIG_IGN. Model a terminal emulator starting a new shell.
                signal.signal(signal.SIGINT, signal.SIG_DFL)
                signal.signal(signal.SIGQUIT, signal.SIG_DFL)
                for sig in ignored:
                    signal.signal(sig, signal.SIG_IGN)
                if blocked:
                    signal.pthread_sigmask(signal.SIG_BLOCK, blocked)
                os.chdir(self.cwd)
                env = dict(self.env, PS1=PROMPT.decode(), TERM="xterm-256color" if editing else "dumb")
                options = [] if editing else ["--noediting"]
                argv = ["-bash", *options] if login else [str(shell), "--norc", *options]
                os.execve(shell, argv, env)
            finally:
                os._exit(125)
        def close():
            try:
                finished = os.waitpid(pid, os.WNOHANG)[0] == pid
            except ChildProcessError:
                finished = True
            if finished:
                os.close(fd)
                return
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
        return (pid, fd) if with_pid else fd

    def test_variable_tab_uses_live_names_and_preserves_quoting(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"TNR_UNIQUE_VALUE=expanded; TNR_UNIQUE_HIDDEN=old; unset TNR_UNIQUE_HIDDEN; echo ready''OK\r")
                self.read_until(fd, b"readyOK\r\n")
                for word, expected in [(b"$TNR_UNI", b"expanded"), (b'"${TNR_UNI', b"expanded"),
                                       (b"'$TNR_UNI", b"$TNR_UNIQUE_VALUE"),
                                       (b"\\$TNR_UNI", b"$TNR_UNI")]:
                    os.write(fd, b"printf '<%s>\\n' " + word + b"\t\r")
                    self.read_until(fd, b"<" + expected + b">\r\n")
                os.write(fd, b"complete -W WRONG printf; echo spec''OK\r")
                self.read_until(fd, b"specOK\r\n")
                os.write(fd, b"printf '<%s>\\n' $TNR_UNI\t\r")
                self.read_until(fd, b"<>\r\n")

    def test_resize_during_edit_affects_the_command_being_entered(self):
        import struct
        import termios
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"printf ")
                self.read_until(fd, b"printf ")
                fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 41, 103, 0, 0))
                os.write(fd, b"'size<%sx%s>\\n' \"$LINES\" \"$COLUMNS\"\r")
                self.read_until(fd, b"size<41x103>\r\n")

    def test_motion_burst_stops_before_custom_binding_and_child_input(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"""bind '"\\C-f":beginning-of-line'; echo configured''OK\r""")
        self.read_until(fd, b"configuredOK\r\n")
        os.write(fd, b"v=abc\x02\x06#\r")
        os.write(fd, b"printf '<%s>\\n' \"${v-unset}\"\r")
        self.read_until(fd, b"<unset>\r\n")
        # The command is accepted by the editor. Its foreground reader takes a
        # newline-delimited record already queued while the terminal is raw.
        os.write(fd, b"read -r v\x02\x1b[C\rchild payload\nprintf '<%s>\\n' \"$v\"\r")
        self.read_until(fd, b"<child payload>\r\n")

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

    def test_disown_and_hangup_on_a_controlling_terminal(self):
        helper = self.cwd / "worker.py"
        helper.write_text("""import pathlib, signal, sys, time
name = sys.argv[1]
def hung_up(*_):
    pathlib.Path(name + '.hup').touch()
    sys.exit(0)
signal.signal(signal.SIGHUP, hung_up)
pathlib.Path(name + '.ready').touch()
for _ in range(500):
    if pathlib.Path(name + '.release').exists():
        pathlib.Path(name + '.alive').touch()
        break
    time.sleep(.01)
""")
        def await_files(paths):
            end = time.monotonic() + 4
            while time.monotonic() < end and not all(path.exists() for path in paths):
                time.sleep(.01)
            self.assertTrue(all(path.exists() for path in paths), paths)

        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell)
                pid = os.tcgetpgrp(fd)
                names = [f"{pid}-{kind}" for kind in ("owned", "marked", "removed")]
                for name in names:
                    os.write(fd, f"{shlex.quote(sys.executable)} worker.py {name} &\n".encode())
                    self.read_until(fd, PROMPT)
                await_files([self.cwd / (name + ".ready") for name in names])
                os.write(fd, b"disown -h %2; disown %3; jobs -p >listed\n")
                self.read_until(fd, PROMPT)
                self.assertEqual(len((self.cwd / "listed").read_bytes().splitlines()), 2)
                os.kill(pid, signal.SIGHUP)
                await_files([self.cwd / (names[0] + ".hup")])
                for name in names[1:]:
                    (self.cwd / (name + ".release")).touch()
                await_files([self.cwd / (name + ".alive") for name in names[1:]])
                deadline = time.monotonic() + 4
                while time.monotonic() < deadline:
                    reaped, status = os.waitpid(pid, os.WNOHANG)
                    if reaped:
                        self.assertEqual(os.waitstatus_to_exitcode(status), -signal.SIGHUP)
                        break
                    time.sleep(.01)
                else:
                    self.fail("interactive shell did not exit after SIGHUP")

    def test_disown_job_id_pid_and_running_filter(self):
        self.compare("""exec 2>/dev/null
sleep .1 & p=$!; disown "$p"; jobs -p; wait "$p"; echo pid:$?
sleep .1 & sleep .1 & disown -r; jobs -p; echo all:$?
sleep .1 & p=$!; disown -h; test "$(jobs -p)" = "$p"; echo kept:$?; wait
disown %404; echo missing:$?
""", 0, b"pid:0\nall:0\nkept:0\nmissing:1\n", minimum=5)

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

    def test_foreground_executable_receives_terminal_before_read(self):
        helper = self.cwd / "first-read"
        helper.write_text(f"#!{sys.executable}\n" + """import os
assert os.tcgetpgrp(0) == os.getpgrp() == os.getpid()
print("reader-ready", flush=True)
print("read=" + input(), flush=True)
""")
        helper.chmod(0o755)
        for shell in (TONARI, BASH):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell)
                os.write(fd, b"./first-read\n")
                self.read_until(fd, b"reader-ready\r\n")
                os.write(fd, b"hello\n")
                self.assertIn(b"read=hello\r\n", self.read_until(fd, PROMPT))

    def test_single_foreground_job_stops_and_resumes(self):
        helper = self.cwd / "stop-once"
        helper.write_text(f"#!{sys.executable}\n" + """import os, signal
os.kill(os.getpid(), signal.SIGSTOP)
assert os.tcgetpgrp(0) == os.getpgrp()
print("resumed-once")
raise SystemExit(23)
""")
        helper.chmod(0o755)
        for shell in (TONARI, BASH):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell)
                os.write(fd, b"./stop-once marker\n")
                self.read_until(fd, PROMPT)
                os.write(fd, b"jobs\n")
                jobs = self.read_until(fd, PROMPT)
                self.assertIn(b"Stopped", jobs)
                self.assertIn(b"./stop-once marker", jobs)
                os.write(fd, b"fg\n")
                self.assertIn(b"resumed-once\r\n", self.read_until(fd, PROMPT))
                os.write(fd, b"echo status=$?\n")
                self.assertIn(b"status=23\r\n", self.read_until(fd, PROMPT))

    @unittest.skipUnless(sys.platform == "darwin", "Darwin suspended-spawn signal mask")
    def test_foreground_does_not_inject_blocked_sigcont(self):
        helper = self.cwd / "pending"
        helper.write_text(f"#!{sys.executable}\n" + """import signal
assert signal.SIGCONT in signal.pthread_sigmask(signal.SIG_BLOCK, [])
print("pending=" + str(signal.SIGCONT in signal.sigpending()))
""")
        helper.chmod(0o755)
        for shell in (TONARI, BASH):
            with self.subTest(shell=shell):
                fd = self.terminal_session(shell=shell, blocked=[signal.SIGCONT])
                os.write(fd, b"./pending\n")
                self.assertIn(b"pending=False\r\n", self.read_until(fd, PROMPT))

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
        send(b"\x1b?", b"item-as  item-at  item-au")
        send(clear + b": x-l\tmore", PROMPT + b": x-link/more")
        send(clear + b"tool-\targ", PROMPT + b"tool-allowed arg")
        send(clear + b"complete -d -o nospace -o default demo\r", b"\x1b[?2004h")
        send(b"demo safe\tmore", PROMPT + b"demo safe\\ path/more")
        send(clear + b"demo item-at\targ", PROMPT + b"demo item-at arg")
        send(clear + b"complete -W 'start stop start' demo\r", b"\x1b[?2004h")
        send(b"demo s\t", PROMPT + b"demo st")
        listing = send(b"\t", b"  stop")
        self.assertIn(b"> start\n  stop", listing.replace(b"\r", b""))

    def test_picker_filters_selects_and_preserves_the_draft(self):
        for name in ["pick-apple", "pick-red apple🐟", "pick-red pear🐟", "pick-dollar$(bad)"]:
            (self.cwd / name).touch()
        fd = self.terminal_session(editing=True)
        def send(keys, marker):
            os.write(fd, keys)
            return self.read_until(fd, marker)
        send(b"printf '%s' pick-\t\t", b"> pick-apple")
        send(b"ZZZ", b"(no matches)")
        send(b"\r", b"(no matches)")
        send(b"\x7f\x7f\x7f" + "🐟".encode(), "> pick-red apple🐟".encode())
        send(b"\x1b[B", "> pick-red pear🐟".encode())
        send(b"\r", "pick-red\\ pear🐟 ".encode())
        self.assertFalse((self.cwd / "chosen").exists())
        send(b">chosen\r", b"\x1b[?2004h")
        self.assertEqual((self.cwd / "chosen").read_text(), "pick-red pear🐟")
        send(b"v=original; : pick-\t\t", b"> pick-apple")
        send(b"\x1b[200~red pear\x1b[201~", "> pick-red pear🐟".encode())
        send(b"\x07", b"v=original; : pick-")
        send(b"apple; printf %s \"$v\" > restored\r", b"\x1b[?2004h")
        self.assertEqual((self.cwd / "restored").read_text(), "original")
        send(b"printf '%s' pick-\t\t", b"> pick-apple")
        send(b"dollar\r", b"pick-dollar\\$\\(bad\\) ")
        send(b">literal\r", b"\x1b[?2004h")
        self.assertEqual((self.cwd / "literal").read_text(), "pick-dollar$(bad)")

    def test_large_file_completion_keeps_names_and_byte_order(self):
        folder = self.cwd / "files"
        folder.mkdir()
        names = [f"long-prefix-{i % 11:02d}-雪🐟-{i:04d}" for i in range(1150)]
        names.extend(["long-prefix", "long-prefix-", "long-prefix-0"])
        for name in reversed(names):
            (folder / name).touch()
        expected = sorted(("files/" + n).encode() for n in names)
        for shell in (BASH, TONARI):
            with self.subTest(shell=shell):
                proc = self.command(shell, "compgen -f -- files/")
                self.assertEqual(proc.returncode, 0, proc.stderr)
                actual = proc.stdout.splitlines()
                self.assertEqual(actual if shell == TONARI else sorted(actual), expected)

    def test_picker_pages_and_restores_midline_cursor(self):
        for i in range(24):
            (self.cwd / f"paper-{i:02d}").touch()
        fd = self.terminal_session(editing=True)
        os.write(fd, b"printf '%s' paper- >chosen" + b"\x02" * 8 + b"\t\t")
        self.read_until(fd, b"> paper-00")
        os.write(fd, b"paper-1" + b"\t" * 8)
        self.read_until(fd, b"> paper-18")
        os.write(fd, b"\x1b[Z")
        self.read_until(fd, b"> paper-17")
        # Split the down arrow between reads and send an unbound CSI sequence.
        os.write(fd, b"\x1b[")
        time.sleep(0.02)
        os.write(fd, b"B\x1b[3~\r")
        self.read_until(fd, b"paper-18 >chosen")
        self.assertFalse((self.cwd / "chosen").exists())
        os.write(fd, b"X\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "chosen").read_text(), "paper-18X")
        os.write(fd, b"printf '%s' paper- >cancelled" + b"\x02" * 11 + b"\t\t")
        self.read_until(fd, b"> paper-00")
        os.write(fd, b"19")
        self.read_until(fd, b"> paper-19")
        # Escape alone restores the draft; a later key goes at its original cursor.
        os.write(fd, b"\x1b")
        self.read_until(fd, b"paper- >cancelled")
        os.write(fd, b"end\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "cancelled").read_text(), "paper-end")

    def test_picker_respects_programmable_completion_and_interrupt(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b'pick() { printf %s "$1" >arg; }; words() { echo call >>calls; '
                     b'COMPREPLY=(alpha beta); }; complete -o nospace -F words pick\r')
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"pick \t\t")
        self.read_until(fd, b"> alpha")
        calls = (self.cwd / "calls").read_bytes()
        os.write(fd, b"be")
        self.read_until(fd, b"> beta")
        os.write(fd, b"\r-more\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "arg").read_text(), "beta-more")
        self.assertEqual((self.cwd / "calls").read_bytes(), calls)
        os.write(fd, b"echo executed >discarded; pick \t\t")
        self.read_until(fd, b"> alpha")
        os.write(fd, b"\x03")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"echo ready >resumed\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertFalse((self.cwd / "discarded").exists())
        self.assertEqual((self.cwd / "resumed").read_text(), "ready\n")

    def test_picker_ranks_history_within_the_same_context(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        rows = [(0, "tool --model cedar")] * 4 + [(0, "tool --model birch")] * 2
        rows += [(0, "tool --model alder --model alder --model alder"), (0, "tool --model birch-2"), (1, "tool --model failed")]
        journal.write_text("".join(f"1\t1\t{code}\t{self.cwd}\t{cmd}\n" for code, cmd in rows))
        fd = self.terminal_session(editing=True)
        os.write(fd, b"tool --model \t\t")
        out = self.read_until(fd, b"  alder").replace(b"\r", b"")
        self.assertIn(b"> cedar\n  birch\n  birch-2\n  alder", out)
        os.write(fd, b"bir\x1b[B\r")
        self.read_until(fd, PROMPT + b"tool --model birch-2 ")
        os.write(fd, b"\x15complete -o nosort -W 'zeta zeta alpha amber alpha' tool\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"tool \t\t")
        out = self.read_until(fd, b"  amber").replace(b"\r", b"")
        self.assertIn(b"> zeta\n  alpha\n  amber", out)

    def test_completion_keeps_terminal_input_with_its_owner(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"""pick() { /bin/sh -c 'printf "owner-%s>" ready >&2; IFS= read -r answer; printf %s "$answer" > owned'; COMPREPLY=(match); }; complete -F pick demo\r""")
                self.read_until(fd, b"\x1b[?2004h")
                os.write(fd, b"demo ma\t")
                self.read_until(fd, b"owner-ready>")
                os.write(fd, b"reply\n")
                self.read_until(fd, b"tch ")
                self.assertEqual((self.cwd / "owned").read_bytes(), b"reply")

    def test_completion_echoes_edits_before_a_helper_finishes(self):
        fd = self.terminal_session(editing=True)
        for mode in ("foreground", "capture", "shell loop"):
            with self.subTest(mode=mode):
                # The helper cannot exit until the test has observed both the IME
                # commit and a separate Backspace. No wall-clock threshold.
                gate = self.cwd / "release"
                gate.unlink(missing_ok=True)
                wait = """/bin/sh -c 'printf "waiting-%s>" ready >&2; while [ ! -f release ]; do /bin/sleep 0.01; done'"""
                if mode == "capture":
                    wait = f"ignored=$({wait})"
                elif mode == "shell loop":
                    wait = """printf 'waiting-%s>' ready >&2; while [[ ! -e release ]]; do :; done"""
                cmd = f'calls=0; pick() {{ ((calls++)); {wait}; COMPREPLY=(match); }}; demo() {{ printf "%s|%s|%s" "$1" "$2" "$calls" > completed; }}; complete -o typeahead -F pick demo'
                os.write(fd, cmd.encode() + b"\r")
                self.read_until(fd, b"\x1b[?2004h")
                os.write(fd, b"demo ma\t")
                self.read_until(fd, b"waiting-ready>")
                os.write(fd, "日本🌱".encode())
                self.read_until(fd, PROMPT + "demo ma日本🌱".encode())
                os.write(fd, b"\x7f!")
                self.read_until(fd, PROMPT + "demo ma日本!".encode())
                gate.touch()
                os.write(fd, b"\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "completed").read_text(), "match|日本!|1")

    def test_abbreviated_paths_and_project_arguments(self):
        kernel = self.cwd / "src/backend/cuda/kernel file🌱"
        kernel.parent.mkdir(parents=True)
        kernel.touch()
        (self.cwd / ".git").mkdir()
        (self.cwd / "build/4070ti-release").mkdir(parents=True)
        (self.cwd / "other").mkdir()
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        journal.write_text(f"1\t1\t0\t{self.cwd}\tcmake --build build/4070ti-release\n"
                           f"1\t2\t0\t{self.cwd}/other\trun --model mistral-large\n"
                           f"1\t3\t1\t{self.cwd}\trun --model mistral-failed\n"
                           f"1\t4\t0\t{self.cwd.parent}\trun --model mistral-unrelated\n"
                           f"1\t5\t0\t{self.cwd}\trun --model $(touch EVALUATED)\n")
        fd = self.terminal_session(editing=True)
        def edit(line, expected):
            os.write(fd, b"\x15" + line + b"\t")
            self.read_until(fd, PROMPT + expected)
        edit(b": s/b/c/ker", ": src/backend/cuda/kernel\\ file🌱 ".encode())
        edit(b"cd s/b/cu", b"cd src/backend/cuda/")
        (self.cwd / "src/build/core").mkdir(parents=True)
        (self.cwd / "src/build/core/kernel-b").touch()
        os.write(fd, b"\x15: s/b/c/ker\t\t")
        self.read_until(fd, b"src/build/core/kernel-b")
        edit(b": ~/s/build/co/ke", b": ~/src/build/core/kernel-b ")
        (self.cwd / "src/build/.hidden").mkdir()
        (self.cwd / "src/build/.hidden/private").touch()
        edit(b": s/build/.h/pr", b": src/build/.hidden/private ")
        (self.cwd / "src/link").symlink_to("backend")
        edit(b": s/l/c/ker", ": src/link/cuda/kernel\\ file🌱 ".encode())
        edit(b"ctest --test-dir 4070", b"ctest --test-dir build/4070ti-release/")
        edit(b"run --model ", b"run --model mistral-large ")
        (self.cwd / "mistral-local").touch()
        edit(b"run --model mist", b"run --model mistral-local ")
        os.write(fd, b"\x15complete -W unrelated run\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"run --model mist\t@")
        self.read_until(fd, PROMPT + b"run --model mist@")
        self.assertFalse((self.cwd / "EVALUATED").exists())

    def test_alias_completion_context_and_reverse_search_caret(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"""shopt -s progcomp_alias; alias g='real sub'; cb(){ printf '%s|%s|%s|%s|%s' "$1" "$3" "$COMP_LINE" "$COMP_CWORD" "$COMP_POINT" > ctx; COMPREPLY=(match); }; complete -F cb real\r""")
                self.read_until(fd, b"\x1b[?2004h")
                os.write(fd, b"g ma\t")
                self.read_until(fd, b"tch ")
                self.assertEqual((self.cwd / "ctx").read_text(), "real|sub|real sub ma|2|11")
                os.write(fd, b"\x15v=AC\x02\x12missing\x07B\x05; printf %s \"$v\" > caret\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "caret").read_text(), "ABC")

        fd = self.terminal_session(editing=True)
        os.write(fd, b"alias g=real; complete -W original real\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"g or\t")
        self.read_until(fd, PROMPT + b"g original ")
        os.write(fd, b"\x15complete -W override g\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"g ov\t")
        self.read_until(fd, PROMPT + b"g override ")
        os.write(fd, b"\x15complete -r g; shopt -u progcomp_alias\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"g or\t@")
        self.read_until(fd, PROMPT + b"g or@")

    def test_long_line_appends_without_reprinting_the_paste(self):
        fd = self.terminal_session(editing=True)
        payload = b"x" * 4080
        os.write(fd, b"\x1b[200~v=" + payload + b"Z\x1b[201~")
        self.read_until(fd, b"Z")
        os.write(fd, "日本🌱!".encode())
        display = self.read_until(fd, b"!")
        self.assertLess(len(display), 64)
        os.write(fd, b"\x7f@")
        self.read_until(fd, b"@")
        os.write(fd, b"\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b'printf %s "$v" > value\r')
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "value").read_bytes(), payload + "Z日本🌱@".encode())

    def test_stalled_paste_is_visible_before_the_closing_marker(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"v='!'\x02\x02\x1b[200~" + "日本".encode() + b"\xf0\x9f")
        partial = self.read_until(fd, PROMPT + "v='日本!'".encode())
        self.assertNotIn("�".encode(), partial)
        os.write(fd, b"\x8c\xb1\x1b[201~\x05; printf %s \"$v\" > result\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "result").read_text(), "日本🌱!")

    def test_large_multiline_viewport_and_cursor_edits(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"""save(){ printf %s "$READLINE_LINE" > snapshot; printf 'saved<%s>' yes >&2; }; bind -x '"\\C-x!":save'\r""")
        self.read_until(fd, b"\x1b[?2004h")
        # Write in pieces as a real terminal would. The editor only paints the
        # visible neighbourhood even when there are thousands of hard breaks.
        text = "BEGIN " + ("日本🌱 xyz\n" * 3000) + "last Ω"
        packet = b"\x1b[200~" + text.encode() + b"\x1b[201~"
        while packet:
            packet = packet[os.write(fd, packet[:2048]):]
        drawn = self.read_until(fd, "Ω".encode())
        self.assertLess(max(map(len, drawn.split(b"\x1b[J"))), 4096)
        os.write(fd, b"\x01")
        # Home finishes its redraw by positioning the caret after the ASCII
        # prompt. A PTY read may end anywhere within the preceding viewport.
        head = self.read_until(fd, b"\r\x1b[%dC" % len(PROMPT))
        self.assertIn(b"BEGIN ", head)
        self.assertLess(len(head), 4096)
        os.write(fd, b"\x06")
        moved = self.read_until(fd, b"\x1b[1C")
        self.assertLess(len(moved), 32)
        os.write(fd, b"\x1b[3~" + "雪".encode() + b"\x05\x7f@\x18!")
        self.read_until(fd, b"saved<yes>")
        self.assertEqual((self.cwd / "snapshot").read_text(), "B雪" + text[2:-1] + "@")

    def test_overwrite_copy_and_yank_against_readline(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"""bind '"\\e[2~": overwrite-mode'; bind '"\\ew": copy-region-as-kill'\r""")
                self.read_until(fd, b"\x1b[?2004h")
                for edit, expected in [
                    ("東京".encode(), "A東京D"), (b"X\x7f", "A CD"),
                    (b"\x1b[200~xy\x1b[201~", "AxyBCD"),
                ]:
                    os.write(fd, b"v='ABCD'" + b"\x02" * 4 + b"\x1b[2~" + edit
                             + b'\x05; printf %s "$v" > result\r')
                    self.read_until(fd, b"\x1b[?2004h")
                    self.assertEqual((self.cwd / "result").read_text(), expected)
                os.write(fd, b"v='abc'" + b"\x02\0\x02\x02\x1bw\x19\x05; printf %s \"$v\" > result\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "result").read_text(), "abcbc")

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
        enter(other, b"HISTIGNORE='*ignore-this-command*'; echo ignore-this-command > /dev/null")
        journal = self.cwd / ".local/state/tonari/history"
        self.assertNotIn("ignore-this-command", journal.read_text())
        self.assertNotIn("omit-secret", journal.read_text())
        self.assertFalse((self.cwd / ".bash_history").exists())
        # Remove a complete UTF-8 character and insert a replacement at the caret.
        enter(fd, 'echo 猫雪'.encode() + b'\x1b[D\x7f' + '犬'.encode() + b'\x1b[F > unicode')
        self.assertEqual((self.cwd / "unicode").read_text(), "犬雪\n")

    def test_glob_uses_current_shell_locale_and_invalidates_sort_order(self):
        files = self.cwd / "names"
        files.mkdir()
        for i in range(512):
            (files / f"{['a', 'Z', 'Å', 'é', '雪', '🌱'][i % 6]}{i:03}").touch()
        code = r'''LC_ALL=C
printf '%s\0' names/*; printf 'END\0'
f() { local LC_ALL=en_US.UTF-8; printf '%s\0' names/*; }
f; printf 'END\0'; printf '%s\0' names/*'''
        ref = self.command(BASH, code)
        self.assertEqual(ref.returncode, 0, ref.stderr)
        a, b, c = ref.stdout.split(b"END\0")
        self.assertEqual(a, c)
        result = self.command(TONARI, code)
        self.assertEqual((result.returncode, result.stderr), (0, b""))
        if sys.platform.startswith("linux"):
            byte_order = b"".join(name + b"\0" for name in sorted(
                os.fsencode(p.relative_to(self.cwd)) for p in files.iterdir()))
            self.assertEqual(result.stdout, b"END\0".join([byte_order] * 3))
        else:
            self.assertNotEqual(a, b)
            self.assertEqual(result.stdout, ref.stdout)

    def test_large_globs_match_reference_order_after_directory_changes(self):
        directory = self.cwd / "files"
        directory.mkdir()
        names = [f"{['Z', 'å', '雪', 'é', 'a', '🌱'][i % 6]}-{i:05}.txt" for i in range(4241)]
        random.Random(971).shuffle(names)
        for name in names:
            (directory / name).touch()
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        source = r'printf "%s\0" files/*.txt; : > files/new.txt; printf "%s\0" files/*.txt'
        reference = self.command(BASH, source)
        self.assertEqual(reference.returncode, 0, reference.stderr)
        self.assertEqual(reference.stdout.count(b"\0"), len(names) * 2 + 1)
        (directory / "new.txt").unlink()
        result = self.command(TONARI, source)
        self.assertEqual((result.returncode, result.stderr), (0, b""))
        self.assertEqual(result.stdout, reference.stdout)

    def test_large_journal_tail_and_deferred_search(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        row = b"1\t1\t0\t/tmp\t"
        oldest = b"echo oldest > found-old"
        newest = b"echo newest > found-new"
        journal.write_bytes(row + oldest + b"\n" + row + b"echo oldest\xff > wrong\n"
                            + (row + "echo padding 雪\n".encode()) * 8000
                            + row + newest + b"\n" + row + b"unfinished")
        fd = self.terminal_session(editing=True)
        os.write(fd, b"echo new")
        self.read_until(fd, b"\x1b[2mest > found-new")
        os.write(fd, b"\x1b[C\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "found-new").read_bytes(), b"newest\n")
        # Fall back across the tail boundary, retaining the last successful
        # search when a subsequent refinement in the same packet fails.
        os.write(fd, b"\x12oldestX\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "found-old").read_bytes(), b"oldest\n")
        # A replacement file has a different inode and its prefix may be UTF-8.
        replacement = journal.with_suffix(".new")
        replacement.write_bytes((row + b"padding\n") * 8000 + row + "echo 雪 > latest\n".encode())
        replacement.replace(journal)
        os.write(fd, "echo 雪".encode() + b"\x1b[C\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "latest").read_text(), "雪\n")

    def test_escaped_journal_search_and_suggestions(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        row = b"1\t1\t0\t/tmp\t"
        target = rb"\n" + "雪".encode()
        stored = b"\\" + target
        journal.write_bytes(
            row + b"echo first " + stored + b"\n"
            + row + b"echo second " + stored + b"\n"
            # Backslash followed by a newline has the same raw suffix.
            + row + b"echo wrong " + b"\\" + stored + b"\n"
            + row + b"invalid\\q " + stored + b"\n"
            + b"1\t1\t0\t/" + stored + b"\techo wrong directory\n"
            + row + rb"multi\nline-END" + b"\n"
            + row + b"literal\ttab-END\n" + row + rb"escaped\ttab-END" + b"\n"
            + row + b"literal\rcr-END\n" + row + rb"escaped\rcr-END" + b"\n"
            + (row + b"padding\n") * 9000)
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x12" + target)
        self.read_until(fd, b"': echo second " + target)
        os.write(fd, b"\x12")
        self.read_until(fd, b"': echo first " + target)
        for keys in (b"multi\x16\nline", b"literal\x16\ttab", b"escaped\x16\ttab",
                     b"literal\x16\rcr", b"escaped\x16\rcr"):
            with self.subTest(keys=keys):
                fd = self.terminal_session(editing=True)
                os.write(fd, keys)
                self.read_until(fd, b"\x1b[2m-END\x1b[0m")
        # One record exceeds a reverse-scan block. The streaming fallback
        # must still select the same older escaped command.
        with journal.open("ab") as output:
            output.write(row + b"x" * (160 * 1024) + b"\n" + (row + b"pad\n") * 2000)
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x12" + target)
        self.read_until(fd, b"': echo second " + target)

    def edited_prompt_after(self, fd, output):
        data = self.read_until(fd, output)
        if b"\x1b[?2004h" not in data.partition(output)[2]:
            self.read_until(fd, b"\x1b[?2004h")

    def test_last_arg_repeats_and_preserves_quoting(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                seed = ["cmd escaped\\ space", "cmd '東京 の🌱.txt'", "cmd $(printf nested)"]
                setup = "show(){ printf '<%s>\\n' \"$@\"; }; " + "; ".join(
                    "history -s " + shlex.quote(line) for line in seed)
                os.write(fd, (setup + "; echo rea''dy\r").encode())
                self.edited_prompt_after(fd, b"ready\r\n")
                os.write(fd, b"show zz\x02\x02\x1b.\x1b.zz\r")
                self.edited_prompt_after(fd, "<東京 の🌱.txtzzzz>\r\n".encode())
                os.write(fd, b"history -s 'cmd escaped\\ space'; echo rea''dy\r")
                self.edited_prompt_after(fd, b"ready\r\n")
                os.write(fd, b"show \x1b_\r")
                self.edited_prompt_after(fd, b"<escaped space>\r\n")
                os.write(fd, b"history -s 'cmd $(printf nested)'; echo rea''dy\r")
                self.edited_prompt_after(fd, b"ready\r\n")
                os.write(fd, b"show \x1b.\r")
                self.edited_prompt_after(fd, b"<nested>\r\n")

    def test_undo_paste_delete_and_last_arg_against_bash(self):
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"show(){ printf '<%s>\\n' \"$1\"; }; history -s 'cmd alpha'; echo rea''dy\r")
                self.edited_prompt_after(fd, b"ready\r\n")
                os.write(fd, b"show \x1b.\x1fplain\r")
                self.edited_prompt_after(fd, b"<plain>\r\n")
                os.write(fd, b"show AB\x02\x1b[200~" + "日本🌱\nline".encode() + b"\x1b[201~\x1fX\r")
                self.edited_prompt_after(fd, b"<AXB>\r\n")
                os.write(fd, b"show final\x7f\x18\x15\r")
                self.edited_prompt_after(fd, b"<final>\r\n")

    def test_undo_suggestion_and_completion(self):
        (self.cwd / "only-file").touch()
        fd = self.terminal_session(editing=True)
        os.write(fd, b"show(){ printf '<%s>\\n' \"$1\"; }; history -s 'show historical'; echo rea''dy\r")
        self.edited_prompt_after(fd, b"ready\r\n")
        for accept in (b"\x1b[C", b"\x1bf"):
            os.write(fd, b"show hist")
            self.read_until(fd, b"\x1b[2morical")
            os.write(fd, accept + b"\x1fX\r")
            self.edited_prompt_after(fd, b"<histX>\r\n")
            os.write(fd, b"history -s 'show historical'; echo rea''dy\r")
            self.edited_prompt_after(fd, b"ready\r\n")
        os.write(fd, b"show ./only-\t\x1fX\r")
        self.edited_prompt_after(fd, b"<./only-X>\r\n")

    def test_undo_key_binding_and_line_scope(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"""bind '"\\C-]":undo'; bind '"\\C-o":insert-last-argument'; history -s 'cmd bound'; echo rea''dy\r""")
        self.edited_prompt_after(fd, b"ready\r\n")
        os.write(fd, b"v=\x0f\x1dOK\r")
        os.write(fd, b'\x1fprintf "<%s>\\n" "$v"\r')
        self.edited_prompt_after(fd, b"<OK>\r\n")
        os.write(fd, b"read -e -i seed -p 'VALUE> ' v; printf '<%s>\\n' \"$v\"\r")
        self.read_until(fd, b"VALUE> seed")
        os.write(fd, b"abc\x1f\r")
        self.edited_prompt_after(fd, b"<seed>\r\n")

    def test_history_descriptor_word_boundaries(self):
        self.compare("""history -s 'cmd 2>&31-suffix <&-tail >&3-- >& 1'
history -p '!!:1' '!!:2' '!!:3' '!!:4' '!!:5' '!!:6' '!!:7' '!!:8'""",
                     0, b"2>&31-\nsuffix\n<&-\ntail\n>&3-\n-\n>&\n1\n", minimum=5)

    def test_last_arg_lexical_boundaries(self):
        cases = [("cmd foo # trailing note", "foo"), ("cmd 2>&1", "2>&1"),
                 ("cmd 2>&31-suffix", "suffix"), ("cmd <&-tail", "tail"),
                 ("cmd >&3--", "-"), ("cmd >& 1", "1"),
                 ("cmd file\\ ", "file\\ "), ("cmd foo　bar", "foo　bar"),
                 ("cat <<EOF\nhello there\nEOF", "EOF"),
                 ("cmd $(touch YANK_EXECUTED)", "$(touch YANK_EXECUTED)")]
        capture = """rec(){ printf '%s' "$READLINE_LINE" >captured; echo saved''OK; }; bind -x '"\\C-]":rec'; """
        for shell in self.shells_for(5):
            for line, want in cases + ([("cmd $((1 + 3))", "$((1 + 3))")] if shell == TONARI else []):
                with self.subTest(shell=shell, line=line):
                    fd = self.terminal_session(editing=True, shell=shell)
                    # Keep multiline fixture data in one terminal submission.
                    quoted = "$'" + line.replace("\\", "\\\\").replace("'", "\\'").replace("\n", "\\n") + "'"
                    os.write(fd, (capture + "history -s " + quoted + "; echo rea''dy\r").encode())
                    self.edited_prompt_after(fd, b"ready\r\n")
                    os.write(fd, b"\x1b.\x1d")
                    self.read_until(fd, b"savedOK\r\n")
                    self.assertEqual((self.cwd / "captured").read_text(), want)
                    self.assertFalse((self.cwd / "YANK_EXECUTED").exists())

    def test_undo_overwrite_cut_picker_and_cursor(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"show(){ printf '<%s>\\n' \"$1\"; }; echo rea''dy\r")
        self.edited_prompt_after(fd, b"ready\r\n")
        os.write(fd, "show 猫🌱".encode() + b"\x02\x1b[2~" + "海".encode() + b"\x1f\x1b[2~X\r")
        self.edited_prompt_after(fd, "<猫X🌱>\r\n".encode())
        os.write(fd, b"show firstmore\x02\x02\x02\x02\x0b\x1fZ\r")
        self.edited_prompt_after(fd, b"<firstZmore>\r\n")
        for name in ("pick-one", "pick-two"):
            (self.cwd / name).touch()
        os.write(fd, b"show ./pick-\t\t")
        self.read_until(fd, b"pick-two")
        os.write(fd, b"two\r")
        self.read_until(fd, b"show ./pick-two")
        os.write(fd, b"\x1fX\r")
        self.read_until(fd, b"<./pick-X>\r\n")

    def test_navigation_prefix_arrows_and_bash_bindings(self):
        for shell in self.shells_for():
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                setup = (b"""bind '"\\e[A":history-search-backward'; bind '"\\e[B":history-search-forward'; """
                         if shell == BASH else b"")
                os.write(fd, setup + b"history -s v=pickFIRST; history -s v=other; history -s v=pickLAST; echo rea''dy\r")
                self.read_until(fd, b"ready\r\n")
                os.write(fd, b"v=pick\x1b[A\x1b[A\x1b[B\r")
                os.write(fd, b'printf "<%s>\\n" "$v"\r')
                self.read_until(fd, b"<pickLAST>\r\n")
                os.write(fd, b"v=pick\x1b[A\x1b[A\x1b[A\r")
                os.write(fd, b'printf "<%s>\\n" "$v"\r')
                self.read_until(fd, b"<pickFIRST>\r\n")

    def test_navigation_prefix_restores_draft_and_cursor(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, "history -s v=東京古; history -s v=東京新; history -s v=other; echo rea''dy\r".encode())
        self.read_until(fd, b"ready\r\n")
        os.write(fd, "v=東京🌱".encode() + b"\x02\x1b[A\x1b[A\x1b[B\x1b[Bz\r")
        os.write(fd, b'printf "<%s>\\n" "$v"\r')
        self.read_until(fd, "<東京z🌱>\r\n".encode())
        # A changed recalled line starts a new prefix; a failed lookup keeps the draft.
        os.write(fd, "v=東京".encode() + b"\x1b[A\x05x\x1b[A\r")
        os.write(fd, b'printf "<%s>\\n" "$v"\r')
        self.read_until(fd, "<東京z🌱x>\r\n".encode())
        # Rebinding the arrow to chronological navigation takes precedence.
        os.write(fd, b"""bind '"\\e[A":previous-history'; history -s v=chronological; echo rea''dy\r""")
        self.read_until(fd, b"ready\r\n")
        os.write(fd, b"v=unrelated\x1b[A\r")
        os.write(fd, b'printf "<%s>\\n" "$v"\r')
        self.read_until(fd, b"<chronological>\r\n")

    def test_navigation_vi_and_split_escape_sequences(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"set -o vi; history -s v=VIold; history -s v=unrelated; echo rea''dy\r")
        self.read_until(fd, b"ready\r\n")
        os.write(fd, b"v=VI\x1b[")
        time.sleep(.01)
        os.write(fd, b"A\r")
        os.write(fd, b'printf "<%s>\\n" "$v"\r')
        self.read_until(fd, b"<VIold>\r\n")
        # A suffix on a custom binding must get a chance to resolve after the arrow.
        os.write(fd, b"""set -o emacs; bind '"\\e[Af":"v=longer"'; echo key''map\r""")
        self.read_until(fd, b"keymap\r\n")
        os.write(fd, b"\x1b[Af\r")
        os.write(fd, b'printf "<%s>\\n" "$v"\r')
        self.read_until(fd, b"<longer>\r\n")

    def test_navigation_shows_longer_suggestion_after_recall(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"history -s 'echo navfirst second third'; history -s 'echo navfirst'; "
                     b"history -s 'echo unrelated'; echo rea''dy\r")
        self.read_until(fd, b"ready\r\n")
        os.write(fd, b"echo nav\x1b[A")
        self.read_until(fd, b"echo navfirst\x1b[2m second third")
        os.write(fd, b"\x1bf\r")
        self.read_until(fd, b"navfirst second\r\n")

    def test_navigation_directory_history(self):
        first = "東京 $(touch BAD)"
        for name in (first, "two", "three", "four"):
            (self.cwd / name).mkdir()
        (self.cwd / "logical").symlink_to(self.cwd / "two", target_is_directory=True)
        left, right = b"\x1b[1;3D", b"\x1b[1;3C"
        fd = self.terminal_session(editing=True)
        os.write(fd, ("cd '" + first + "'; pushd ../logical >/dev/null; cd ../three; "
                      "PROMPT_COMMAND='printf \"%s\\n\" \"$PWD\" >> \"$HOME/hooks\"'; PS1='[\\W]> '\r").encode())
        self.read_until(fd, b"[three]> ")
        os.write(fd, left)
        self.read_until(fd, b"[logical]> ")
        os.write(fd, b"\x1bb")
        self.read_until(fd, ("[" + first + "]> ").encode())
        os.write(fd, right)
        self.read_until(fd, b"[logical]> ")
        os.write(fd, b'printf "%s\\n%s" "$PWD" "$OLDPWD" > "$HOME/state"\r')
        self.read_until(fd, b"\x1b[?2004h")
        pwd, old = (self.cwd / "state").read_text().splitlines()
        self.assertEqual((Path(pwd).name, Path(old).name), ("logical", first))
        # Failed cd and a subshell cd preserve the forward branch.
        os.write(fd, b"cd does-not-exist 2>/dev/null; (cd ../four)\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, right)
        self.read_until(fd, b"[three]> ")
        # Skip a deleted visit and discard the old forward branch after a new cd.
        (self.cwd / "logical").unlink()
        os.write(fd, left)
        self.read_until(fd, ("[" + first + "]> ").encode())
        os.write(fd, b"cd ../four\r")
        self.read_until(fd, b"[four]> ")
        os.write(fd, right + b'printf "<%s>\\n" "${PWD##*/}"\r')
        self.read_until(fd, b"<four>\r\n")
        self.assertFalse(list(self.cwd.rglob("BAD")))
        hooks = [Path(s).name for s in (self.cwd / "hooks").read_text().splitlines()]
        self.assertEqual(hooks[:8], ["three", "logical", first, "logical", "logical", "logical", "three", first])

    def test_navigation_directory_keys_inside_input(self):
        (self.cwd / "child").mkdir()
        fd = self.terminal_session(editing=True)
        os.write(fd, b"cd child; PS2='MORE> '; PS1='[\\W]> '\r")
        self.read_until(fd, b"[child]> ")
        left, right = b"\x1b[1;3D", b"\x1b[1;3C"
        os.write(fd, b"printf aa >word" + left + b"my" + right + b"\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "child/myword").read_bytes(), b"aa")
        os.write(fd, b"read -ep 'VALUE> ' val; printf '<%s %s>\\n' \"$val\" \"${PWD##*/}\"\r")
        self.read_until(fd, b"VALUE> ")
        os.write(fd, left + b"sample\r")
        self.read_until(fd, b"<sample child>\r\n")
        os.write(fd, b"if true; then\r")
        self.read_until(fd, b"MORE> ")
        os.write(fd, left + b'printf "<%s>\\n" "${PWD##*/}"; fi\r')
        self.read_until(fd, b"<child>\r\n")

    def test_history_word_accept_and_bash_word_motion(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        lines = ["take '東京 の🌱' second", r"take path\ with\ spaces later",
                 "take $(printf 'x y') suffix", "take alpha|cat", "take 'two words' unfinished\""]
        journal.write_text("".join("1\t1\t0\t/tmp\t" + line.replace("\\", "\\\\") + "\n"
                                   for line in lines))
        fd = self.terminal_session(editing=True)
        os.write(fd, b'take(){ printf "%s" "$1" > result; };\r')
        self.read_until(fd, b"\x1b[?2004h")
        for prefix, rest, selected, value in [
            ("take '東", "京 の🌱'", "take '東京 の🌱'", "東京 の🌱"),
            ("take pa", r"th\ with\ spaces", r"take path\ with\ spaces", "path with spaces"),
            ("take $", "(printf 'x y')", "take $(printf 'x y')", "x"),
            ("take al", "pha|cat", "take alpha", "alpha"),
            ("take 'tw", "o words'", "take 'two words'", "two words"),
        ]:
            os.write(fd, prefix.encode())
            self.read_until(fd, b"\x1b[2m" + rest.encode())
            os.write(fd, b"\x1bf")
            self.read_until(fd, PROMPT + selected.encode() + b"\x1b[2m")
            os.write(fd, b"\r")
            self.read_until(fd, b"\x1b[?2004h")
            self.assertEqual((self.cwd / "result").read_text(), value)
        for shell in self.shells_for():
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"printf AA >motion; echo wit''nessOK\x01\x1bf\x1bfB\r")
                self.read_until(fd, b"witnessOK\r\n")
                self.assertEqual((self.cwd / "motion").read_bytes(), b"AAB")
                (self.cwd / "motion").unlink()

    def test_runtime_review_history_keeps_longer_command_with_tab(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        journal.write_bytes(b"1\t1\t0\t/old\tprintf reached >flag;\t: pick token\n"
                            b"2\t123\t2\t0\t/new\t: pick token\n" * 2)
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x12pick token")
        self.read_until(fd, b"> : pick token")
        os.write(fd, b"\x1b[B")
        self.read_until(fd, b"> printf reached")
        os.write(fd, b"\r\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "flag").read_bytes(), b"reached")

    def test_history_fragment_picker_select_cancel_and_paste(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        def row(c):
            return "1\t4\t0\t/tmp\t" + c.replace("\\", "\\\\") + "\n"
        lines = [f"printf %s '東京🌱 {n:02d}' > port15432" for n in range(13)]
        journal.write_text("".join(map(row, lines + [lines[-1], "printf unrelated-15432"]))
                           + "1\t4\t0\t/東京🌱/15432\tprintf metadata\n"
                           + "1\t4\t0\t/tmp\tunfinished 東京🌱 15432")
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x12\x1b[200~15432 " + "東京🌱".encode() + b"\x1b[201~")
        self.read_until(fd, ("> " + lines[-1]).encode())
        os.write(fd, b"\x1b[B" * 8 + b"\r")
        self.read_until(fd, PROMPT + lines[4].encode())
        self.assertFalse((self.cwd / "port15432").exists())
        os.write(fd, b"\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "port15432").read_text(), "東京🌱 04")
        os.write(fd, b"v=AC\x02\x12not found\r")
        self.read_until(fd, b"(no matches)")
        os.write(fd, b'\x07B\x05; printf %s "$v" > restored\r')
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "restored").read_text(), "ABC")
        os.write(fd, b"\x12port15432\t")
        self.read_until(fd, b"history> port15432")
        os.write(fd, b"\x1b")
        self.read_until(fd, PROMPT)

    def test_history_fragment_crosses_blocks_and_ignores_unfinished_rows(self):
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        row = b"1\t2\t0\t/tmp\t"
        target = "printf '%s' '東京🌱 42' > selected".encode()
        journal.write_bytes(row + target + b"\n" + row + b"x" * 700000 + b"\n"
                            + row + b"invalid\\q " + target + b"\n"
                            + row + b"invalid-utf8-\xff " + target + b"\n"
                            + row + b"unfinished " + target)
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x12" + "42　東京🌱".encode())
        self.read_until(fd, b"> " + target)
        os.write(fd, b"\r\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "selected").read_text(), "東京🌱 42")

    def test_history_fragment_local_without_journal(self):
        self.env["HISTFILE"] = "/dev/null"
        fd = self.terminal_session(editing=True)
        line = b"printf abc >session-only"
        os.write(fd, line + b"\r")
        self.read_until(fd, b"\x1b[?2004h")
        os.write(fd, b"\x12session-only abc")
        self.read_until(fd, b"> " + line)
        os.write(fd, b"\x07")
        self.read_until(fd, PROMPT)
        self.assertFalse((self.cwd / ".local/state/tonari/history").exists())

    def test_unsuccessful_reverse_search_keeps_the_original_draft(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"v=previous\r")
                self.read_until(fd, b"\x1b[?2004h")
                os.write(fd, b"v=draft\x12" + "☃absent".encode() + b"\r")
                self.read_until(fd, b"\x1b[?2004h")
                os.write(fd, b'echo "$v" >miss-result\r')
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "miss-result").read_text(), "draft\n")

    def test_reverse_search_accepts_same_escaped_command_as_bash(self):
        commands = [rb"printf '%s\n' selected=first-foo\ bar",
                    rb"printf '%s\n' selected=second-foo\ bar"]
        commands += [b": padding"] * 8000
        (self.cwd / ".bash_history").write_bytes(b"\n".join(commands) + b"\n")
        self.env["HISTSIZE"] = str(len(commands))
        journal = self.cwd / ".local/state/tonari/history"
        journal.parent.mkdir(parents=True)
        journal.write_bytes(b"".join(b"1\t1\t0\t/tmp\t" + cmd.replace(b"\\", b"\\\\") + b"\n"
                                     for cmd in commands))
        for shell in self.shells_for():
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                os.write(fd, b"\x12foo\\ bar\r")
                self.read_until(fd, b"selected=second-foo bar\r\n")

    def test_utf8_burst_at_cursor_and_split_character(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"v='!'" + b"\x02\x02")
        self.read_until(fd, b"\x1b[11C")
        word = "東京🌱e\u0301" * 20
        # Leave the next scalar deliberately incomplete. The valid prefix must
        # be visible before we write the rest.
        os.write(fd, word.encode() + b"END\xf0\x9f")
        emitted = self.read_until(fd, b"END!'")
        self.assertLess(emitted.count(b"\x1b[J"), 10)
        os.write(fd, b"\x98\xba\x05; printf %s \"$v\" > result\r")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "result").read_text(), word + "END😺!")

    def test_bulk_history_completion_and_foreground_input(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                def enter(source):
                    os.write(fd, source.encode() + b"\r")
                    self.read_until(fd, b"\x1b[?2004h")
                enter("f() { printf %s \"$1$2\" > result; }; pick() { echo x >> calls; COMPREPLY=(\"$2完了\"); }; complete -F pick f")
                os.write(fd, "f 日本語".encode() + b"\t" + "🌱".encode() + b"\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "result").read_text(), "日本語完了🌱")
                self.assertEqual((self.cwd / "calls").read_text(), "x\n")
                (self.cwd / "calls").unlink()
                enter("printf %s 候補 > selected")
                enter("printf %s 候 > selected")
                os.write(fd, b"\x12" + "候補無🌱".encode() + b"\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "selected").read_text(), "候補")
                (self.cwd / "external").unlink(missing_ok=True)
                enter("printf %s 別 > selected")
                os.write(fd, b"\x12\x1b[200~" + "候補".encode() + b"\x1b[201~\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "selected").read_text(), "候補")
                packet = b"\x1b[200~" + "v=日本; /bin/sh -c 'IFS= read -r x; printf %s \"$x\" > external'".encode()
                os.write(fd, packet + b"\x1b[201~\r" + "子の入力🌱\n".encode())
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "external").read_text(), "子の入力🌱")

    def test_utf8_burst_respects_custom_keys_and_paste_bypasses_them(self):
        self.env["LC_ALL"] = "en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8"
        (self.cwd / "inputrc").write_text('"@@": "置換"\n')
        self.env["INPUTRC"] = str(self.cwd / "inputrc")
        for shell in self.shells_for(5):
            with self.subTest(shell=shell):
                fd = self.terminal_session(editing=True, shell=shell)
                path = self.cwd / "result"
                path.unlink(missing_ok=True)
                packet = "v='前@@後".encode() + b"\x1b[200~" + "@@\t雪".encode() + b"\x1b[201~"
                os.write(fd, packet + b"'\x05; printf %s \"$v\" > result\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual(path.read_text(), "前置換後@@\t雪")

    def test_accepts_full_utf8_history_suggestion_and_recalls_line(self):
        fd = self.terminal_session(editing=True)
        command = "printf '%s' '一括" + "日本🌱" * 21 + "終' > recalled"
        os.write(fd, command.encode() + b"\r")
        self.read_until(fd, b"\x1b[?2004h")
        expected = ("一括" + "日本🌱" * 21 + "終").encode()
        for keys in (b"\x1b[A", "printf '%s' '一括".encode() + b"\x1b[C"):
            with self.subTest(keys=keys):
                (self.cwd / "recalled").unlink()
                os.write(fd, keys + b"\r")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "recalled").read_bytes(), expected)

    def test_search_paste_is_literal(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x12\x1b[200~" + "雪\t🌱".encode() + b"\r\ntouch should-not-exist\r\x1b[A\x1b[201~")
        display = self.read_until(fd, b"should-not-exist^J^[[A': ")
        self.assertIn("雪^I🌱^Jtouch".encode(), display)
        os.write(fd, b"\x07")
        self.read_until(fd, PROMPT)
        self.assertFalse((self.cwd / "should-not-exist").exists())

    def test_paste_boundary_and_input_for_foreground_read(self):
        fd = self.terminal_session(editing=True)
        os.write(fd, b"\x1b[200~v=" + b"x" * 512 + b"\x1b[20")
        os.write(fd, b"1~; IFS= read -r reply; echo \"$reply:${#v}\" > pasted-result\rpayload\n")
        self.read_until(fd, b"\x1b[?2004h")
        self.assertEqual((self.cwd / "pasted-result").read_text(), "payload:512\n")

        # Exercise every end-marker offset within the seven-byte read. The first
        # key after it accepts the command. The external reader owns the next byte.
        for padding in range(7):
            with self.subTest(padding=padding):
                child = b"/bin/sh -c 'IFS= read -r v; printf %s \"$v\" > child'"
                value = f"payload-{padding}".encode()
                os.write(fd, b"\x1b[200~" + b" " * padding + child + b"\x1b[201~\r" + value + b"\n")
                self.read_until(fd, b"\x1b[?2004h")
                self.assertEqual((self.cwd / "child").read_bytes(), value)

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
