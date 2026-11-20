import json
from pathlib import Path
import subprocess
import sys
import unittest
import tempfile
import hashlib

HERE = Path(__file__).resolve().parent
CLI = HERE / 'rpf9_recovered_catalog.py'


class RecoveredCatalog(unittest.TestCase):
    def run_cli(self, *args, check=True):
        return subprocess.run([sys.executable, str(CLI), *map(str, args)],
                              check=check, text=True, capture_output=True)

    def test_list_only_selected_nested_archive(self):
        result = json.loads(self.run_cli('list', 'script.rpf', '--base', '0xc4000').stdout)
        self.assertFalse(result['native_toc_decoded'])
        self.assertEqual(len(result['entries']), 1012)
        self.assertEqual(result['entries'][0]['stored_start'], '0xd6000')

    def test_extract_matches_original_verified_resource(self):
        with tempfile.TemporaryDirectory() as d:
            dst = Path(d) / 'one.psin'
            self.run_cli('extract', 'script.rpf', '--base', '0xc4000', '--ordinal', 0, '--out', dst)
            original = HERE / 'script' / '000D6000.psin'
            self.assertEqual(hashlib.sha256(dst.read_bytes()).digest(),
                             hashlib.sha256(original.read_bytes()).digest())
            self.assertNotEqual(self.run_cli('extract', 'script.rpf', '--base', '0xc4000',
                                             '--ordinal', 0, '--out', dst, check=False).returncode, 0)

    def test_unknown_archive_does_not_return_fake_empty_toc(self):
        r = self.run_cli('list', 'script.rpf', '--base', '0x1000', check=False)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('no verified PSIN catalog', r.stderr)


if __name__ == '__main__':
    unittest.main()
