#!/usr/bin/env python3
"""Exercise the public replay command with real, known SMT queries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rz', type=Path, required=True)
    parser.add_argument('--solver', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    rz = args.rz.resolve(strict=True)
    solver = args.solver.resolve(strict=True)
    root = Path(__file__).resolve().parent
    version = subprocess.run([str(solver), '--version'], check=True, capture_output=True, text=True, timeout=10).stdout.strip()
    results = []
    with tempfile.TemporaryDirectory(prefix='res-replay-evidence-') as tmp:
        bundle = Path(tmp)
        for name, accepted in [('unsat', True), ('sat', False), ('multiple', False), ('invalid', False)]:
            query = root / 'queries' / (name + '.smt2')
            (bundle / 'proof.smt2').write_bytes(query.read_bytes())
            manifest = {'program': 'qualification-query', 'obligations': [{'fn': 'query', 'kind': 'ensures', 'idx': 0,
                        'cert': 'proof.smt2', 'sha256': digest(query)}]}
            (bundle / 'manifest.json').write_text(json.dumps(manifest), encoding='utf-8')
            completed = subprocess.run([str(rz), 'verify-all', str(bundle), '--z3'], env={**os.environ, 'Z3_BINARY': str(solver)},
                                       capture_output=True, timeout=30)
            actual = completed.returncode == 0
            passed = actual == accepted and (b'all checks passed' in completed.stdout) == accepted
            results.append({'id': 'REAL-' + name.upper(), 'query': 'queries/' + query.name, 'query_sha256': digest(query),
                            'expected_accept': accepted, 'returncode': completed.returncode, 'passed': passed,
                            'stdout': completed.stdout.decode('utf-8', errors='backslashreplace'),
                            'stderr': completed.stderr.decode('utf-8', errors='backslashreplace')})
            print(f'test real_solver::{name} ... {"ok" if passed else "FAILED"}')
    report = {'schema_version': 1, 'scope': 'Saved-query replay; no source-to-query or qualification claim',
              'compiler_sha256': digest(rz), 'solver_sha256': digest(solver), 'solver_version': version,
              'cases': results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return 0 if all(case['passed'] for case in results) else 1


if __name__ == '__main__':
    raise SystemExit(main())
