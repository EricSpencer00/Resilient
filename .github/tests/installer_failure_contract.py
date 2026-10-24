#!/usr/bin/env python3
"""Additional atomic-failure coverage without changing the frozen six contracts."""
from pathlib import Path
import unittest
import installer_contract as fixtures

class InstallerFailure(unittest.TestCase):
    def setUp(self):
        self.fixture=fixtures.InstallerContract('test_install_configures_reusable_shell_without_manual_export')
        self.fixture.setUp()
        binary=self.fixture.prefix/'bin/rz'
        binary.parent.mkdir(parents=True)
        binary.write_text('previous installation')
        self.fixture.profile.write_text('# previous profile\n')

    def tearDown(self):
        self.fixture.tearDown()

    def unchanged(self):
        self.assertEqual((self.fixture.prefix/'bin/rz').read_text(),'previous installation')
        self.assertEqual(self.fixture.profile.read_text(),'# previous profile\n')

    def test_non_runnable_candidate_preserves_installation_and_profile(self):
        self.fixture.make_archive({'rz':b'not an executable'})
        self.assertNotEqual(self.fixture.install().returncode,0)
        self.unchanged()

    def test_wrong_version_preserves_installation_and_profile(self):
        self.fixture.make_archive({'rz':b'#!/bin/sh\nprintf "rz 1.1.0\\n"\n'})
        self.assertNotEqual(self.fixture.install().returncode,0)
        self.unchanged()

    def test_unwritable_profile_preserves_installation_and_profile(self):
        self.fixture.profile.chmod(0o444)
        self.assertNotEqual(self.fixture.install().returncode,0)
        self.unchanged()

    def test_checksum_failure_preserves_existing_profile(self):
        (self.fixture.transport/'SHA256SUMS').write_text('')
        self.assertNotEqual(self.fixture.install().returncode,0)
        self.unchanged()

if __name__=='__main__': unittest.main(verbosity=2)
