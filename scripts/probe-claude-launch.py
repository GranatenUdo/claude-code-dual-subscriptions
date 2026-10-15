"""Actual-client certification scenarios for `claude-code-dual launch`.

Run only inside the bwrap namespace prepared by scripts/certify.py, with the
pinned client at /client and the test-only harness (tests/dual_certification.rs)
at /launcher. Credentials are synthetic and generated in this ephemeral home; the
mock upstreams live inside the harness. Evidence is allowlisted JSON: no request
bodies, credentials or capability values are printed.
"""
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
from pathlib import Path

HOME = Path('/home/test')
PROJECT = HOME / 'project'
RECORDS = Path('/tmp/records.jsonl')
TOKEN = 'sk-ant-oat01-SYNTHETIC-OFFLINE-ONLY'
BASE_ENV = {'HOME': str(HOME), 'PATH': '/usr/bin:/bin', 'TERM': 'xterm-256color',
            'DISABLE_AUTOUPDATER': '1'}
LAUNCH = ['/launcher', 'launch', '--claude', '/client', '--mock-record', str(RECORDS)]
ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\].*?(?:\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[78=>]')
DOWN, CTRL_C, CTRL_Z = b'\x1b[B', b'\x03', b'\x1a'


def write_credentials():
    config = HOME / '.claude'
    config.mkdir(exist_ok=True)
    (config / '.credentials.json').write_text(json.dumps({'claudeAiOauth': {
        'accessToken': TOKEN, 'refreshToken': 'SYNTHETIC-NEVER-REFRESH',
        'expiresAt': int((time.time() + 86400) * 1000),
        'scopes': ['user:inference', 'user:profile'], 'subscriptionType': 'max',
        'rateLimitTier': 'default_claude_max_20x'}}))
    (HOME / '.claude.json').write_text(json.dumps({'hasCompletedOnboarding': True,
        'oauthAccount': {'accountUuid': '00000000-0000-4000-8000-000000000001',
            'emailAddress': 'synthetic@example.invalid',
            'organizationUuid': '00000000-0000-4000-8000-000000000002'}}))


MCP_SERVER = r'''import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if 'id' not in message:
        continue
    method = message.get('method')
    if method == 'initialize':
        result = {'protocolVersion': message['params'].get('protocolVersion', '2025-06-18'),
                  'capabilities': {'tools': {}}, 'serverInfo': {'name': 'cert', 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'echo', 'description': 'Synthetic echo',
                  'inputSchema': {'type': 'object', 'properties': {'text': {'type': 'string'}}}}]}
    elif method == 'tools/call':
        result = {'content': [{'type': 'text', 'text': 'CERT-MCP-PONG'}]}
    else:
        result = {}
    print(json.dumps({'jsonrpc': '2.0', 'id': message['id'], 'result': result}), flush=True)
'''


def write_project(allow_bash=False):
    (PROJECT / '.claude').mkdir(parents=True, exist_ok=True)
    (PROJECT / 'fixture.txt').write_text('CERT-FIXTURE-CONTENT\n')
    (HOME / 'mcp_server.py').write_text(MCP_SERVER)
    (PROJECT / '.mcp.json').write_text(json.dumps({'mcpServers': {'cert': {
        'command': '/usr/bin/python3', 'args': [str(HOME / 'mcp_server.py')]}}}))
    allow = ['Read', 'mcp__cert__echo', 'Agent'] + (['Bash'] if allow_bash else [])
    (PROJECT / '.claude/settings.json').write_text(json.dumps({
        'permissions': {'allow': allow, 'deny': [] if allow_bash else ['Bash'],
                        'defaultMode': 'default'},
        'enableAllProjectMcpServers': True,
        'hooks': {'UserPromptSubmit': [{'hooks': [{'type': 'command',
            'command': '/usr/bin/touch /home/test/hook-ran'}]}]}}))


def records(path=RECORDS):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def compact(text):
    return re.sub(r'\s+', '', text)


def excerpt(text, limit=1500):
    """Synthetic screen text for failed checks, with anything secret-shaped removed."""
    text = re.sub(r'[a-f0-9]{64}|sk-ant-\S+|x-claude-dual-capability:\S*', '[redacted]', text)
    return re.sub(r'[ \t\r]+', ' ', text)[-limit:]


class Session:
    """The launcher or a direct client on a fresh 40x120 pseudo-terminal."""

    def __init__(self, argv, env=None, cwd=PROJECT, via_shell=False):
        self.output = b''
        self.mark_at = 0
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
            os.chdir(cwd)
            Path('/tmp/termios-before').write_text(json.dumps(termios.tcgetattr(0)[:4]))
            if via_shell:
                os.execve('/usr/bin/bash', ['bash', '--norc', '--noprofile', '-i'],
                          dict(BASE_ENV, **(env or {}), PS1='PROMPT> ', TERM='xterm-256color'))
            os.execve(argv[0], argv, dict(BASE_ENV, **(env or {})))
        self.status = None

    def pump(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline and self.status is None:
            if select.select([self.fd], [], [], .05)[0]:
                try:
                    self.output += os.read(self.fd, 65536)
                except OSError:
                    pass
            self.poll()

    def poll(self):
        if self.status is None:
            done, value = os.waitpid(self.pid, os.WNOHANG)
            if done:
                self.status = os.waitstatus_to_exitcode(value)
        return self.status

    def screen(self, since_mark=True):
        data = self.output[self.mark_at:] if since_mark else self.output
        return ANSI.sub('', data.decode(errors='replace'))

    def mark(self):
        self.mark_at = len(self.output)

    def wait_for(self, *needles, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            seen = compact(self.screen())
            for needle in needles:
                if compact(needle) in seen:
                    return needle
            if self.status is not None:
                return None
            self.pump(.2)
        return None

    def send(self, data, settle=.4):
        os.write(self.fd, data)
        self.pump(settle)

    def submit(self, text):
        self.mark()
        self.send(text.encode(), settle=.8)
        self.send(b'\r', settle=.2)

    def accept_trust(self):
        """Accepts the trust prompt for the fixture project, or returns once the
        UI is ready when this namespace already trusted it."""
        seen = self.wait_for('Yes,Itrustthisfolder', 'auto mode', 'for agents', timeout=30)
        if seen is None:
            return False
        if seen != 'Yes,Itrustthisfolder':
            self.pump(1)
            return True
        self.pump(1)
        self.send(DOWN, settle=.6)
        self.mark()
        self.send(b'\r', settle=.5)
        return self.wait_for('auto mode', 'for agents', '/effort', timeout=30) is not None

    def wait_exit(self, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and self.poll() is None:
            self.pump(.2)
        return self.status

    def exit_command(self):
        self.mark()
        self.send(b'/exit', settle=1)
        self.send(b'\r', settle=1)
        if self.poll() is None:
            self.send(b'\r', settle=1)
        return self.wait_exit(30)

    def termios_restored(self):
        before = json.loads(Path('/tmp/termios-before').read_text())
        return termios.tcgetattr(self.fd)[:4] == before

    def close(self):
        if self.poll() is None:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        os.close(self.fd)

    def leaked(self):
        return TOKEN.encode() in self.output or b'x-claude-dual-capability:' in self.output


def proc_state(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError, IndexError):
        return None


def proc_groups(pid):
    """(process group, session) of a live process."""
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[2]), int(fields[3])
    except (FileNotFoundError, ProcessLookupError, IndexError, ValueError):
        return None


def group_alive(pgid):
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            groups = proc_groups(int(entry.name))
            if groups and groups[0] == pgid and proc_state(int(entry.name)) not in (None, 'Z'):
                return True
    return False


def wait_quiet(quiet=15, limit=420, path=RECORDS):
    """Waits until the mock has seen no new request for `quiet` seconds."""
    started = time.monotonic()
    last_count, last_change = len(records(path)), time.monotonic()
    while time.monotonic() - started < limit:
        time.sleep(1)
        count = len(records(path))
        if count != last_count:
            last_count, last_change = count, time.monotonic()
        elif time.monotonic() - last_change >= quiet:
            return round(last_change - started, 1)
    return None


def process_pid(name):
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                if (entry / 'cmdline').read_bytes().split(b'\0')[0] == name.encode():
                    return int(entry.name)
            except OSError:
                continue
    return None


def common_checks(evidence):
    anthropic = [r for r in records() if r.get('event') == 'anthropic' and r.get('route') == '/v1/messages']
    codex = [r for r in records() if r.get('event') == 'codex']
    evidence['anthropic_requests'] = len(anthropic)
    evidence['codex_requests'] = len(codex)
    evidence['saved_oauth_and_beta_on_every_request'] = bool(anthropic) and all(
        r['saved_oauth_preserved'] and r['oauth_beta_present'] and r['api_key_absent'] for r in anthropic)
    evidence['capability_never_reached_upstream'] = all(r['capability_absent'] for r in anthropic)
    evidence['no_nonstreaming_fallback'] = all(r['stream'] for r in anthropic)
    evidence['models_seen'] = sorted({str(r.get('model')) for r in anthropic + codex})
    return anthropic, codex


def scenario_conversation():
    write_credentials()
    write_project()
    evidence = {'scenario': 'conversation'}
    session = Session(LAUNCH)
    try:
        evidence['trust_accepted_for_fixture_project'] = session.accept_trust()
        steps = [('CERT-TEXT-1', 'CERT-REPLY-1'), ('CERT-TEXT-2', 'CERT-REPLY-2'),
                 ('CERT-READ', 'CERT-READ-DONE-OK'), ('CERT-BASH', 'CERT-BASH-DONE-ERR'),
                 ('CERT-MCP', 'CERT-MCP-DONE-OK'), ('CERT-AGENT', 'CERT-AGENT-RESULT-OK'),
                 ('CERT-REVIEW', 'CERT-REVIEW-RESULT-OK')]
        for prompt, expected in steps:
            session.submit(prompt)
            evidence[f'turn_{prompt}'] = session.wait_for(expected, timeout=60) is not None
            if not evidence[f'turn_{prompt}']:
                evidence.setdefault('failed_screens', {})[prompt] = excerpt(session.screen())
            session.pump(1)
        evidence['exit_code'] = session.exit_command()
        evidence['terminal_restored'] = session.termios_restored()
        evidence['secret_in_terminal_output'] = session.leaked()
    finally:
        session.close()
    evidence['safety_hook_ran'] = (HOME / 'hook-ran').exists()
    evidence['denied_tool_ran'] = (HOME / 'forbidden-tool-ran').exists()
    anthropic, codex = common_checks(evidence)
    evidence['astra_worker_via_codex'] = any(r.get('model') == 'gpt-6-astra' and r['synthetic_codex_auth'] for r in codex)
    evidence['opus_reviewer_subagent'] = any(r.get('marker') == 'CERT-SUBAGENT' and r.get('model') == 'claude-opus-5-5'
                                             for r in anthropic)
    evidence['mcp_tool_offered'] = any(r['mcp_tool_present'] for r in anthropic)
    evidence['denied_bash_withheld_or_refused'] = not evidence['denied_tool_ran']
    sessions = sorted((HOME / '.claude/projects').glob('*/*.jsonl'), key=lambda p: p.stat().st_mtime)
    evidence['session_persisted'] = bool(sessions)
    if sessions:
        resume_id = sessions[-1].stem
        RECORDS.write_text('')
        resumed = Session(LAUNCH + ['--resume', resume_id])
        try:
            ready = resumed.wait_for('CERT-REPLY-2', 'for agents', timeout=40)
            resumed.pump(2)
            resumed.submit('CERT-TEXT-1')
            evidence['resume_turn'] = resumed.wait_for('CERT-REPLY-1', timeout=60) is not None
            evidence['resume_ui_ready'] = ready is not None
            evidence['resume_exit_code'] = resumed.exit_command()
        finally:
            resumed.close()
        evidence['resume_history_sent'] = any(r.get('history_has_reply') for r in records()
                                              if r.get('event') == 'anthropic')
    required = ['trust_accepted_for_fixture_project', 'safety_hook_ran', 'astra_worker_via_codex',
                'opus_reviewer_subagent', 'saved_oauth_and_beta_on_every_request',
                'capability_never_reached_upstream', 'no_nonstreaming_fallback', 'terminal_restored',
                'resume_turn', 'resume_history_sent'] + [f'turn_{p}' for p, _ in steps]
    evidence['passed'] = (all(evidence.get(key) for key in required) and evidence['exit_code'] == 0
                          and not evidence['denied_tool_ran'] and not evidence['secret_in_terminal_output']
                          and evidence.get('resume_exit_code') == 0)
    return evidence


def direct_env():
    """Environment for a directly started client against the harness mock."""
    server = subprocess.Popen(['/launcher', 'mock-serve', '--mock-record', '/tmp/direct.jsonl'],
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    url = server.stdout.readline().strip().split('=', 1)[1]
    return server, {'ANTHROPIC_BASE_URL': url, 'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1',
                    'CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK': '1'}


def terminal_case(argv, env, action):
    session = Session(argv, env=env)
    try:
        if not session.accept_trust():
            return {'ready': False}
        session.pump(1)
        result = {'ready': True}
        if action == 'exit':
            result['exit_code'] = session.exit_command()
        elif action == 'sigterm':
            os.kill(session.pid, signal.SIGTERM)
            result['exit_code'] = session.wait_exit(20)
        elif action == 'interrupt':
            RECORDS.write_text('') if argv[0] == '/launcher' else None
            session.submit('CERT-SLOW')
            result['streaming'] = session.wait_for('CERT-SLOW-STREAM', timeout=30) is not None
            session.mark()
            session.send(CTRL_C, settle=2)
            result['interrupted_shown'] = session.wait_for('Interrupted', timeout=15) is not None
            session.submit('CERT-TEXT-1')
            result['next_turn_after_interrupt'] = session.wait_for('CERT-REPLY-1', timeout=60) is not None
            session.send(CTRL_C, settle=.5)
            session.send(CTRL_C, settle=.5)
            result['exit_code'] = session.wait_exit(20)
        result['terminal_restored'] = session.termios_restored() if session.status is not None else False
        result['secret_in_terminal_output'] = session.leaked()
        return result
    finally:
        session.close()


def scenario_terminal():
    write_credentials()
    write_project()
    evidence = {'scenario': 'terminal'}
    server, env = direct_env()
    try:
        for action in ('exit', 'sigterm', 'interrupt'):
            evidence[f'launcher_{action}'] = terminal_case(LAUNCH, None, action)
            evidence[f'direct_{action}'] = terminal_case(['/client', '--model', 'claude-opus-5-5'], env, action)
    finally:
        server.kill()
    ends = [r for r in records() if r.get('event') == 'anthropic_stream_end']
    evidence['interrupt_cancelled_upstream_stream'] = any(not r['completed'] for r in ends)
    # Background start through a job-control shell.
    shell = Session(None, via_shell=True)
    try:
        shell.wait_for('PROMPT>', timeout=10)
        shell.mark()
        shell.send((' '.join(LAUNCH) + ' & wait $!; echo BG_EXIT=$?\n').encode(), settle=2)
        match = None
        deadline = time.monotonic() + 20
        while match is None and time.monotonic() < deadline:
            match = re.search(r'BG_EXIT=(\d+)', shell.screen())
            shell.pump(.3)
        evidence['background_start_exit'] = int(match.group(1)) if match else None
        evidence['background_start_refused'] = 'background invocation refused' in shell.screen()
    finally:
        shell.close()
    checks = []
    for action in ('exit', 'sigterm', 'interrupt'):
        launcher, direct = evidence[f'launcher_{action}'], evidence[f'direct_{action}']
        expected = 143 if action == 'sigterm' else direct.get('exit_code')
        checks.append(launcher.get('ready') and launcher.get('exit_code') == expected
                      and launcher.get('terminal_restored') and not launcher.get('secret_in_terminal_output'))
        evidence[f'{action}_matches_direct'] = launcher.get('exit_code') == direct.get('exit_code')
    interrupt = evidence['launcher_interrupt']
    checks.append(interrupt.get('streaming') and interrupt.get('interrupted_shown')
                  and interrupt.get('next_turn_after_interrupt'))
    checks.append(evidence['background_start_refused'] and evidence['background_start_exit'] not in (None, 0))
    evidence['passed'] = all(checks)
    return evidence


def scenario_suspend():
    write_credentials()
    write_project()
    evidence = {'scenario': 'suspend'}
    shell = Session(None, via_shell=True)
    try:
        shell.wait_for('PROMPT>', timeout=10)
        shell.send((' '.join(LAUNCH) + '\n').encode(), settle=1)
        # The trust prompt is drawn by the client inside the shell's terminal.
        ready = shell.accept_trust()
        evidence['ready'] = ready
        cycles = []
        for cycle in range(3):
            shell.pump(1)
            launcher, client = process_pid('/launcher'), process_pid('/client')
            shell.mark()
            shell.send(CTRL_Z, settle=1.5)
            stopped_shown = shell.wait_for('Stopped', timeout=10) is not None
            states = (proc_state(launcher), proc_state(client))
            shell.mark()
            shell.send(b'fg\n', settle=2)
            resumed = (proc_state(launcher) not in ('T', None)) and (proc_state(client) not in ('T', None))
            prompt = f'CERT-TEXT-{1 + cycle % 2}'
            shell.submit(prompt)
            replied = shell.wait_for(f'CERT-REPLY-{1 + cycle % 2}', timeout=60) is not None
            cycles.append({'stopped_shown': stopped_shown, 'both_stopped': states == ('T', 'T'),
                           'resumed': resumed, 'turn_after_fg': replied})
        evidence['cycles'] = cycles
        shell.mark()
        shell.send(b'/exit', settle=1)
        shell.send(b'\r', settle=2)
        shell.send(b'echo SUSPEND_EXIT=$?\n', settle=1)
        match = None
        deadline = time.monotonic() + 30
        while match is None and time.monotonic() < deadline:
            match = re.search(r'SUSPEND_EXIT=(\d+)', shell.screen())
            if match is None and 'SUSPEND_EXIT=$?' in shell.screen():
                shell.send(b'\r', settle=1)
            shell.pump(.3)
        evidence['exit_code'] = int(match.group(1)) if match else None
        evidence['secret_in_terminal_output'] = shell.leaked()
    finally:
        shell.close()
    common_checks(evidence)
    evidence['passed'] = (evidence['ready'] and all(all(c.values()) for c in evidence['cycles'])
                          and evidence['exit_code'] == 0 and not evidence['secret_in_terminal_output'])
    return evidence


def descendant_case(argv, env):
    """Spawns a background job through the Bash tool, then terminates the
    session mid-stream with SIGTERM."""
    pid_file = HOME / 'spawned.pid'
    pid_file.unlink(missing_ok=True)
    session = Session(argv, env=env)
    result = {}
    try:
        result['ready'] = session.accept_trust()
        session.submit('CERT-SPAWN')
        result['spawn_turn'] = session.wait_for('CERT-SPAWN-DONE-RAN', timeout=60) is not None
        spawned = int(pid_file.read_text().strip()) if pid_file.exists() else None
        client = process_pid('/client')
        client_groups = proc_groups(client) if client else None
        spawned_groups = proc_groups(spawned) if spawned else None
        result['spawned_in_client_group'] = bool(client_groups and spawned_groups
                                                 and spawned_groups[0] == client_groups[0])
        result['spawned_in_client_session'] = bool(client_groups and spawned_groups
                                                   and spawned_groups[1] == client_groups[1])
        session.submit('CERT-SLOW')
        result['streaming'] = session.wait_for('CERT-SLOW-STREAM', timeout=30) is not None
        os.kill(session.pid, signal.SIGTERM)
        result['exit_code'] = session.wait_exit(20)
        time.sleep(1)
        result['client_group_alive_after_exit'] = bool(client_groups) and group_alive(client_groups[0])
        result['client_alive_after_exit'] = proc_state(client) not in (None, 'Z') if client else False
        result['spawned_alive_after_exit'] = proc_state(spawned) not in (None, 'Z') if spawned else None
        if spawned and result['spawned_alive_after_exit']:
            os.kill(spawned, signal.SIGKILL)
    finally:
        session.close()
    return result


def scenario_descendants():
    write_credentials()
    write_project(allow_bash=True)
    evidence = {'scenario': 'descendants'}
    evidence['launcher'] = descendant_case(LAUNCH, None)
    ends = [r for r in records() if r.get('event') == 'anthropic_stream_end']
    evidence['active_request_cancelled_upstream'] = any(not r['completed'] for r in ends)
    server, env = direct_env()
    try:
        evidence['direct'] = descendant_case(['/client', '--model', 'claude-opus-5-5'], env)
    finally:
        server.kill()
    launcher, direct = evidence['launcher'], evidence['direct']
    # The owned process group is cleaned up; a job the client itself detaches
    # into another process group behaves exactly as it does without the
    # launcher (intentionally detached descendants are outside the contract).
    evidence['detached_job_matches_direct'] = (
        launcher.get('spawned_alive_after_exit') == direct.get('spawned_alive_after_exit'))
    evidence['passed'] = (launcher.get('ready') and launcher.get('spawn_turn') and launcher.get('streaming')
                          and launcher.get('exit_code') == 143
                          and not launcher.get('client_alive_after_exit')
                          and not launcher.get('client_group_alive_after_exit')
                          and (launcher.get('spawned_in_client_group') is False
                               or not launcher.get('spawned_alive_after_exit'))
                          and evidence['detached_job_matches_direct']
                          and evidence['active_request_cancelled_upstream'])
    return evidence


ERROR_MARKERS = ('CERT-ERR-401', 'CERT-ERR-429', 'CERT-ERR-500', 'CERT-ERR-INTERRUPT', 'CERT-ERR-MODEL')


def scenario_errors():
    write_credentials()
    write_project()
    evidence = {'scenario': 'errors'}
    session = Session(LAUNCH)
    try:
        evidence['ready'] = session.accept_trust()
        for marker in ERROR_MARKERS:
            before = len(records())
            session.submit(marker)
            # Claude backs off between retries for up to ~40s; wait until the
            # upstream sees nothing new for longer than that, then for idle.
            settled = wait_quiet(quiet=45, limit=600)
            idle = session.wait_for('for shortcuts', timeout=60) is not None
            session.pump(2)
            screen = session.screen()
            mine = [r for r in records()[before:] if r.get('marker') == marker]
            evidence[marker] = {
                'bounded_seconds': settled,
                'error_shown': bool(re.search(r'API Error|/login', screen)),
                'idle_after_error': idle,
                'attempts': len(mine),
                'nonstreaming_attempts': len([r for r in mine if not r['stream']]),
                'models': sorted({str(r.get('model')) for r in mine}),
                'screen': excerpt(screen, 600),
            }
            session.send(b'\x1b', settle=1)
        session.submit('CERT-TEXT-1')
        evidence['recovers_after_errors'] = session.wait_for('CERT-REPLY-1', timeout=60) is not None
        evidence['exit_code'] = session.exit_command()
        evidence['secret_in_terminal_output'] = session.leaked()
    finally:
        session.close()
    anthropic, _ = common_checks(evidence)
    evidence['nonstreaming_requests'] = [
        {k: r.get(k) for k in ('marker', 'max_tokens', 'tool_count', 'message_count')}
        for r in anthropic if not r['stream']]
    # Compare an unsupported-model error with a directly started client.
    server, env = direct_env()
    direct_log = Path('/tmp/direct.jsonl')
    try:
        direct = Session(['/client', '--model', 'claude-opus-5-5'], env=env)
        try:
            direct.accept_trust()
            direct.submit('CERT-ERR-MODEL')
            wait_quiet(quiet=10, limit=120, path=direct_log)
            direct_records = [r for r in records(direct_log)
                              if r.get('event') == 'anthropic' and r.get('marker') == 'CERT-ERR-MODEL']
            evidence['direct_model_error'] = {
                'attempts': len(direct_records),
                'nonstreaming_requests': [{k: r.get(k) for k in ('max_tokens', 'tool_count', 'message_count')}
                                          for r in direct_records if not r['stream']]}
        finally:
            direct.close()
    finally:
        server.kill()
    launcher_model_nonstreaming = [r for r in evidence['nonstreaming_requests'] if r['marker'] == 'CERT-ERR-MODEL']
    evidence['model_error_nonstreaming_matches_direct'] = (
        len(launcher_model_nonstreaming) == len(evidence['direct_model_error']['nonstreaming_requests']))
    evidence['no_model_or_provider_switch'] = all(r.get('model') == 'claude-opus-5-5' for r in anthropic)
    # A non-streaming request is acceptable only where the directly started
    # client sends the same one (its own model check, not a stream fallback).
    evidence['passed'] = (evidence['ready'] and all(
        evidence[m]['bounded_seconds'] is not None and evidence[m]['error_shown'] and evidence[m]['attempts'] > 0
        and evidence[m]['idle_after_error']
        and (evidence[m]['nonstreaming_attempts'] == 0
             or (m == 'CERT-ERR-MODEL' and evidence['model_error_nonstreaming_matches_direct']))
        for m in ERROR_MARKERS)
        and evidence['no_model_or_provider_switch'] and evidence['recovers_after_errors']
        and not evidence['secret_in_terminal_output'])
    return evidence


def scenario_refusals():
    write_credentials()
    evidence = {'scenario': 'refusals'}
    cases = {}

    def run(env=None, cwd=HOME):
        result = subprocess.run(LAUNCH, env=dict(BASE_ENV, **(env or {})), cwd=cwd,
                                capture_output=True, timeout=30)
        return {'exit_code': result.returncode,
                'refused_before_start': b'no listener or client was started' in result.stderr
                or b'no listener was started' in result.stderr,
                'no_upstream_request': not any(r.get('event') == 'anthropic' for r in records()),
                'secret_in_output': b'CANARY' in result.stdout + result.stderr}

    cases['auth_token_override'] = run({'ANTHROPIC_AUTH_TOKEN': 'SYNTHETIC-CANARY'})
    cases['model_override'] = run({'ANTHROPIC_MODEL': 'SYNTHETIC-CANARY'})
    conflict = HOME / 'conflict'
    (conflict / '.claude').mkdir(parents=True, exist_ok=True)
    (conflict / '.claude/settings.json').write_text(json.dumps({'model': 'SYNTHETIC-CANARY'}))
    cases['project_model_setting'] = run(cwd=conflict)
    evidence.update(cases)
    evidence['passed'] = all(case['exit_code'] != 0 and case['refused_before_start']
                             and case['no_upstream_request'] and not case['secret_in_output']
                             for case in cases.values())
    return evidence


def scenario_managed():
    """Run with a synthetic /etc/claude-code/managed-settings.json bound in."""
    write_credentials()
    assert Path('/etc/claude-code/managed-settings.json').exists()
    result = subprocess.run(LAUNCH, env=BASE_ENV, cwd=HOME, capture_output=True, timeout=30)
    evidence = {'scenario': 'managed', 'exit_code': result.returncode,
                'refused_before_start': b'managed configuration present' in result.stderr,
                'no_upstream_request': not any(r.get('event') == 'anthropic' for r in records())}
    evidence['passed'] = (evidence['exit_code'] != 0 and evidence['refused_before_start']
                          and evidence['no_upstream_request'])
    return evidence


SCENARIOS = {'conversation': scenario_conversation, 'terminal': scenario_terminal,
             'suspend': scenario_suspend, 'descendants': scenario_descendants,
             'errors': scenario_errors, 'refusals': scenario_refusals, 'managed': scenario_managed}

if __name__ == '__main__':
    assert Path('/client').is_file() and Path('/launcher').is_file()
    assert os.environ.get('HOME') == '/home/test' and not Path('/home/renade').exists()
    name = sys.argv[1]
    result = SCENARIOS[name]()
    print(json.dumps(result))
    sys.exit(0 if result.get('passed') else 1)
