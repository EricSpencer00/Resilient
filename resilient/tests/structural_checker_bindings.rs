//! Narrow binding controls supplement the immutable source/replay cases.
#![cfg(unix)]
use std::process::Command;
#[test]
fn descriptor_cannot_relabel_site_or_reorder_indices() {
    let output = Command::new("python3")
        .arg("-c")
        .arg(r#"
import hashlib, importlib.util, json, pathlib, tempfile, types, sys
root=pathlib.Path(sys.argv[1]); script=root/'agent-scripts/proof-correspondence-checker.py'
spec=importlib.util.spec_from_file_location('checker',script); m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
original=json.loads((root/'resilient/tests/fixtures/proof_correspondence/legacy_if_else.json').read_bytes())
for field,value in [('fn','other:ensures:0'),('kind','requires'),('idx',1)]:
    candidate=json.loads(json.dumps(original)); candidate['paths'][0]['obligation'][field]=value
    with tempfile.TemporaryDirectory() as directory:
        path=pathlib.Path(directory)/'descriptor.json'; raw=json.dumps(candidate).encode(); path.write_bytes(raw)
        args=types.SimpleNamespace(descriptor=str(path),descriptor_sha256=hashlib.sha256(raw).hexdigest(),profile=m.PROFILE)
        try: m.descriptor(args)
        except m.core.Rejected as e: assert e.stage=='binding' and e.code=='invalid_descriptor', (e.stage,e.code)
        else: raise AssertionError('mislabeled descriptor accepted')
print('three explicit identity/order controls rejected')
"#)
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .output().unwrap();
    assert!(output.status.success(), "{output:?}");
}
#[test]
fn changed_imported_authentication_helper_is_rejected() {
    let output = Command::new("python3")
        .arg("-c")
        .arg(r#"
import importlib.util,pathlib,shutil,tempfile,types,sys
root=pathlib.Path(sys.argv[1])/'agent-scripts'
with tempfile.TemporaryDirectory() as directory:
    path=pathlib.Path(directory)
    for name in ['proof-correspondence-checker.py','audit-proof-coverage.py']: shutil.copyfile(root/name,path/name)
    with (path/'audit-proof-coverage.py').open('a') as stream: stream.write('\n# altered reviewed dependency\n')
    spec=importlib.util.spec_from_file_location('changed_checker',path/'proof-correspondence-checker.py'); m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    try: m.audit(types.SimpleNamespace(timeout_seconds=20),{'authenticated':False})
    except m.core.Rejected as e: assert e.stage=='binding' and e.code=='dependency_hash_mismatch',(e.stage,e.code)
    else: raise AssertionError('changed helper accepted')
print('changed dependency rejected before source or replay')
"#)
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .output().unwrap();
    assert!(output.status.success(), "{output:?}");
}
