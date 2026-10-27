#!/usr/bin/env python3
"""Compare authenticated saved queries with trusted, source-authored checkpoints."""
import argparse
import hashlib
import stat
import types
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time

PROFILE = 'legacy-lia-identity-v1'
BV_PROFILE = 'vm-i64-wrap-v1'
DEPENDENCY_SHA256 = 'de839e6f5a2f108421aef147cf64830cfd179d65883b1ff79e1efe59588aab62'


def bootstrap():
    """Authenticate exact bounded regular-file bytes before executing any helper."""
    path = Path(__file__).with_name('audit-proof-coverage.py')
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 2 * 1024 * 1024:
            raise ValueError('dependency must be a bounded regular file')
        raw = b''
        while True:
            chunk = os.read(descriptor, 65536)
            if not chunk:
                break
            raw += chunk
            if len(raw) > 2 * 1024 * 1024:
                raise ValueError('dependency exceeds bounded size')
    finally:
        os.close(descriptor)
    if hashlib.sha256(raw).hexdigest() != DEPENDENCY_SHA256:
        raise ValueError('reviewed dependency digest differs')
    module = types.ModuleType('coverage_primitives')
    module.__file__ = str(path)
    exec(compile(raw, str(path), 'exec'), module.__dict__)
    return module


try:
    core = bootstrap()
except (OSError, ValueError) as error:
    print(json.dumps({'stage': 'binding', 'code': 'dependency_hash_mismatch',
                      'detail': str(error), 'authenticated': False}, sort_keys=True))
    sys.exit(1)


def fail(code, detail):
    core.reject('structure', code, detail)


def expanded(commands):
    """Expand lexical parallel lets; never simplify/reorder Boolean structure."""
    remaining = 100000
    def spend(depth):
        nonlocal remaining
        remaining -= 1
        if remaining < 0 or depth > 64:
            fail('expansion_resource_limit', 'bounded lexical expansion exceeded')
    def copy(node, depth):
        spend(depth)
        return [copy(child, depth + 1) for child in node] if isinstance(node, list) else node
    def walk(node, environment, depth):
        spend(depth)
        if isinstance(node, str):
            if node.startswith(('|', '"')):
                fail('unsupported_identifier', 'quoted identifiers/strings are outside this reviewed profile')
            # Substitution values already belong to their defining lexical scope.
            # Re-expanding them under this environment would capture free names.
            return copy(environment[node], depth + 1) if node in environment else node
        if not isinstance(node, list):
            fail('invalid_session', 'SMT nodes must be lists or atoms')
        if node and node[0] == 'let':
            if len(node) != 3 or not isinstance(node[1], list) or len(node[1]) > 128:
                fail('invalid_let', 'invalid bounded let form')
            values = {}
            for binding in node[1]:
                if (not isinstance(binding, list) or len(binding) != 2
                        or not isinstance(binding[0], str) or not binding[0]):
                    fail('invalid_let', 'invalid let binding')
                name = binding[0]
                if name.startswith(('|', '"')):
                    fail('unsupported_identifier', 'quoted binding aliases are unsupported')
                if name in values:
                    fail('invalid_let', 'duplicate simultaneous binding')
                # SMT-LIB binding values are evaluated in the OUTER environment.
                values[name] = walk(binding[1], environment, depth + 1)
            nested = dict(environment)
            nested.update(values)
            return walk(node[2], nested, depth + 1)
        if node and node[0] in ('forall', 'exists', 'lambda'):
            fail('unsupported_binding', 'only reviewed lexical let bindings are supported')
        return [walk(child, environment, depth + 1) for child in node]
    return walk(commands, {}, 0)


def reference(entry):
    if (not isinstance(entry, dict) or not isinstance(entry.get('fn'), str)
            or not isinstance(entry.get('kind'), str) or type(entry.get('idx')) is not int
            or entry['idx'] < 0):
        fail('invalid_obligation', 'invalid explicit obligation identity')
    return entry['fn'], entry['kind'], entry['idx']


