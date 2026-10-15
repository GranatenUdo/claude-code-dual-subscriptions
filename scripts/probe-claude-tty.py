"""Run only in the documented bwrap namespace, with /client, /launcher, /probe.py.

No prompt is submitted. This checks real-client terminal startup and wrapper
termination, not a live response. Synthetic auth setup comes from the mock probe.
"""
import errno
import json
import os
import pty
import runpy
import select
import signal
import sys
import time
import re
import fcntl
import struct
import subprocess

action = sys.argv[1] if len(sys.argv) > 1 else 'terminate'
if action == 'all':
    for case in ('direct-decline', 'decline', 'terminate'):
        subprocess.run([sys.executable, __file__, case], check=True)
    raise SystemExit(0)
assert action in ('terminate', 'decline', 'direct-decline')
sys.argv = ['/probe.py', 'auth']
runpy.run_path('/probe.py')
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, __import__('termios').TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
    os.chdir('/home/test')
    program = '/client' if action == 'direct-decline' else '/launcher'
    args = [program, '--model', 'claude-opus-5-5'] if action == 'direct-decline' else [program, 'launch', '--claude', '/client']
    os.execve(program, args,
        {'HOME': '/home/test', 'PATH': '/usr/bin:/bin', 'TERM': 'xterm-256color',
         'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1', 'DISABLE_AUTOUPDATER': '1'})
output = b''
status = None
sent = False
started = time.monotonic()
trust_prompt = False
trust_seen_at = None
try:
    while time.monotonic() - started < 18:
        if select.select([fd], [], [], .1)[0]:
            try:
                output += os.read(fd, 8192)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
        screen = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', output.decode(errors='replace'))
        compact = re.sub(r'\s+', '', screen)
        trust_prompt = 'Accessingworkspace:' in compact and 'No,exit' in compact and 'Yes,Itrustthisfolder' in compact
        if trust_prompt and trust_seen_at is None:
            trust_seen_at = time.monotonic()
        if not sent and trust_prompt and time.monotonic() - trust_seen_at > 1:
            if action.endswith('decline'):
                os.write(fd, b'\r')  # Default is "No, exit"; never accept trust.
            else:
                os.kill(pid, signal.SIGTERM)
            sent = True
        done, value = os.waitpid(pid, os.WNOHANG)
        if done:
            status = value
            break
    evidence = {'wrapper_exit': os.waitstatus_to_exitcode(status) if status is not None else None,
        'action': action, 'action_sent': sent, 'launcher_banner': b'claude-code-dual:' in output,
        'client_ui_present': 'ClaudeCode' in compact, 'normal_trust_prompt_present': trust_prompt,
        'output_contains_synthetic_oauth': b'sk-ant-oat01-SYNTHETIC-OFFLINE-ONLY' in output,
        'output_contains_capability_header': b'x-claude-dual-capability:' in output}
    print(json.dumps(evidence))
    if not evidence['client_ui_present'] or status is None:
        screen = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', output.decode(errors='replace'))
        screen = re.sub(r'\x1b\].*?(?:\x07|\x1b\\)', '', screen)
        screen = re.sub(r'[a-f0-9]{64}|sk-ant-\S+', '[redacted]', screen)
        if not evidence['output_contains_synthetic_oauth'] and not evidence['output_contains_capability_header']:
            print(json.dumps({'terminal_size': list(os.get_terminal_size(fd)), 'synthetic_screen_excerpt': screen[-3000:]}))
    assert evidence['wrapper_exit'] == (1 if action.endswith('decline') else 143) and evidence['client_ui_present'] and trust_prompt
    assert not evidence['output_contains_synthetic_oauth'] and not evidence['output_contains_capability_header']
finally:
    if status is None:
        os.kill(pid, signal.SIGTERM)
        os.waitpid(pid, 0)
    os.close(fd)
