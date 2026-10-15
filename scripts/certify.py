#!/usr/bin/env python3
"""Run the actual-client certification matrix for `claude-code-dual launch`.

Host-side driver. It builds the test-only harness (tests/dual_certification.rs)
with `cargo test --no-run`, then runs every scenario of
scripts/probe-claude-launch.py in its own bwrap namespace: no host network, no
real home or credentials, synthetic accounts, and the pinned client bound
read-only at /client. Evidence is written as JSON next to the head commit SHA.

Usage: scripts/certify.py --client /absolute/pinned/claude [--out DIR]
                          [scenario ...]
Honours CARGO_TARGET_DIR; never runs the production binary against real upstreams.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = ['conversation', 'terminal', 'suspend', 'descendants', 'errors', 'refusals', 'managed']


def harness_executable():
    build = subprocess.run(
        ['cargo', 'test', '--locked', '--test', 'dual_certification', '--no-run',
         '--message-format=json'],
        cwd=ROOT, capture_output=True, text=True, check=True)
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if (message.get('reason') == 'compiler-artifact'
                and message.get('target', {}).get('name') == 'dual_certification'
                and message.get('executable')):
            return Path(message['executable'])
    raise SystemExit('dual_certification executable not found in cargo output')


def run_scenario(name, client, launcher, timeout):
    command = ['bwrap', '--unshare-all', '--die-with-parent', '--new-session',
               '--ro-bind', '/usr', '/usr', '--symlink', 'usr/bin', '/bin',
               '--ro-bind', '/lib', '/lib', '--ro-bind', '/lib64', '/lib64',
               '--ro-bind', str(client), '/client', '--ro-bind', str(launcher), '/launcher',
               '--ro-bind', str(ROOT / 'scripts/probe-claude-launch.py'), '/certify_probe.py',
               '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp', '--dir', '/home/test',
               '--chdir', '/home/test', '--clearenv', '--setenv', 'HOME', '/home/test',
               '--setenv', 'PATH', '/usr/bin:/bin', '--setenv', 'DISABLE_AUTOUPDATER', '1']
    managed = None
    if name == 'managed':
        managed = tempfile.NamedTemporaryFile('w', suffix='.json', delete=False)
        managed.write(json.dumps({'permissions': {'deny': ['Bash']}}))
        managed.close()
        command += ['--dir', '/etc/claude-code', '--ro-bind', managed.name,
                    '/etc/claude-code/managed-settings.json']
    command += ['/usr/bin/python3', '/certify_probe.py', name]
    started = time.monotonic()
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
        lines = [line for line in result.stdout.splitlines() if line.startswith('{')]
        evidence = json.loads(lines[-1]) if lines else {'scenario': name, 'passed': False,
                                                         'stderr_tail': result.stderr[-2000:]}
        evidence['probe_exit'] = result.returncode
    except subprocess.TimeoutExpired:
        evidence = {'scenario': name, 'passed': False, 'timeout_seconds': timeout}
    finally:
        if managed:
            os.unlink(managed.name)
    evidence['seconds'] = round(time.monotonic() - started, 1)
    return evidence


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--client', required=True, type=Path)
    parser.add_argument('--out', type=Path, default=Path(
        os.environ.get('CARGO_TARGET_DIR') or ROOT / 'target') / 'certification')
    parser.add_argument('--timeout', type=int, default=900)
    parser.add_argument('scenarios', nargs='*', default=SCENARIOS)
    args = parser.parse_args()
    client = args.client.resolve()
    assert client.is_file(), 'pinned client not found'
    head = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout.strip()
    dirty = bool(subprocess.run(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=ROOT,
                                capture_output=True, text=True, check=True).stdout.strip())
    launcher = harness_executable()
    report = {
        'head': head,
        'worktree_dirty': dirty,
        'client_sha256': hashlib.sha256(client.read_bytes()).hexdigest(),
        'client_version': subprocess.run([str(client), '--version'], capture_output=True, text=True,
                                         env={'DISABLE_AUTOUPDATER': '1', 'HOME': '/nonexistent',
                                              'PATH': '/usr/bin:/bin'}).stdout.strip(),
        'scenarios': [],
    }
    for name in args.scenarios:
        evidence = run_scenario(name, client, launcher, args.timeout)
        report['scenarios'].append(evidence)
        print(f"{name}: {'PASS' if evidence.get('passed') else 'FAIL'} ({evidence['seconds']}s)",
              flush=True)
    report['passed'] = all(item.get('passed') for item in report['scenarios'])
    args.out.mkdir(parents=True, exist_ok=True)
    path = args.out / f'certification-{head[:12]}.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(f'report: {path}')
    return 0 if report['passed'] and not dirty else 1


if __name__ == '__main__':
    sys.exit(main())
