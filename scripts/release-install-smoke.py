#!/usr/bin/env python3
"""Install assembled native archives through the exact landing-page command."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

repo = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(repo/'.github/tests'))
from installer_contract import InstallerContract

assets = Path(sys.argv[1]).resolve()
fixture = InstallerContract('test_install_configures_reusable_shell_without_manual_export')
fixture.setUp()
try:
    record = next(item for item in json.loads((assets/'release-manifest.json').read_text())['platforms'] if item['target'] == fixture.target())
    source = subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()
    assert record['source_commit'] == source
    for path in assets.iterdir():
        if path.is_file(): shutil.copyfile(path, fixture.transport/path.name)
    for tool in ('git','cargo','rustc'):
        sentinel=fixture.tools/tool
        sentinel.write_text('#!/bin/sh\nprintf used > "$RZ_TEST_UNEXPECTED_TOOL"\nexit 97\n')
        sentinel.chmod(0o755)
    marker=fixture.root/'unexpected-tool'
    fixture.env.update(RZ_VERSION=record['tag'],RZ_TEST_UNEXPECTED_TOOL=str(marker))
    command=f"curl -fsSL https://github.com/EricSpencer00/Resilient/releases/download/{record['tag']}/install.sh | bash"
    subprocess.run(['bash','-o','pipefail','-c',command],env=fixture.env,check=True,timeout=120)
    binary=fixture.prefix/'bin/rz'
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == record['binary_sha256']
    assert not marker.exists(), 'installation invoked a source-build prerequisite'
    shell=subprocess.check_output(['bash','--noprofile','--norc','-c','. "$1"; rz --version','--',str(fixture.profile)],env=fixture.env,text=True)
    assert shell.startswith('rz '+record['version']+':'),shell
    subprocess.run(['bash',str(repo/'scripts/release-smoke-test.sh'),str(binary)],check=True,timeout=60)
    subprocess.run(['bash',str(repo/'scripts/release-lsp-smoke-test.sh'),str(binary)],check=True,timeout=60)
    print(json.dumps({'source_commit':source,'target':record['target'],'installed_binary_sha256':record['binary_sha256'],'command':command,'fresh_shell_version':shell.strip(),'source_build_tools_invoked':False},sort_keys=True,indent=2))
finally:
    fixture.tearDown()
