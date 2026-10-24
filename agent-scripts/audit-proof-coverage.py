#!/usr/bin/env python3
"""Audit explicitly requested implementation sites after authenticated source replay."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import selectors
import shutil
import signal
import stat
import threading
import subprocess
import sys
import time

MAX_DOCUMENT = 2 * 1024 * 1024
MAX_QUERY = 1024 * 1024
MAX_OUTPUT = 512 * 1024
MAX_SITES = 512


class Rejected(Exception):
    def __init__(self, stage, code, detail):
        self.stage, self.code = stage, code
        self.detail = str(detail) if isinstance(detail, Path) else detail


def reject(stage, code, detail):
    raise Rejected(stage, code, detail)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path, limit=MAX_DOCUMENT):
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            reject('binding', 'non_regular_input', str(path))
        if info.st_size > limit:
            reject('binding', 'resource_limit', str(path))
        chunks, count = [], 0
        while count <= limit:
            chunk = os.read(descriptor, min(65536, limit + 1 - count))
            if not chunk:
                break
            chunks.append(chunk)
            count += len(chunk)
        if count > limit:
            reject('binding', 'resource_limit', str(path))
        return b''.join(chunks)
    finally:
        os.close(descriptor)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            reject('binding', 'duplicate_json_key', key)
        result[key] = value
    return result


def decode(raw):
    return json.loads(raw, object_pairs_hook=unique_object)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def bounded(argv, timeout, stage, input_bytes=None, environment=None):
    """Bound both output streams and kill the inherited solver process group."""
    process = subprocess.Popen(argv, stdin=subprocess.PIPE if input_bytes is not None else subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True, env=environment)
    captured = {process.stdout: bytearray(), process.stderr: bytearray()}
    selector = selectors.DefaultSelector()
    for stream in captured:
        selector.register(stream, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    pending = memoryview(input_bytes or b'')
    if input_bytes is not None:
        os.set_blocking(process.stdin.fileno(), False)
        selector.register(process.stdin, selectors.EVENT_WRITE)
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                reject(stage, stage + '_timeout', 'subprocess deadline exceeded')
            for key, _ in selector.select(min(remaining, 0.1)):
                if key.fileobj is process.stdin:
                    try:
                        written = os.write(process.stdin.fileno(), pending[:16384]) if pending else 0
                        pending = pending[written:]
                    except BlockingIOError:
                        continue
                    except BrokenPipeError:
                        pending = pending[:0]
                    if not pending:
                        selector.unregister(process.stdin)
                        process.stdin.close()
                    continue
                chunk = os.read(key.fileobj.fileno(), 16384)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                captured[key.fileobj].extend(chunk)
                if sum(len(value) for value in captured.values()) > MAX_OUTPUT:
                    reject(stage, 'subprocess_output_limit', 'bounded subprocess output exceeded')
        code = process.wait(timeout=max(0.01, deadline - time.monotonic()))
        return code, bytes(captured[process.stdout]), bytes(captured[process.stderr])
    finally:
        selector.close()
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=2)
        for stream in (process.stdin, process.stdout, process.stderr):
            if stream is not None:
                stream.close()


def sexpressions(raw):
    """Preserve command order/multiplicity; normalize only whitespace/comments."""
    text = raw.decode('utf-8')
    roots, stack, tokens = [], [], 0
    index = 0
    while index < len(text):
        char = text[index]
        if char.isspace():
            index += 1
            continue
        if char == ';':
            end = text.find('\n', index)
            index = len(text) if end < 0 else end + 1
            continue
        tokens += 1
        if tokens > 100000:
            reject('coverage', 'query_resource_limit', 'too many SMT tokens')
        if char == '(':
            node = []
            (stack[-1] if stack else roots).append(node)
            stack.append(node)
            if len(stack) > 64:
                reject('coverage', 'query_resource_limit', 'SMT nesting exceeds64')
            index += 1
        elif char == ')':
            if not stack:
                reject('coverage', 'invalid_query', 'unmatched SMT closing delimiter')
            stack.pop()
            index += 1
        else:
            start = index
            if char in ('"', '|'):
                delimiter = char
                index += 1
                while index < len(text):
                    if text[index] == delimiter:
                        index += 1
                        if delimiter == '"' and index < len(text) and text[index] == '"':
                            index += 1
                            continue
                        break
                    index += 1
                else:
                    reject('coverage', 'invalid_query', 'unterminated SMT quoted atom')
            else:
                while index < len(text) and not text[index].isspace() and text[index] not in '();':
                    index += 1
            if not stack or start == index:
                reject('coverage', 'invalid_query', 'atom outside a command')
            stack[-1].append(text[start:index])
    if stack:
        reject('coverage', 'invalid_query', 'unclosed SMT command')
    if not roots or any(not isinstance(command, list) or not command for command in roots):
        reject('coverage', 'invalid_query', 'empty SMT session')
    allowed = {'set-logic', 'declare-const', 'declare-fun', 'assert', 'check-sat'}
    if any(command[0] not in allowed for command in roots):
        reject('coverage', 'unsupported_session', 'only reviewed single-session commands are supported')
    for command in roots:
        if command[0] == 'declare-fun' and (len(command) != 4 or command[2] != []):
            reject('coverage', 'unsupported_session', 'only zero-arity scalar declarations are supported')
        if command[0] == 'declare-const' and len(command) != 3:
            reject('coverage', 'unsupported_session', 'invalid scalar declaration')
    if roots[-1] != ['check-sat'] or sum(command[0] == 'check-sat' for command in roots) != 1:
        reject('coverage', 'unsupported_session', 'exactly one final check-sat is required')
    return roots


def render(node):
    return '(' + ' '.join(render(value) if isinstance(value, list) else value for value in node) + ')'


def reference(value):
    if (not isinstance(value, dict) or not isinstance(value.get('fn'), str)
            or not isinstance(value.get('kind'), str) or type(value.get('idx')) is not int or value['idx'] < 0):
        reject('coverage', 'invalid_policy', 'invalid obligation reference')
    return value['fn'], value['kind'], value['idx']


def audit(args):
    if os.name != 'posix' or not 0.1 <= args.timeout_seconds <= 60:
        reject('binding', 'unsupported_environment', 'POSIX and timeout0.1..60seconds required')
    deadline = time.monotonic() + args.timeout_seconds
    def budget(stage):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            reject(stage, stage + '_timeout', 'total audit deadline exceeded')
        return remaining
    policy_raw = read(args.policy)
    if digest(policy_raw) != args.policy_sha256:
        reject('binding', 'policy_hash_mismatch', 'trusted expected-policy digest does not match')
    policy = decode(policy_raw)
    if not isinstance(policy, dict):
        reject('binding', 'invalid_policy', 'policy must be an object')
    if (policy.get('schema') != 'resilient-expected-implementation-coverage'
            or type(policy.get('version')) is not int or policy.get('version') != 1):
        reject('binding', 'invalid_policy', 'unsupported policy schema/version')
    if not args.profile or policy.get('profile') != args.profile:
        reject('binding', 'profile_mismatch', 'requested reviewed model profile differs')
    certs = Path(args.cert_dir).resolve(strict=True)
    manifest_raw = read(certs / 'manifest.json')
    manifest = decode(manifest_raw)
    binding = manifest.get('source_binding')
    if not isinstance(binding, dict) or digest(canonical(binding)) != policy.get('source_graph_sha256'):
        reject('binding', 'source_graph_mismatch', 'signed input graph differs from reviewed policy')
    entries = manifest.get('obligations')
    if not isinstance(entries, list) or len(entries) > MAX_SITES:
        reject('binding', 'resource_limit', 'manifest obligation count exceeds512')
    preflight_bytes = 0
    for entry in entries:
        filename = entry.get('cert')
        if (not isinstance(filename, str) or not filename.endswith('.smt2')
                or any(character in filename for character in '/\\:')
                or Path(filename).name != filename):
            reject('binding', 'invalid_query', 'query must have a confined relative filename')
        preflight_bytes += len(read(certs / filename, MAX_QUERY))
        if preflight_bytes > 16 * MAX_QUERY:
            reject('binding', 'resource_limit', 'total query bytes exceed16MiB')
    read(Path(args.pubkey).resolve(strict=True))
    compiler = Path(args.rz).resolve(strict=True)
    compiler_hash = digest(read(compiler, 256 * 1024 * 1024))
    if compiler_hash != policy.get('compiler', {}).get('sha256'):
        reject('binding', 'compiler_hash_mismatch', 'actual executable differs from reviewed policy')
    code, output, _ = bounded([str(compiler), '--version', '--verbose'], budget('binding'), 'binding')
    build_profile = policy['compiler'].get('build_profile')
    if code or not build_profile or ('  profile: ' + build_profile) not in output.decode().splitlines():
        reject('binding', 'build_profile_mismatch', 'actual build profile differs from policy')
    configured = shutil.which(os.environ.get('Z3_BINARY', 'z3'))
    if not configured:
        reject('binding', 'solver_unavailable', 'configured Z3_BINARY is unavailable')
    solver = str(Path(configured).resolve(strict=True))
    solver_hash = digest(read(solver, 256 * 1024 * 1024))
    if solver_hash != policy.get('solver', {}).get('sha256'):
        reject('binding', 'solver_hash_mismatch', 'actual configured solver differs from trusted policy')
    environment = dict(os.environ, Z3_BINARY=solver)
    code, output, errors = bounded([str(compiler), 'verify-all', str(certs), '--source-root', str(Path(args.source_root).resolve(strict=True)),
                                    '--pubkey', str(Path(args.pubkey).resolve(strict=True)), '--z3'], budget('authentication'), 'authentication', environment=environment)
    if code:
        reject('authentication', 'authentication_failed', (output + errors).decode(errors='replace')[-4096:])
    if (read(certs / 'manifest.json') != manifest_raw
            or digest(read(compiler, 256 * 1024 * 1024)) != compiler_hash
            or digest(read(solver, 256 * 1024 * 1024)) != solver_hash):
        reject('authentication', 'input_changed', 'manifest/compiler/solver changed during verification')
    sites = policy.get('sites')
    if not isinstance(sites, list) or not 1 <= len(sites) <= MAX_SITES:
        reject('coverage', 'invalid_policy', 'expected1..512explicit sites')
    source_paths = {entry['path'] for entry in binding['files']}
    identities, requested, unselected = set(), [], []
    for site in sites:
        if not isinstance(site, dict):
            reject('coverage', 'invalid_policy', 'site must be an object')
        source = site.get('source')
        path = PurePosixPath(source) if isinstance(source, str) else PurePosixPath('/')
        if source not in source_paths or path.is_absolute() or '..' in path.parts or str(path) != source:
            reject('coverage', 'invalid_policy', 'site source must name an authenticated input')
        identity = (source, site.get('declaration'), site.get('clause'), site.get('path'))
        if (not isinstance(identity[1], str) or not identity[1] or type(identity[2]) is not int
                or identity[2] < 0 or not isinstance(identity[3], str) or not identity[3]):
            reject('coverage', 'invalid_policy', 'site needs qualified declaration/clause/path identity')
        if len(identity[1].split('::')) < 2 or not all(identity[1].split('::')):
            reject('coverage', 'invalid_policy', 'declaration identity must be qualified')
        if identity in identities:
            reject('coverage', 'duplicate_site', identity)
        identities.add(identity)
        if site.get('status') == 'unselected':
            unselected.append(dict(zip(('source', 'declaration', 'clause', 'path'), identity)))
        elif site.get('status') != 'required':
            reject('coverage', 'unresolved_site', {'site': identity, 'status': site.get('status')})
        else:
            requested.append((identity, site))
    if not requested:
        reject('coverage', 'unresolved_site', 'no requested implementation sites')
    obligations = manifest['obligations']
    if not isinstance(obligations, list) or len(obligations) > MAX_SITES:
        reject('coverage', 'resource_limit', 'too many manifest obligations')
    actual = {}
    for obligation in obligations:
        key = reference(obligation)
        if key in actual:
            reject('coverage', 'duplicate_obligation', key)
        actual[key] = obligation
    expected, matched = set(), []
    query_bytes = 0
    for identity, site in requested:
        key = reference(site.get('obligation'))
        if key[1] != 'ensures_implementation':
            reject('coverage', 'wrong_basis', identity)
        if key in expected:
            reject('coverage', 'duplicate_site', 'multiple sites claim the same emitted obligation')
        expected.add(key)
        if key not in actual:
            if any(other[0] == key[0] and other[2] == key[2] for other in actual):
                reject('coverage', 'wrong_basis', identity)
            reject('coverage', 'missing_obligation', identity)
        obligation = actual[key]
        filename = obligation['cert']
        if not isinstance(filename, str) or Path(filename).name != filename:
            reject('coverage', 'invalid_query', 'query must have a single relative filename')
        query_path = certs / filename
        if query_path.is_symlink() or not query_path.resolve(strict=True).is_relative_to(certs):
            reject('coverage', 'invalid_query', 'query must be a confined regular file')
        raw = read(query_path, MAX_QUERY)
        query_bytes += len(raw)
        if query_bytes > 16 * MAX_QUERY:
            reject('coverage', 'resource_limit', 'total query bytes exceed16MiB')
        if digest(raw) != obligation['sha256']:
            reject('coverage', 'input_changed', 'query changed after authenticated replay')
        commands = sexpressions(raw)
        if commands != site.get('commands'):
            reject('coverage', 'checkpoint_mismatch', identity)
        feasibility = site.get('feasibility_commands')
        if not isinstance(feasibility, list):
            reject('coverage', 'invalid_feasibility_checkpoint', 'independently reviewed feasibility commands are required')
        feasibility_raw = ('\n'.join(render(command) for command in feasibility) + '\n').encode()
        if len(feasibility_raw) > MAX_QUERY or sexpressions(feasibility_raw) != feasibility:
            reject('coverage', 'invalid_feasibility_checkpoint', 'invalid bounded feasibility session')
        declarations = lambda session: [command for command in session if command[0] in ('set-logic', 'declare-const', 'declare-fun')]
        if declarations(commands) != declarations(feasibility):
            reject('coverage', 'invalid_feasibility_checkpoint', 'full query and feasibility checkpoint must share exact theory/symbol declarations')
        code, result, errors = bounded([solver, '-in', '-smt2'], budget('coverage'), 'coverage',
                                       feasibility_raw, environment=environment)
        if code or errors or result.strip() != b'sat':
            reject('coverage', 'infeasible_obligation' if not code and not errors and result.strip() == b'unsat' else 'unresolved_feasibility', identity)
        matched.append({'site': dict(zip(('source', 'declaration', 'clause', 'path'), identity)), 'obligation': key, 'query_sha256': digest(raw), 'feasibility_checkpoint_sha256': digest(canonical(feasibility)), 'reviewed_feasibility': 'sat'})
    if digest(read(solver, 256 * 1024 * 1024)) != solver_hash:
        reject('coverage', 'input_changed', 'solver changed during feasibility checks')
    auditor_hash = digest(read(Path(__file__).resolve()))
    budget('coverage')
    extras = [key for key in actual if key[1] == 'ensures_implementation' and key not in expected]
    if extras:
        reject('coverage', 'extra_obligation', extras)
    return {'stage': 'coverage', 'code': 'covered', 'claim': 'only the supplied independently reviewed policy; not automatic source completeness',
            'auditor_sha256': auditor_hash, 'policy_sha256': args.policy_sha256, 'compiler_sha256': compiler_hash, 'profile': args.profile,
            'source_graph_sha256': policy['source_graph_sha256'], 'solver_sha256': solver_hash,
            'sites': matched, 'unselected': unselected,
            'auxiliary_nonimplementation_obligations': sum(key[1] != 'ensures_implementation' for key in actual)}


def with_deadline(args):
    if (os.name != 'posix' or threading.current_thread() is not threading.main_thread()
            or not 0.1 <= args.timeout_seconds <= 60):
        reject('binding', 'unsupported_environment', 'POSIX main thread and timeout0.1..60seconds required')
    previous_handler = signal.getsignal(signal.SIGALRM)
    previous_timer = signal.getitimer(signal.ITIMER_REAL)
    started = time.monotonic()
    def expired(_signal, _frame):
        reject('binding', 'audit_timeout', 'outer audit deadline exceeded')
    signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, args.timeout_seconds)
    try:
        return audit(args)
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)
        if previous_timer[0] > 0:
            signal.setitimer(signal.ITIMER_REAL, max(0.000001, previous_timer[0] - (time.monotonic() - started)), previous_timer[1])


def main():
    class Parser(argparse.ArgumentParser):
        def error(self, message):
            reject('binding', 'invalid_arguments', message)
    parser = Parser(description=__doc__)
    for flag in ('rz', 'cert-dir', 'source-root', 'pubkey', 'policy', 'policy-sha256', 'profile'):
        parser.add_argument('--' + flag, required=True)
    parser.add_argument('--timeout-seconds', type=float, default=30)
    try:
        report = with_deadline(parser.parse_args())
    except Rejected as error:
        report = {'stage': error.stage, 'code': error.code, 'detail': error.detail}
    except (OSError, ValueError, KeyError, IndexError, TypeError, AttributeError, RecursionError, subprocess.SubprocessError) as error:
        report = {'stage': 'binding', 'code': 'invalid_input', 'detail': str(error)}
    print(json.dumps(report, sort_keys=True))
    return 0 if report['code'] == 'covered' else 1


if __name__ == '__main__':
    sys.exit(main())
