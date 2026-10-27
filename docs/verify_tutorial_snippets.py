#!/usr/bin/env python3
"""Execute tutorial source fences; fail when documentation coverage is empty."""

import argparse
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
OPEN_FENCE = re.compile(r" {0,3}(`{3,}|~{3,})(.*)")


def snippets(path):
    fence = None
    target = False
    body = []
    start = 0
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if fence is not None:
            if re.fullmatch(r" {0,3}" + re.escape(fence[0]) + "{" + str(len(fence)) + r",}[ \t]*", line):
                if target:
                    source = "\n".join(body) + "\n"
                    if not source.strip():
                        raise ValueError(f"{path}:{start}: empty resilient fence")
                    yield path, start, source
                fence = None
            elif target:
                body.append(line)
            continue
        match = OPEN_FENCE.fullmatch(line)
        if match is None:
            continue
        marker, info = match.groups()
        if marker[0] == "`" and "`" in info:
            continue
        fence = marker
        target = info.strip().split(maxsplit=1)[:1] == ["resilient"]
        body = []
        start = number
    if fence is not None:
        raise ValueError(f"{path}:{start}: unterminated code fence")


def compiler_path(value):
    if not value:
        for mode in ("release", "debug"):
            candidate = ROOT / "resilient" / "target" / mode / "rz"
            if candidate.is_file() and os.access(candidate, os.X_OK):
                return str(candidate)
        value = "rz"
    if "/" in value and not Path(value).is_absolute():
        value = str(ROOT / value)
    found = shutil.which(value)
    if found is None:
        raise ValueError(f"rz executable not found: {value}; set RESILIENT_BIN or --compiler")
    return str(Path(found).resolve())


def verify(compiler, directory, timeout):
    files = sorted(directory.glob("*.md"))
    if not files:
        raise ValueError(f"no tutorial Markdown files in {directory}")
    extracted = [snippet for path in files for snippet in snippets(path)]
    if not extracted:
        raise ValueError(f"no resilient snippets found in {directory}")
    failed = 0
    with tempfile.TemporaryDirectory(prefix="resilient-tutorial-") as temporary:
        root = Path(temporary)
        for index, (path, line, source) in enumerate(extracted, 1):
            program = root / f"snippet-{index}.rz"
            program.write_text(source, encoding="utf-8")
            with (root / "output.log").open("w+b") as output:
                try:
                    result = subprocess.run(
                        [compiler, "--seed", "0", "--no-cache", str(program)],
                        cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=timeout,
                    )
                    status = None if result.returncode == 0 else f"exit {result.returncode}"
                except subprocess.TimeoutExpired:
                    status = f"timeout after {timeout:g}s"
                if status is not None:
                    failed += 1
                    end = output.seek(0, os.SEEK_END)
                    output.seek(max(0, end - 8192))
                    detail = output.read().decode("utf-8", errors="replace")
                    print(f"{path}:{line}: FAIL ({status})\n{detail}", file=sys.stderr)
                else:
                    print(f"{path}:{line}: OK")
    print(f"Checked {len(extracted)} snippets; {failed} failed.")
    return 1 if failed else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default=os.environ.get("RESILIENT_BIN"))
    parser.add_argument("--tutorial-dir", type=Path, default=ROOT / "docs" / "tutorial")
    parser.add_argument("--timeout", type=float, default=30, help="seconds per snippet")
    args = parser.parse_args()
    try:
        if not math.isfinite(args.timeout) or args.timeout <= 0:
            raise ValueError("timeout must be finite and positive")
        return verify(compiler_path(args.compiler), args.tutorial_dir, args.timeout)
    except (OSError, ValueError) as error:
        print(f"Tutorial verification failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