def bv_commands(path):
    """Reviewed identity template; no algebraic or Boolean normalization."""
    remaining = 256
    def domain(node, depth=0):
        nonlocal remaining
        remaining -= 1
        if remaining < 0 or depth > 32:
            return False
        if isinstance(node, str):
            return node in ('true', 'false')
        if not isinstance(node, list) or not node:
            return False
        operator = node[0]
        if operator in ('and', 'or') and len(node) >= 2:
            return all(domain(child, depth + 1) for child in node[1:])
        if operator == 'not' and len(node) == 2:
            return domain(node[1], depth + 1)
        if operator in ('=', 'bvsge') and len(node) == 3:
            literal = node[2]
            return (node[1] == 'x' and isinstance(literal, str)
                    and len(literal) == 18 and literal.startswith('#x')
                    and all(char in '0123456789abcdef' for char in literal[2:]))
        return False

    guard = path.get('domain')
    if not domain(guard):
        core.reject('binding', 'invalid_descriptor', 'unreviewed BV identity domain')
    goal = ['=', 'x', 'x']
    claim = ['and', 'true',
             ['=>', 'true', ['and', 'true', ['=>', guard, 'true']]],
             ['=>', 'true', ['=>', guard, goal]]]
    return [['set-logic', 'QF_BV'],
            ['declare-fun', 'x', [], ['_', 'BitVec', '64']],
            ['assert', ['not', claim]], ['check-sat']]


def descriptor(args):
    raw = core.read(args.descriptor)
    if core.digest(raw) != args.descriptor_sha256:
        core.reject('binding', 'descriptor_hash_mismatch', 'trusted descriptor digest differs')
    value = core.decode(raw)
    if (not isinstance(value, dict) or value.get('schema') != 'resilient-structural-source-checkpoints'
            or type(value.get('version')) is not int or value['version'] != 1):
        core.reject('binding', 'invalid_descriptor', 'unsupported descriptor schema/version')
    if args.profile not in (PROFILE, BV_PROFILE) or value.get('profile') != args.profile:
        core.reject('binding', 'unsupported_profile', 'explicit matching reviewed identity profile required')
    source = value.get('source')
    if (not isinstance(source, dict) or not isinstance(source.get('path'), str)
            or Path(source['path']).name != source['path'] or not source['path'].endswith('.rz')
            or any(char in source['path'] for char in '/\\:')
            or type(source.get('bytes')) is not int or not 0 <= source['bytes'] <= core.MAX_DOCUMENT
            or not isinstance(source.get('sha256'), str) or len(source['sha256']) != 64):
        core.reject('binding', 'invalid_descriptor', 'expected one confined source file')
    declaration = value.get('qualified_declaration')
    clause = value.get('clause')
    if declaration != 'program::identity' or type(clause) is not int or clause != 0:
        core.reject('binding', 'invalid_descriptor', 'only the manually reviewed program::identity clause0 is supported')
    paths = value.get('paths')
    if not isinstance(paths, list) or not 1 <= len(paths) <= 128:
        core.reject('binding', 'invalid_descriptor', 'expected1..128 explicit paths')
    identities = set()
    for index, path in enumerate(paths):
        if not isinstance(path, dict) or not isinstance(path.get('id'), str) or not path['id']:
            core.reject('binding', 'invalid_descriptor', 'missing path identity')
        if path['id'] in identities:
            core.reject('binding', 'invalid_descriptor', 'duplicate path identity')
        identities.add(path['id'])
        if path.get('obligation') != {'fn': 'identity:ensures:0', 'kind': 'ensures_implementation', 'idx': index}:
            core.reject('binding', 'invalid_descriptor', 'expected contiguous source-order identity:ensures:0 implementation paths')
        commands = path.get('commands')
        if not isinstance(commands, list):
            core.reject('binding', 'invalid_descriptor', 'missing exact command checkpoint')
        if args.profile == BV_PROFILE and commands != bv_commands(path):
            core.reject('binding', 'invalid_descriptor', 'BV domain/return command checkpoint fields disagree')
        encoded = ('\n'.join(core.render(command) for command in commands) + '\n').encode()
        if len(encoded) > core.MAX_QUERY or core.sexpressions(encoded) != commands:
            core.reject('binding', 'invalid_descriptor', 'invalid command checkpoint')
        prefix = [['set-logic', 'AUFLIA'], ['declare-const', 'x', 'Int']]
        goal = path.get('postcondition_after_substitution')
        if path.get('returned_value') != 'x' or goal != ['=', 'x', 'x']:
            core.reject('binding', 'invalid_descriptor', 'only the reviewed identity-return goal is supported')
        if args.profile == BV_PROFILE:
            continue
        if path['id'] != 'entry' or path.get('domain') != 'true':
            prefix.append(['assert', path.get('domain')])
        expected = prefix + [['assert', ['not', goal]], ['check-sat']]
        if commands != expected:
            core.reject('binding', 'invalid_descriptor', 'path/domain/return checkpoint fields disagree')
    return value


