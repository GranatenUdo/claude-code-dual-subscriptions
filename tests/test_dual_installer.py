"""Offline ownership/rollback tests; all mutations are confined to temporary roots."""
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
import fcntl
import gzip
import time
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("installer", Path(__file__).parents[1] / "scripts/dual-install.py")
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "dual"

    def archive(self, extra=None):
        path = self.base / "artifact.tar.gz"
        elf = b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x3e\x00" + b"synthetic-not-executable"
        with tarfile.open(path, "w:gz") as tar:
            for name, content in [("claude-code-dual", elf), ("claude-code-proxy", elf), ("LICENSE", b"MIT synthetic test fixture")]:
                member = tarfile.TarInfo(name)
                member.size = len(content)
                tar.addfile(member, io.BytesIO(content))
            if extra:
                tar.addfile(extra)
        return path, installer.digest(path.read_bytes())

    def install(self, version="0.1.0"):
        path, checksum = self.archive()
        installer.install(self.root, path, version, checksum)

    def test_install_upgrade_rollback_uninstall_preserves_unrelated_data(self):
        unrelated = self.base / "credentials-canary"
        unrelated.write_text("never touched")
        self.install()
        self.install("0.2.0")
        self.assertEqual(installer.current_version(self.root), "0.2.0")
        installer.activate(self.root, "0.1.0")
        self.assertEqual(installer.current_version(self.root), "0.1.0")
        installer.uninstall(self.root)
        self.assertFalse(self.root.exists())
        self.assertEqual(unrelated.read_text(), "never touched")

    def test_checksum_failure_has_no_install_side_effects(self):
        path, _ = self.archive()
        with self.assertRaisesRegex(ValueError, "checksum"):
            installer.install(self.root, path, "0.1.0", "0" * 64)
        self.assertFalse(self.root.exists())

    def test_links_traversal_and_extra_archive_members_are_rejected(self):
        for name, kind in [("../escape", tarfile.REGTYPE), ("unowned", tarfile.REGTYPE), ("claude-code-dual", tarfile.SYMTYPE)]:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.linkname = "/tmp/never"
            path, checksum = self.archive(member)
            with self.assertRaises(ValueError):
                installer.install(self.root, path, "0.1.0", checksum)
            self.assertFalse(self.root.exists())

    def test_modified_or_unowned_files_prevent_removal(self):
        self.install()
        target = self.root / "versions/0.1.0/claude-code-dual"
        target.write_text("user edited")
        with self.assertRaisesRegex(ValueError, "changed"):
            installer.uninstall(self.root)
        self.assertTrue(target.exists())

    def test_existing_unowned_root_is_never_adopted(self):
        self.root.mkdir()
        canary = self.root / "keep"
        canary.write_text("keep")
        path, checksum = self.archive()
        with self.assertRaises((OSError, ValueError)):
            installer.install(self.root, path, "0.1.0", checksum)
        self.assertEqual(canary.read_text(), "keep")

    def test_symlinked_root_and_activation_are_refused(self):
        outside = self.base / "outside"
        outside.mkdir()
        self.root.symlink_to(outside)
        path, checksum = self.archive()
        with self.assertRaisesRegex(ValueError, "symlink"):
            installer.install(self.root, path, "0.1.0", checksum)
        self.root.unlink()
        self.install()
        (self.root / "current").unlink()
        (self.root / "current").symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "pointer"):
            installer.uninstall(self.root)
        self.assertTrue(outside.exists())

    def test_concurrent_operation_is_refused_without_mutation(self):
        fd = os.open(self.base, os.O_RDONLY | os.O_DIRECTORY)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            path, checksum = self.archive()
            with self.assertRaisesRegex(ValueError, "another installation"):
                installer.install(self.root, path, "0.1.0", checksum)
            self.assertFalse(self.root.exists())
        finally:
            os.close(fd)

    def test_version_overwrite_and_invalid_version_refused(self):
        self.install()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.install()
        path, checksum = self.archive()
        with self.assertRaisesRegex(ValueError, "version"):
            installer.install(self.root, path, "../escape", checksum)

    def test_compressed_metadata_bomb_is_bounded_before_tar_parser(self):
        path = self.base / "bomb.tar.gz"
        path.write_bytes(gzip.compress(b"x" * 32768))
        with patch.object(installer, "LIMIT", 16384), patch.object(installer.tarfile, "open") as parser:
            with self.assertRaisesRegex(ValueError, "expanded TAR"):
                installer.install(self.root, path, "0.1.0", installer.digest(path.read_bytes()))
            parser.assert_not_called()
        self.assertFalse(self.root.exists())

    def response(self, content):
        response = io.BytesIO(content)
        response.status = 200
        response.headers = {'Content-Length': str(len(content))}
        response.geturl = lambda: 'https://release-assets.githubusercontent.com/synthetic-asset'
        return response

    def test_version_download_is_pinned_and_checked_before_install(self):
        path, checksum = self.archive()
        with patch.object(installer, '_fetch_bounded', side_effect=installer._download), patch.object(installer.urllib.request, 'build_opener') as build:
            build.return_value.open.return_value = self.response(path.read_bytes())
            installer.install_version(self.root, '0.1.0', checksum)
            request = build.return_value.open.call_args.args[0]
            self.assertEqual(request.full_url, installer.RELEASE_BASE + '/v0.1.0/' + installer.ASSET)
            self.assertFalse(request.has_header('Authorization'))
            self.assertEqual(installer.current_version(self.root), '0.1.0')

    def test_download_checksum_failure_preserves_existing_install(self):
        self.install()
        path, _ = self.archive()
        with patch.object(installer, '_fetch_bounded', side_effect=installer._download), patch.object(installer.urllib.request, 'build_opener') as build:
            build.return_value.open.return_value = self.response(path.read_bytes())
            with self.assertRaisesRegex(ValueError, 'checksum'):
                installer.install_version(self.root, '0.2.0', '0' * 64)
        self.assertEqual(installer.current_version(self.root), '0.1.0')
        self.assertFalse((self.root / 'versions/0.2.0').exists())

    def test_invalid_version_or_missing_hash_never_opens_network(self):
        with patch.object(installer.urllib.request, 'build_opener') as build:
            for version, checksum in [('latest', '0' * 64), ('0.1.0', '')]:
                with self.assertRaises(ValueError):
                    installer.install_version(self.root, version, checksum)
            build.assert_not_called()

    def test_unavailable_or_oversized_download_has_no_install_side_effect(self):
        with patch.object(installer, '_fetch_bounded', side_effect=installer._download), patch.object(installer.urllib.request, 'build_opener') as build:
            build.return_value.open.side_effect = installer.urllib.error.URLError('SECRET-CANARY')
            with self.assertRaisesRegex(ValueError, 'unavailable') as caught:
                installer.install_version(self.root, '0.1.0', '0' * 64)
            self.assertNotIn('CANARY', str(caught.exception))
            build.return_value.open.side_effect = None
            build.return_value.open.return_value = self.response(b'x' * 20)
            with patch.object(installer, 'LIMIT', 10), self.assertRaisesRegex(ValueError, 'size bound'):
                installer.install_version(self.root, '0.1.0', '0' * 64)
        self.assertFalse(self.root.exists())

    def test_redirects_reject_other_hosts_http_credentials_and_ports(self):
        redirect = installer.ReleaseRedirects()
        request = installer.urllib.request.Request(installer.RELEASE_BASE)
        for url in ['http://github.com/asset', 'https://example.invalid/a',
                    'https://user:secret@github.com/a', 'https://github.com:444/a']:
            with self.assertRaisesRegex(ValueError, 'approved HTTPS'):
                redirect.redirect_request(request, None, 302, 'Found', {}, url)

    def test_overall_deadline_terminates_a_blocking_read(self):
        response = self.response(b'')
        response.headers = {}
        response.read = lambda *_: time.sleep(10)
        started = time.monotonic()
        with patch.object(installer, 'DOWNLOAD_TIMEOUT', .1), patch.object(installer.urllib.request, 'build_opener') as build:
            build.return_value.open.return_value = response
            with self.assertRaisesRegex(ValueError, 'time bound'):
                installer.install_version(self.root, '0.1.0', '0' * 64)
        self.assertLess(time.monotonic() - started, 2)
        self.assertFalse(self.root.exists())

    def test_bounded_worker_success_installs_verified_bytes(self):
        path, checksum = self.archive()
        with patch.object(installer.urllib.request, 'build_opener') as build:
            build.return_value.open.return_value = self.response(path.read_bytes())
            installer.install_version(self.root, '0.1.0', checksum)
        self.assertEqual(installer.current_version(self.root), '0.1.0')


if __name__ == "__main__":
    unittest.main()
