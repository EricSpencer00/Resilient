#!/usr/bin/env python3
"""Enumerate lexical contract sites in the versioned restricted declaration grammar."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import signal
import stat
import sys
import threading
import time

GRAMMAR_SHA256 = '70464aaa428d9395b2d85d191bad292b34dbdaf09f151dffee86714a215a885f'
MAX_FILE = 131072
MAX_TOTAL = 1048576
MAX_TOKENS = 65536
MAX_DEPTH = 32


class Rejected(Exception):
    def __init__(self, code, detail):
        self.code, self.detail = code, str(detail)


def reject(code, detail):
    raise Rejected(code, detail)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path, limit):
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
            reject('resource_limit', 'expected bounded regular input: ' + str(path))
        chunks, size = [], 0
        while size <= limit:
            chunk = os.read(descriptor, min(65536, limit + 1 - size))
            if not chunk:
                break
            chunks.append(chunk)
            size += len(chunk)
        if size > limit:
            reject('resource_limit', 'input exceeds byte limit')
        return b''.join(chunks)
    finally:
        os.close(descriptor)


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            reject('invalid_graph', 'duplicate JSON key')
        result[key] = value
    return result


def relative(value):
    if (not isinstance(value, str) or not value or len(value) > 256
            or any(char in value for char in '\\:\x00')
            or PurePosixPath(value).is_absolute() or '..' in PurePosixPath(value).parts
            or str(PurePosixPath(value)) != value or not value.endswith('.rz')):
        reject('invalid_graph', 'expected confined canonical .rz path')
    return value


def source_path(root, name):
    path = root
    for part in PurePosixPath(relative(name)).parts:
        path /= part
        if path.is_symlink():
            reject('invalid_graph', 'source symlink is outside this stable-snapshot profile')
    path.resolve(strict=True).relative_to(root)
    return path


def lex(raw):
    raw.decode('utf-8', errors='strict')
    tokens, index = [], 0
    while index < len(raw):
        begin, byte = index, raw[index]
        if byte in b' \t\r\n':
            index += 1
            continue
        if raw[index:index + 2] == b'//':
            end = raw.find(b'\n', index + 2)
            index = len(raw) if end < 0 else end + 1
            continue
        if raw[index:index + 2] == b'/*':
            end = raw.find(b'*/', index + 2)
            nested = raw.find(b'/*', index + 2, end if end >= 0 else len(raw))
            if end < 0 or nested >= 0:
                reject('invalid_lexical_input', 'unterminated/nested comment outside grammar')
            index = end + 2
            continue
        if byte == 34:
            index += 1
            while index < len(raw) and raw[index] != 34:
                if raw[index] in (10, 13):
                    reject('invalid_lexical_input', 'literal string newline outside grammar')
                if raw[index] == 92:
                    index += 1
                    if index >= len(raw) or raw[index] not in b'\\"nrt':
                        reject('invalid_lexical_input', 'unsupported string escape')
                index += 1
            if index >= len(raw):
                reject('invalid_lexical_input', 'unterminated string')
            index += 1
            value = json.loads(raw[begin:index].decode())
            token = ('string', value, begin, index)
        elif 65 <= byte <= 90 or 97 <= byte <= 122 or byte == 95:
            index += 1
            while index < len(raw) and (65 <= raw[index] <= 90 or 97 <= raw[index] <= 122
                                        or 48 <= raw[index] <= 57 or raw[index] == 95):
                index += 1
            token = ('identifier', raw[begin:index].decode('ascii'), begin, index)
        elif 48 <= byte <= 57:
            index += 1
            while index < len(raw) and 48 <= raw[index] <= 57:
                index += 1
            token = ('integer', raw[begin:index].decode('ascii'), begin, index)
        else:
            pair = raw[index:index + 2]
            if pair in (b'->', b'::', b'==', b'!=', b'>=', b'<=', b'&&', b'||'):
                index += 2
            elif byte in b'{}()[],;:.+-*/%<>!=&|':
                index += 1
            else:
                reject('invalid_lexical_input', 'unsupported token at byte ' + str(index))
            token = ('symbol', raw[begin:index].decode('ascii'), begin, index)
        tokens.append(token)
        if len(tokens) > MAX_TOKENS:
            reject('resource_limit', 'source token bound exceeded')
    return tokens


class Parser:
    PRECEDENCE = {'||': 1, '&&': 2, '==': 3, '!=': 3, '<': 4, '>': 4, '<=': 4, '>=': 4,
                  '+': 5, '-': 5, '*': 6, '/': 6, '%': 6}

    def __init__(self, path, raw):
        self.path, self.raw, self.tokens, self.position = path, raw, lex(raw), 0
        self.declarations, self.sites, self.imports, self.identities = [], [], [], set()

    def peek(self):
        if self.position >= len(self.tokens):
            return None
        token = self.tokens[self.position]
        # A literal must neither act as syntax nor masquerade as end-of-input.
        return ('string-literal',) if token[0] == 'string' else token[1]

    def take(self, value=None):
        if self.position >= len(self.tokens):
            reject('unsupported_grammar', 'expected ' + str(value) + ' in ' + self.path)
        token = self.tokens[self.position]
        if value is not None:
            expected_kind = 'identifier' if value.isidentifier() else 'symbol'
            if token[0] != expected_kind or token[1] != value:
                reject('unsupported_grammar', 'expected ' + str(value) + ' in ' + self.path)
        self.position += 1
        return token

    def identifier(self):
        token = self.take()
        if token[0] != 'identifier' or token[1] in ('fn', 'mod', 'impl', 'use', 'struct', 'requires', 'ensures'):
            reject('unsupported_grammar', 'expected bounded ASCII identifier')
        return token[1]

    def expression(self, minimum=0, depth=0):
        if depth > MAX_DEPTH:
            reject('resource_limit', 'contract expression depth exceeded')
        if self.peek() in ('!', '-', '+'):
            self.take()
            self.expression(7, depth + 1)
        elif self.peek() == '(':
            self.take('(')
            self.expression(0, depth + 1)
            self.take(')')
        elif self.position < len(self.tokens) and self.tokens[self.position][0] in ('identifier', 'integer'):
            self.identifier() if self.tokens[self.position][0] == 'identifier' else self.take()
        else:
            reject('unsupported_grammar', 'unsupported contract expression atom')
        while self.peek() in ('.', '('):
            if self.peek() == '.':
                self.take('.')
                self.identifier()
            else:
                self.take('(')
                if self.peek() != ')':
                    self.expression(0, depth + 1)
                    while self.peek() == ',':
                        self.take(',')
                        self.expression(0, depth + 1)
                self.take(')')
        while self.peek() in self.PRECEDENCE and self.PRECEDENCE[self.peek()] >= minimum:
            precedence = self.PRECEDENCE[self.take()[1]]
            self.expression(precedence + 1, depth + 1)

    def body(self):
        self.take('{')
        stack = ['}']
        closing = {'{': '}', '(': ')', '[': ']'}
        while stack:
            token = self.take()
            value = token[1]
            if token[0] == 'string':
                continue
            if value in ('fn', 'mod', 'impl', 'use', 'struct', 'requires', 'ensures'):
                reject('unsupported_grammar', 'declaration-bearing body syntax outside grammar')
            if value in closing:
                stack.append(closing[value])
                if len(stack) > MAX_DEPTH:
                    reject('resource_limit', 'body delimiter depth exceeded')
            elif value in ('}', ')', ']'):
                if value != stack.pop():
                    reject('unsupported_grammar', 'unbalanced body delimiters')

    def function(self, scope, method):
        begin = self.take('fn')[2]
        name = self.identifier()
        kind = 'method' if method else 'function'
        identity = (kind, tuple(scope), name)
        if identity in self.identities:
            reject('duplicate_declaration', str(identity))
        self.identities.add(identity)
        self.take('(')
        if self.peek() != ')':
            while True:
                parameter = self.identifier()
                if parameter != 'self':
                    self.identifier()
                elif not method:
                    reject('unsupported_grammar', 'self outside method parameter')
                if self.peek() != ',':
                    break
                self.take(',')
        self.take(')')
        if self.peek() == '->':
            self.take('->')
            self.identifier()
        clauses, ordinals = [], {}
        while self.peek() in ('requires', 'ensures'):
            clause = self.take()
            expression_start = self.position
            self.expression()
            end = self.tokens[self.position - 1][3]
            ordinal = ordinals.get(clause[1], 0)
            ordinals[clause[1]] = ordinal + 1
            clauses.append({'clause_kind': clause[1], 'clause_ordinal': ordinal,
                            'clause_span': [clause[2], end], 'clause_sha256': digest(self.raw[clause[2]:end]),
                            'clause_text': self.raw[self.tokens[expression_start][2]:end].decode()})
        if self.peek() != '{':
            reject('unsupported_grammar', 'expected function body after contract header')
        end = self.tokens[self.position][2]
        base = {'path': self.path, 'declaration_kind': kind, 'lexical_scope': list(scope), 'name': name}
        self.declarations.append({**base, 'header_span': [begin, end], 'header_sha256': digest(self.raw[begin:end])})
        self.sites.extend({**base, **clause} for clause in clauses)
        self.body()

    def declarations_block(self, scope=(), depth=0, method=False, nested=False):
        if depth > MAX_DEPTH:
            reject('resource_limit', 'declaration depth exceeded')
        while self.peek() is not None and self.peek() != '}':
            token = self.peek()
            if token == 'fn':
                self.function(scope, method)
            elif not method and token in ('mod', 'impl'):
                self.take()
                name = self.identifier()
                self.take('{')
                self.declarations_block((*scope, name), depth + 1, token == 'impl', True)
            elif not method and token == 'struct':
                self.take('struct')
                self.identifier()
                self.take('{')
                while self.peek() != '}':
                    self.identifier()
                    self.identifier()
                    if self.peek() == ',':
                        self.take(',')
                self.take('}')
            elif not method and token == 'use':
                self.take('use')
                specifier = self.take()
                if specifier[0] != 'string' or not specifier[1].endswith('.rz'):
                    reject('unsupported_grammar', 'only literal local .rz imports supported')
                alias = None
                if self.peek() == 'as':
                    self.take('as')
                    alias = self.identifier()
                self.take(';')
                import_path = PurePosixPath(self.path).parent / specifier[1]
                target = relative(posixpath.normpath(str(import_path)))
                self.imports.append({'from': self.path, 'specifier': specifier[1], 'namespace': alias, 'target': target})
            else:
                reject('unsupported_grammar', 'unknown declaration or executable entry syntax: ' + str(token))
            if len(self.declarations) > 256 or len(self.sites) > 1024:
                reject('resource_limit', 'declaration/clause bound exceeded')
        if nested:
            self.take('}')
        elif self.peek() is not None:
            reject('unsupported_grammar', 'unexpected closing delimiter')


def inventory(args):
    grammar = read(args.grammar, MAX_FILE)
    if digest(grammar) != GRAMMAR_SHA256:
        reject('unsupported_grammar', 'grammar bytes differ from reviewed version')
    graph_raw = read(args.graph, MAX_TOTAL)
    if digest(graph_raw) != args.graph_sha256:
        reject('graph_hash_mismatch', 'trusted source graph digest differs')
    graph = json.loads(graph_raw, object_pairs_hook=unique)
    if (not isinstance(graph, dict) or type(graph.get('version')) is not int or graph['version'] != 1 or graph.get('builtins') != []
            or not isinstance(graph.get('files'), list) or not 1 <= len(graph['files']) <= 32
            or not isinstance(graph.get('imports'), list) or len(graph['imports']) > 256):
        reject('invalid_graph', 'bounded version1 local graph required')
    entry = relative(graph.get('entry'))
    root = Path(args.source_root).resolve(strict=True)
    files, declarations, sites, imports, names, total, token_count = [], [], [], [], set(), 0, 0
    for file in graph['files']:
        if (not isinstance(file, dict) or set(file) != {'path', 'bytes', 'sha256'}
                or type(file.get('bytes')) is not int or not 0 <= file['bytes'] <= MAX_FILE
                or not isinstance(file.get('sha256'), str) or len(file['sha256']) != 64
                or any(char not in '0123456789abcdef' for char in file['sha256'])):
            reject('invalid_graph', 'invalid file record')
        name = relative(file.get('path'))
        if name in names:
            reject('invalid_graph', 'duplicate file identity')
        names.add(name)
        raw = read(source_path(root, name), MAX_FILE)
        total += len(raw)
        if total > MAX_TOTAL:
            reject('resource_limit', 'total source bytes exceeded')
        actual = {'path': name, 'bytes': len(raw), 'sha256': digest(raw)}
        if file != actual:
            reject('source_hash_mismatch', name)
        parser = Parser(name, raw)
        token_count += len(parser.tokens)
        if token_count > MAX_TOKENS:
            reject('resource_limit', 'whole graph token bound exceeded')
        parser.declarations_block()
        files.append(actual)
        declarations.extend(parser.declarations)
        sites.extend(parser.sites)
        imports.extend(parser.imports)
        if len(declarations) > 256 or len(sites) > 1024:
            reject('resource_limit', 'graph declaration/clause bound exceeded')
    if entry not in names:
        reject('invalid_graph', 'missing entry source')
    def edge(value):
        if (not isinstance(value, dict) or set(value) != {'from', 'specifier', 'namespace', 'target'}
                or not isinstance(value['specifier'], str)
                or value['namespace'] is not None and not isinstance(value['namespace'], str)):
            reject('invalid_graph', 'invalid edge record')
        return relative(value['from']), value['specifier'], value['namespace'], relative(value['target'])
    if Counter(map(edge, imports)) != Counter(map(edge, graph['imports'])):
        reject('import_graph_mismatch', 'source imports differ including alias/repetition multiplicity')
    reachable = {entry}
    for _ in range(len(names)):
        reachable.update(value['target'] for value in imports if value['from'] in reachable)
    if reachable != names:
        reject('invalid_graph', 'source graph has missing/unreachable files')
    return {'code': 'inventoried', 'grammar': 'restricted-declarations-v1', 'grammar_sha256': GRAMMAR_SHA256,
            'graph_sha256': args.graph_sha256, 'files': files, 'declarations': declarations,
            'contract_sites': sites, 'import_edges': imports,
            'claim': 'independent restricted lexical enumeration only; no signature authentication, compiler validity, resolved call mapping, proof eligibility or qualification acceptance'}


def main():
    class Arguments(argparse.ArgumentParser):
        def error(self, message):
            reject('invalid_arguments', message)
    previous = timer = started = None
    try:
        parser = Arguments(description=__doc__)
        for name in ('source-root', 'graph', 'graph-sha256', 'grammar'):
            parser.add_argument('--' + name, required=True)
        args = parser.parse_args()
        if os.name != 'posix' or threading.current_thread() is not threading.main_thread():
            reject('unsupported_environment', 'POSIX main thread required')
        previous, timer, started = signal.getsignal(signal.SIGALRM), signal.getitimer(signal.ITIMER_REAL), time.monotonic()
        def expired(_signal, _frame):
            reject('resource_limit', 'whole inventory deadline exceeded')
        signal.signal(signal.SIGALRM, expired)
        signal.setitimer(signal.ITIMER_REAL, 20)
        report = inventory(args)
        if time.monotonic() - started >= 20:
            reject('resource_limit', 'whole inventory deadline exceeded')
    except Rejected as error:
        report = {'code': error.code, 'detail': error.detail}
    except (OSError, ValueError, KeyError, TypeError, IndexError, RecursionError) as error:
        report = {'code': 'invalid_input', 'detail': str(error)}
    finally:
        if previous is not None:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous)
            if timer[0] > 0:
                signal.setitimer(signal.ITIMER_REAL, max(0.000001, timer[0] - (time.monotonic() - started)), timer[1])
    print(json.dumps(report, sort_keys=True))
    return 0 if report['code'] == 'inventoried' else 1


if __name__ == '__main__':
    sys.exit(main())
