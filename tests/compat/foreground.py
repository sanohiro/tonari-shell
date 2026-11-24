"""Repeat the audited controlling-terminal pipeline witness and preserve failed assertions."""
import json
from pathlib import Path
import sys
import time
import traceback

from bench import stats, terminal_checks


def main():
    tonari, bash = (Path(arg).resolve() for arg in sys.argv[1:])
    shell_tests = terminal_checks(tonari)
    samples = [[], []]
    errors = []
    for iteration in range(43):
        for index in (iteration % 2, 1 - iteration % 2):
            shell = (bash, tonari)[index]
            test = shell_tests()
            test.setUp()
            try:
                start = time.perf_counter_ns()
                test.check_foreground_pipeline(test.terminal_session(shell=shell))
                elapsed = time.perf_counter_ns() - start
                if iteration >= 2:
                    samples[index].append(elapsed)
            except (AssertionError, OSError):
                errors.append(dict(iteration=iteration, shell=str(shell), detail=traceback.format_exc()))
            finally:
                test.doCleanups()
    result = dict(samples=samples, errors=errors, warmups=2)
    if not errors:
        result["timing"] = stats(samples)
    print(json.dumps(result))
    return bool(errors)


if __name__ == "__main__":
    sys.exit(main())
