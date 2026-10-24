#!/usr/bin/env python3
"""Verify the published curl-only install on a fresh hosted runner."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

repo=Path(__file__).resolve().parents[1]
tag=sys.argv[1]
assert tag.startswith('v') and tag[1:].replace('.','').isdigit()
base=f'https://github.com/EricSpencer00/Resilient/releases/download/{tag}'
manifest=json.loads(subprocess.check_output(['curl','-fsSL',base+'/release-manifest.json'],timeout=60))
source=subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()
assert manifest['source_commit']==source
arch=subprocess.check_output(['uname','-m'],text=True).strip()
system=subprocess.check_output(['uname','-s'],text=True).strip()
target=('aarch64' if arch in ('aarch64','arm64') else 'x86_64')+('-apple-darwin' if system=='Darwin' else '-unknown-linux-gnu')
record=next(item for item in manifest['platforms'] if item['target']==target)
with tempfile.TemporaryDirectory(prefix='rz-public-install-') as temporary:
    root=Path(temporary); tools=root/'tools'; tools.mkdir(); marker=root/'unexpected-tool'
    for tool in ('git','cargo','rustc','python','python3','jq'):
        sentinel=tools/tool
        sentinel.write_text('#!/bin/sh\nprintf used > "$RZ_TEST_UNEXPECTED_TOOL"\nexit 97\n')
        sentinel.chmod(0o755)
    prefix=root/'prefix'; profile=root/'profile'
    environment=dict(os.environ,RZ_PREFIX=str(prefix),RZ_PROFILE=str(profile),RZ_VERSION=tag,RZ_TEST_UNEXPECTED_TOOL=str(marker),PATH=str(tools)+os.pathsep+os.environ['PATH'])
    command=f'curl -fsSL {base}/install.sh | bash'
    subprocess.run(['bash','-o','pipefail','-c',command],env=environment,check=True,timeout=180)
    assert not marker.exists(), 'installer invoked an additional language or source-build tool'
    binary=prefix/'bin/rz'
    digest=hashlib.sha256(binary.read_bytes()).hexdigest()
    assert digest==record['binary_sha256']
    shell=subprocess.check_output(['bash','--noprofile','--norc','-c','. "$1"; rz --version','--',str(profile)],text=True)
    assert shell.startswith('rz '+tag[1:]+':'),shell
    subprocess.run(['bash',str(repo/'scripts/release-smoke-test.sh'),str(binary)],check=True,timeout=90)
    subprocess.run(['bash',str(repo/'scripts/release-lsp-smoke-test.sh'),str(binary)],check=True,timeout=90)
    print(json.dumps({'command':command,'source_commit':source,'target':target,'installed_binary_sha256':digest,'fresh_shell_version':shell.strip(),'additional_tools_invoked':False},sort_keys=True,indent=2))
