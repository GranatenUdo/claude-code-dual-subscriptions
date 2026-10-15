#!/usr/bin/env python3
"""Version-pinned artifact staging. No release channel exists yet.

Requires Linux x86_64 GNU and Python 3.10+. No sudo, shell/profile edits,
credential access, executable invocation, or automatic update. A supplied hash
checks bytes, not publisher identity; obtain artifacts and hashes independently.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import tarfile
import tempfile
import functools
import fcntl
import gzip
import multiprocessing
import urllib.request
import urllib.parse
import urllib.error

OWNER = {"product": "claude-code-dual", "schema": 1}
NAMES = ("claude-code-dual", "claude-code-proxy", "LICENSE")
LIMIT = 256 * 1024 * 1024
RELEASE_BASE = 'https://github.com/GranatenUdo/claude-code-dual-subscriptions/releases/download'
ASSET = 'claude-code-dual-linux-x86_64-gnu.tar.gz'
DOWNLOAD_TIMEOUT = 60


def allowed_download_url(url):
    parsed = urllib.parse.urlsplit(url)
    return (parsed.scheme == 'https' and parsed.hostname in {
        'github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com'
    } and parsed.port in (None, 443) and not parsed.username and not parsed.password and not parsed.fragment)


class ReleaseRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        require(allowed_download_url(newurl), 'release redirect outside approved HTTPS hosts')
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def install_version(root, version, expected):
    """Download only an exact release asset, requiring an independently trusted hash.

    No credentials, ambient proxies, latest-version lookup or downloaded scripts.
    Network failure/checksum mismatch cannot mutate the installed root.
    """
    version_id(version)
    require(bool(re.fullmatch(r'[a-f0-9]{64}', expected)), 'exact SHA-256 required before download')
    no_links(root)
    require(root.parent.is_dir(), 'root parent must already exist')
    url = f'{RELEASE_BASE}/v{version}/{ASSET}'
    with tempfile.TemporaryDirectory(prefix='dual-download-') as folder:
        archive = Path(folder) / ASSET
        _fetch_bounded(url, archive)
        install(root, archive, version, expected)


def _download(url, archive):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), ReleaseRedirects())
    try:
        with opener.open(urllib.request.Request(url, headers={'User-Agent': 'claude-code-dual-installer'}), timeout=15) as response:
            require(response.status == 200 and allowed_download_url(response.geturl()), 'release download refused')
            size = response.headers.get('Content-Length')
            require(size is None or (size.isdecimal() and int(size) <= LIMIT), 'download exceeds size bound')
            total = 0
            with archive.open('xb') as output:
                while True:
                    chunk = response.read(min(64 * 1024, LIMIT + 1 - total))
                    if not chunk:
                        break
                    total += len(chunk)
                    require(total <= LIMIT, 'download exceeds size bound')
                    output.write(chunk)
    except (urllib.error.URLError, TimeoutError, OSError):
        raise ValueError('release download unavailable; installation unchanged') from None


def _download_worker(url, archive, result):
    try:
        _download(url, archive)
        result.send(True)
    except Exception:
        # Network exceptions can contain signed URLs. Never relay their text.
        result.send(False)
    finally:
        result.close()


def _fetch_bounded(url, archive):
    # Linux-only CLI, before any threads. A process bounds DNS, redirects and
    # slow-drip reads, which socket inactivity timeouts cannot bound overall.
    context = multiprocessing.get_context('fork')
    receiver, sender = context.Pipe(duplex=False)
    worker = context.Process(target=_download_worker, args=(url, archive, sender))
    try:
        worker.start()
        sender.close()
        worker.join(DOWNLOAD_TIMEOUT)
        require(not worker.is_alive(), 'download time bound exceeded; installation unchanged')
        require(worker.exitcode == 0 and receiver.poll() and receiver.recv(), 'release download unavailable or rejected; installation unchanged')
    finally:
        if worker.pid is not None and worker.is_alive():
            worker.terminate()
            worker.join(1)
            if worker.is_alive():
                worker.kill()
                worker.join(1)
        receiver.close()
        sender.close()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def no_links(path):
    require(path.is_absolute(), "root must be absolute")
    for item in (path, *path.parents):
        require(not item.is_symlink(), "symlinked path refused")


def serialized(operation):
    """Lock the existing parent directory; create no persistent lock file."""
    @functools.wraps(operation)
    def wrapped(root, *args):
        no_links(root)
        fd = os.open(root.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise ValueError("another installation operation owns the parent-directory lock") from None
            return operation(root, *args)
        finally:
            os.close(fd)
    return wrapped


def version_id(value):
    require(bool(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[a-z0-9.-]+)?", value)), "invalid exact version")
    return value


def digest(data):
    return hashlib.sha256(data).hexdigest()


def regular_read(path, limit):
    no_links(path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        import stat
        require(stat.S_ISREG(os.fstat(stream.fileno()).st_mode), "regular file required")
        data = stream.read(limit + 1)
    require(len(data) <= limit, "file exceeds size bound")
    return data


def owned_root(root):
    no_links(root)
    require(root.is_dir(), "owned root absent")
    marker = json.loads(regular_read(root / ".dual-owner.json", 1024))
    require(marker == OWNER, "root ownership mismatch")
    require(set(p.name for p in root.iterdir()) <= {".dual-owner.json", "versions", "current"}, "unexpected root files; refusing mutation (preserve and inspect interrupted-operation files manually)")
    no_links(root / "versions")
    require((root / "versions").is_dir(), "versions directory absent")


def inspect_version(root, version):
    version_id(version)
    directory = root / "versions" / version
    no_links(directory)
    manifest = json.loads(regular_read(directory / "manifest.json", 8192))
    require(manifest.get("owner") == OWNER and manifest.get("version") == version, "manifest ownership mismatch")
    require(set(manifest.get("files", {})) == set(NAMES), "manifest file list mismatch")
    require(set(p.name for p in directory.iterdir()) == set(NAMES) | {"manifest.json"}, "unexpected version files")
    for name in NAMES:
        require(digest(regular_read(directory / name, LIMIT)) == manifest["files"][name], "owned file changed; refusing mutation")
    return directory


def current_version(root):
    pointer = root / "current"
    if not pointer.exists() and not pointer.is_symlink():
        return None
    require(pointer.is_symlink(), "current is not an owned activation pointer")
    target = os.readlink(pointer)
    require(target.startswith("versions/") and target.count("/") == 1, "invalid activation pointer")
    version = version_id(target.removeprefix("versions/"))
    inspect_version(root, version)
    return version


def _activate(root, version):
    owned_root(root)
    inspect_version(root, version)
    current_version(root)
    # Unique path, and replace only after the existing pointer was validated.
    fd, temporary = tempfile.mkstemp(prefix=".activation-", dir=root)
    os.close(fd)
    os.unlink(temporary)
    try:
        os.symlink("versions/" + version, temporary)
        os.replace(temporary, root / "current")
    finally:
        if os.path.lexists(temporary):
            os.unlink(temporary)


@serialized
def activate(root, version):
    _activate(root, version)


@serialized
def install(root, archive, version, expected):
    version_id(version)
    require(bool(re.fullmatch(r"[a-f0-9]{64}", expected)), "SHA-256 must be an exact lowercase hex digest")
    # Validate every byte and archive member before changing installation state.
    data = regular_read(archive, LIMIT)
    require(digest(data) == expected, "artifact checksum mismatch")
    import io
    files = {}
    with gzip.GzipFile(fileobj=io.BytesIO(data)) as compressed:
        expanded = compressed.read(LIMIT + 1)
    require(len(expanded) <= LIMIT, "total expanded TAR exceeds limit")
    with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as tar:
        for member in tar:
            require(member.name in NAMES and member.name not in files, "unexpected or duplicate archive member")
            require(member.isfile() and not member.issym() and not member.islnk(), "archive links and special files refused")
            require(0 < member.size <= LIMIT, "invalid archive member size")
            require(sum(len(v) for v in files.values()) + member.size <= LIMIT, "expanded archive exceeds limit")
            stream = tar.extractfile(member)
            require(stream is not None, "archive member unavailable")
            files[member.name] = stream.read(member.size + 1)
            require(len(files[member.name]) == member.size, "truncated archive")
    require(set(files) == set(NAMES), "artifact must contain both binaries and LICENSE")
    # Linux x86_64 ELF identity check; no executable is run during installation.
    for name in NAMES[:2]:
        content = files[name]
        require(len(content) >= 20 and content[:6] == b"\x7fELF\x02\x01" and content[18:20] == b"\x3e\x00", "artifact is not Linux x86_64 ELF")
    no_links(root)
    require(root.parent.is_dir(), "root parent must already exist")
    if root.exists():
        owned_root(root)
        current_version(root)
    else:
        root.mkdir(mode=0o700)
        (root / ".dual-owner.json").write_text(json.dumps(OWNER) + "\n")
        (root / "versions").mkdir(mode=0o700)
    destination = root / "versions" / version
    require(not destination.exists() and not destination.is_symlink(), "version already exists; refusing overwrite")
    # Private staging shares the target filesystem. No extractall/path interpretation.
    with tempfile.TemporaryDirectory(prefix=".stage-", dir=root / "versions") as temp:
        stage = Path(temp)
        for name, content in files.items():
            (stage / name).write_bytes(content)
            (stage / name).chmod(0o600 if name == "LICENSE" else 0o700)
        manifest = {"owner": OWNER, "version": version, "archive_sha256": expected,
                    "files": {name: digest(content) for name, content in files.items()}}
        (stage / "manifest.json").write_text(json.dumps(manifest, sort_keys=True) + "\n")
        # Rename completed directory; TemporaryDirectory cleans only its old empty path.
        os.rename(stage, destination)
    _activate(root, version)


@serialized
def uninstall(root):
    owned_root(root)
    current_version(root)
    versions = list((root / "versions").iterdir())
    for directory in versions:
        inspect_version(root, directory.name)
    # All ownership/content checks complete before any removals. No recursive delete.
    if (root / "current").is_symlink():
        (root / "current").unlink()
    for directory in versions:
        for name in (*NAMES, "manifest.json"):
            (directory / name).unlink()
        directory.rmdir()
    (root / "versions").rmdir()
    (root / ".dual-owner.json").unlink()
    root.rmdir()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True, help="dedicated absolute per-user directory; never an existing package prefix")
    commands = parser.add_subparsers(dest="command", required=True)
    add = commands.add_parser("install-local", help="stage developer artifact; does not establish publisher trust")
    add.add_argument("--archive", type=Path, required=True)
    add.add_argument("--version", required=True)
    add.add_argument("--sha256", required=True)
    download = commands.add_parser('install-version', help='download exact release asset; requires independently trusted SHA-256; no releases exist yet')
    download.add_argument('--version', required=True)
    download.add_argument('--sha256', required=True)
    rollback = commands.add_parser("activate", help="manual upgrade/rollback to an already verified local version")
    rollback.add_argument("--version", required=True)
    commands.add_parser("uninstall", help="remove only unchanged manifest-owned files; preserve credentials elsewhere")
    args = parser.parse_args()
    require(platform.system() == "Linux" and platform.machine() == "x86_64", "only Linux/WSL2 x86_64 is in scope")
    require(os.geteuid() != 0, "do not run with sudo/root")
    require(platform.libc_ver()[0] == "glibc", "GNU libc required")
    require(tuple(map(int, platform.libc_ver()[1].split(".")[:2])) >= (2, 39), "glibc >=2.39 required for planned Ubuntu 24.04 artifacts")
    release = platform.freedesktop_os_release()
    require(release.get("ID") == "ubuntu" and release.get("VERSION_ID") in {"24.04", "26.04"}, "only Ubuntu 24.04/26.04 candidates are in scope")
    if args.command == "install-local":
        install(args.root, args.archive, args.version, args.sha256)
    elif args.command == 'install-version':
        install_version(args.root, args.version, args.sha256)
    elif args.command == "activate":
        activate(args.root, args.version)
    else:
        uninstall(args.root)
    print("Completed owned-file operation. No shell settings or credentials changed.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, tarfile.TarError) as error:
        # Do not print user-provided archive fields or secret-bearing file content.
        raise SystemExit("Operation refused: " + (str(error) if isinstance(error, ValueError) else "filesystem/archive error"))
