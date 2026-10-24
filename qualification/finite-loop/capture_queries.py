#!/usr/bin/env python3
"""Retain source-bound signed finite-loop queries from a hosted compiler build."""
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

repo = pathlib.Path(__file__).resolve().parents[2]
binary = pathlib.Path(sys.argv[1]).resolve()
assert binary == (repo / "resilient/target/debug/rz").resolve()
output = pathlib.Path(sys.argv[2]).resolve()
output.mkdir(parents=True, exist_ok=True)
# These are explanatory checkpoint identities; the solver verdict comes only
# from replaying the compiler's unmodified, signed SMT-LIB files below.
cases = {
    "shadow": ("let x = -10; for x in [1, 2] invariant x >= 0 { }", [{"x": 1}, {"x": 2}]),
    "rebind": ("for x in [1, 2] invariant x >= 0 { x = -1; }", [{"x": 1}, {"x": 2}]),
    "accumulation": ("let s = 0; for x in [1, 2, 3] invariant s <= 6 { s = s + x; }", [{"x": 1, "s": 0}, {"x": 2, "s": 1}, {"x": 3, "s": 3}]),
}

def run(*args):
    return subprocess.check_output(args, stderr=subprocess.PIPE, timeout=60)

with tempfile.TemporaryDirectory(prefix="res-finite-signing-") as temporary:
    key_root = pathlib.Path(temporary)
    openssl_key = key_root / "openssl.pem"
    run("openssl", "genpkey", "-algorithm", "ed25519", "-out", str(openssl_key))
    private_der = run("openssl", "pkey", "-in", str(openssl_key), "-outform", "DER")
    public_der = run("openssl", "pkey", "-in", str(openssl_key), "-pubout", "-outform", "DER")
    assert private_der[:-32].hex() == "302e020100300506032b657004220420"
    assert public_der[:-32].hex() == "302a300506032b6570032100"
    private = key_root / "private.pem"
    private.write_text("-----BEGIN ED25519 PRIVATE KEY-----\n" + private_der[-32:].hex() + "\n-----END ED25519 PRIVATE KEY-----\n")
    public = output / "public.pem"
    public.write_text("-----BEGIN ED25519 PUBLIC KEY-----\n" + public_der[-32:].hex() + "\n-----END ED25519 PUBLIC KEY-----\n")
    for name, (source, entries) in cases.items():
        case_root = output / name
        case_root.mkdir()
        program = case_root / "program.rz"
        program.write_text(source + "\n")
        certificates = case_root / "certificates"
        run(str(binary), "--no-cache", "-t", "--emit-certificate", str(certificates), "--sign-cert", str(private), str(program))
        run(str(binary), "verify-all", str(certificates), "--pubkey", str(public), "--require-manifest-signature", "--z3")
        manifest = json.loads((certificates / "manifest.json").read_text())
        assert len(manifest["obligations"]) == len(entries) + 1
        queries = []
        for index, obligation in enumerate(manifest["obligations"]):
            query = certificates / obligation["cert"]
            text = query.read_text()
            assert text.count("(check-sat)") == 1
            verdict = run("z3", "-smt2", str(query)).decode().strip()
            assert verdict == "unsat", (query, verdict)
            checkpoints = [0] if index == 0 else list(range(index - 1, min(index + 1, len(entries))))
            queries.append({"cert": obligation["cert"], "sha256": hashlib.sha256(query.read_bytes()).hexdigest(), "checkpoints": checkpoints, "verdict": verdict})
        record = {"source_commit": run("git", "-C", str(repo), "rev-parse", "HEAD").decode().strip(), "source_tree": run("git", "-C", str(repo), "rev-parse", "HEAD^{tree}").decode().strip(), "compiler_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "program_sha256": hashlib.sha256(program.read_bytes()).hexdigest(), "entry_checkpoints": entries, "queries": queries}
        (case_root / "query-review.json").write_text(json.dumps(record, sort_keys=True, indent=2) + "\n")
        print(f"retained {name}: {len(queries)} signed queries, independently unsat")