def audit(args, state):
    start = time.monotonic()
    def budget():
        remaining = args.timeout_seconds - (time.monotonic() - start)
        if remaining <= 0:
            core.reject('binding', 'audit_timeout', 'whole audit deadline exceeded')
        return remaining
    dependency = Path(__file__).with_name('audit-proof-coverage.py')
    dependency_hash = core.digest(core.read(dependency))
    if dependency_hash != DEPENDENCY_SHA256:
        core.reject('binding', 'dependency_hash_mismatch', 'reviewed authentication/parser dependency changed')
    expected = descriptor(args)
    source_root = Path(args.source_root).resolve(strict=True)
    certs = Path(args.cert_dir).resolve(strict=True)
    source = expected['source']
    source_raw = core.read(source_root / source['path'])
    if len(source_raw) != source['bytes'] or core.digest(source_raw) != source['sha256']:
        core.reject('binding', 'source_hash_mismatch', 'actual source differs from trusted source checkpoint')
    manifest_raw = core.read(certs / 'manifest.json')
    manifest = core.decode(manifest_raw)
    graph = {'builtins': [], 'entry': source['path'], 'files': [source], 'imports': [], 'version': 1}
    if manifest.get('source_binding') != graph:
        core.reject('binding', 'source_graph_mismatch', 'profile requires exactly the reviewed single-file input graph')
    entries = manifest.get('obligations')
    if not isinstance(entries, list) or not 1 <= len(entries) <= 128:
        core.reject('binding', 'resource_limit', 'expected1..128 retained obligations')
    total = 0
    for entry in entries:
        name = entry.get('cert')
        if (not isinstance(name, str) or Path(name).name != name or not name.endswith('.smt2')
                or any(char in name for char in '/\\:')):
            core.reject('binding', 'invalid_query_path', 'one confined query filename required')
        total += len(core.read(certs / name, core.MAX_QUERY))
        if total > 16 * core.MAX_QUERY:
            core.reject('binding', 'resource_limit', 'total retained queries exceed16MiB')
    compiler = Path(args.rz).resolve(strict=True)
    compiler_hash = core.digest(core.read(compiler, 256 * 1024 * 1024))
    if compiler_hash != args.compiler_sha256:
        core.reject('binding', 'compiler_hash_mismatch', 'actual trusted compiler differs')
    configured = shutil.which(args.solver)
    if not configured:
        core.reject('binding', 'solver_unavailable', 'configured solver unavailable')
    solver = str(Path(configured).resolve(strict=True))
    solver_hash = core.digest(core.read(solver, 256 * 1024 * 1024))
    if solver_hash != args.solver_sha256:
        core.reject('binding', 'solver_hash_mismatch', 'actual trusted solver differs')
    code, version, _ = core.bounded([str(compiler), '--version', '--verbose'], budget(), 'binding')
    if code or not args.build_profile or '  profile: ' + args.build_profile not in version.decode().splitlines():
        core.reject('binding', 'build_profile_mismatch', 'actual compiler build profile differs')
    key = Path(args.pubkey).resolve(strict=True)
    core.read(key)
    environment = dict(os.environ, Z3_BINARY=solver)
    code, output, errors = core.bounded([str(compiler), 'verify-all', str(certs), '--source-root', str(source_root),
                                       '--pubkey', str(key), '--z3'], budget(), 'authentication', environment=environment)
    if code:
        core.reject('authentication', 'authentication_failed', (output + errors).decode(errors='replace')[-4096:])
    if (core.read(certs / 'manifest.json') != manifest_raw or core.read(source_root / source['path']) != source_raw
            or core.digest(core.read(compiler, 256 * 1024 * 1024)) != compiler_hash
            or core.digest(core.read(solver, 256 * 1024 * 1024)) != solver_hash):
        core.reject('authentication', 'input_changed', 'input/tool changed during authenticated replay')
    state['authenticated'] = True
    actual = {}
    for entry in entries:
        identity = reference(entry)
        if identity in actual:
            fail('duplicate_obligation', identity)
        actual[identity] = entry
    requested, matched = set(), []
    for path in expected['paths']:
        identity = reference(path.get('obligation'))
        if identity[1] != 'ensures_implementation' or identity in requested:
            fail('invalid_descriptor', 'required implementation identities must be unique')
        requested.add(identity)
        if identity not in actual:
            fail('missing_obligation', identity)
        entry = actual[identity]
        raw = core.read(certs / entry['cert'], core.MAX_QUERY)
        if core.digest(raw) != entry['sha256']:
            fail('input_changed', 'query changed after authenticated replay')
        actual_commands = expanded(core.sexpressions(raw))
        if actual_commands != path['commands']:
            fail('checkpoint_mismatch', {'path': path['id'], 'obligation': identity})
        matched.append({'path': path['id'], 'obligation': identity, 'query_sha256': core.digest(raw),
                        'expanded_commands_sha256': core.digest(core.canonical(actual_commands))})
    extra = [identity for identity in actual if identity not in requested]
    if extra:
        fail('extra_obligation', extra)
    if core.digest(core.read(dependency)) != dependency_hash:
        fail('input_changed', 'reviewed dependency changed during audit')
    checker_hash = core.digest(core.read(Path(__file__).resolve()))
    budget()
    return {'stage': 'structure', 'code': 'matched', 'authenticated': True, 'profile': args.profile,
            'claim': 'only supplied source-authored structural checkpoints; not automatic source enumeration, runtime reachability or full translation proof',
            'descriptor_sha256': args.descriptor_sha256, 'source_sha256': source['sha256'],
            'qualified_declaration': expected['qualified_declaration'], 'clause': expected['clause'],
            'compiler_sha256': compiler_hash, 'solver_sha256': solver_hash, 'paths': matched,
            'checker_sha256': checker_hash, 'dependency_sha256': dependency_hash,
            'tool_graph_sha256': core.digest(core.canonical({'checker': checker_hash, 'dependency': dependency_hash}))}


