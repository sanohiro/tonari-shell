"""Read cases and grade actual bytes with the pinned upstream Python 2 runner."""
from __future__ import print_function
import base64
import glob
import json
import os
import sys
from test import sh_spec


def main():
    grade = len(sys.argv) > 1 and sys.argv[1] == "grade"
    records = {}
    if grade:
        for line in sys.stdin:
            row = json.loads(line)
            records[row['key']] = row
    for path in sorted(glob.glob('spec/*.test.sh')):
        with open(path) as f:
            meta, cases = sh_spec.ParseTestFile(path, sh_spec.Tokenizer(f))
        suite = os.path.basename(path).split('.')[0]
        labels = meta.get('compare_shells', '').split()
        include = (any(s.split('-')[0] == 'bash' for s in labels) or suite == 'redir-order')
        include = include and meta.get('suite') != 'disabled'
        for index, case in enumerate(cases):
            key = '%s/%d' % (suite, index)
            if not grade:
                print(json.dumps(dict(key=key, suite=suite, index=index, name=case['desc'],
                                      line=case['line_num'], code=base64.b64encode(case['code']),
                                      include=include, compare_shells=labels,
                                      legacy=bool(meta.get('legacy_tmp_dir')))))
            elif key in records:
                row = records[key]
                grades = {}
                for label, actual in row['actual'].items():
                    if actual['limit']:
                        grades[label] = 'TIME' if actual['limit'] == 'timeout' else 'OUTPUT_LIMIT'
                        continue
                    data = dict(status=actual['status'], stdout=base64.b64decode(actual['stdout']),
                                stderr=base64.b64decode(actual['stderr']))
                    results = [a.Check('bash', data)[0] for a in sh_spec.CreateAssertions(case, 'bash')]
                    result = min(results)
                    grades[label] = next(k for k,v in vars(sh_spec.Result).items()
                                         if k.isupper() and v == result)
                print(json.dumps(dict(key=key, grades=grades)))


if __name__ == '__main__':
    main()