def main():
    class Parser(argparse.ArgumentParser):
        def error(self, message):
            core.reject('binding', 'invalid_arguments', message)
    parser = Parser(description=__doc__)
    for name in ('rz', 'compiler-sha256', 'solver', 'solver-sha256', 'cert-dir', 'source-root', 'pubkey',
                 'descriptor', 'descriptor-sha256', 'profile', 'build-profile'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--timeout-seconds', type=float, default=20)
    state = {'authenticated': False}
    previous_handler = previous_timer = started = None
    try:
        args = parser.parse_args()
        if (os.name != 'posix' or threading.current_thread() is not threading.main_thread()
                or not 0.1 <= args.timeout_seconds <= 60):
            core.reject('binding', 'unsupported_environment', 'POSIX main-thread timeout0.1..60seconds required')
        previous_handler = signal.getsignal(signal.SIGALRM)
        previous_timer = signal.getitimer(signal.ITIMER_REAL)
        started = time.monotonic()
        def expired(_signum, _frame):
            core.reject('binding', 'audit_timeout', 'outer audit deadline exceeded')
        signal.signal(signal.SIGALRM, expired)
        signal.setitimer(signal.ITIMER_REAL, args.timeout_seconds)
        report = audit(args, state)
    except core.Rejected as error:
        report = {'stage': error.stage, 'code': error.code, 'detail': error.detail, **state}
    except (OSError, ValueError, KeyError, IndexError, TypeError, AttributeError, RecursionError, subprocess.SubprocessError) as error:
        report = {'stage': 'binding', 'code': 'invalid_input', 'detail': str(error), **state}
    finally:
        if previous_handler is not None:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous_handler)
            if previous_timer[0] > 0:
                signal.setitimer(signal.ITIMER_REAL, max(0.000001, previous_timer[0] - (time.monotonic() - started)), previous_timer[1])
    print(json.dumps(report, sort_keys=True))
    return 0 if report['code'] == 'matched' else 1


if __name__ == '__main__':
    sys.exit(main())
