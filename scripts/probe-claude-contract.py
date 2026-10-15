"""Synthetic actual-client probe. Run only inside the documented bwrap namespace.

The isolated root must contain /client and this script, with no real home mounted.
All emitted evidence is allowlisted; request bodies and credentials are never printed.
"""
import http.server
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import sys

if len(sys.argv) > 1 and sys.argv[1] == 'all':
    for case in ('auth', 'general-purpose', 'reviewer', 'resume', 'policy'):
        subprocess.run([sys.executable, __file__, case], check=True)
    raise SystemExit(0)

assert Path('/client').is_file() and os.environ.get('HOME') == '/home/test'
assert not Path('/home/renade').exists()
home = Path('/home/test')
config = home / '.claude'
config.mkdir(exist_ok=True)
token = 'sk-ant-oat01-SYNTHETIC-OFFLINE-ONLY'
cap = 'SYNTHETIC-LOCAL-CAPABILITY'
(config / '.credentials.json').write_text(json.dumps({'claudeAiOauth': {
    'accessToken': token, 'refreshToken': 'SYNTHETIC-NEVER-REFRESH',
    'expiresAt': int((time.time() + 86400) * 1000),
    'scopes': ['user:inference', 'user:profile'], 'subscriptionType': 'max',
    'rateLimitTier': 'default_claude_max_20x',
}}))
(home / '.claude.json').write_text(json.dumps({'hasCompletedOnboarding': True,
    'oauthAccount': {'accountUuid': '00000000-0000-4000-8000-000000000001',
    'emailAddress': 'synthetic@example.invalid',
    'organizationUuid': '00000000-0000-4000-8000-000000000002'}}))
records = []
scenario = sys.argv[1] if len(sys.argv) > 1 else 'auth'
assert scenario in ('auth', 'general-purpose', 'reviewer', 'resume', 'policy')
work = home
if scenario == 'policy':
    work = home / 'project'
    (work / '.claude').mkdir(parents=True, exist_ok=True)
    (work / '.claude/settings.json').write_text(json.dumps({
        'hooks': {'UserPromptSubmit': [{'hooks': [{'type': 'command',
            'command': '/usr/bin/touch /home/test/hook-ran'}]}]},
        'permissions': {'deny': ['Bash']}}))

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        body = json.loads(raw)
        records.append({'route': self.path.split('?')[0],
            'model': body.get('model'),
            'saved_oauth_preserved': self.headers.get('Authorization') == 'Bearer ' + token,
            'capability_preserved': self.headers.get('x-claude-dual-capability') == cap,
            'oauth_beta_present': 'oauth-2025-04-20' in self.headers.get('anthropic-beta', ''),
            'api_key_absent': self.headers.get('x-api-key') is None,
            'agent_tool_present': any(t.get('name') == 'Agent' for t in body.get('tools', [])),
            'synthetic_assistant_history': any(m.get('role') == 'assistant' and 'synthetic response' in json.dumps(m.get('content')) for m in body.get('messages', [])),
            'one_million_beta_present': 'context-1m-' in self.headers.get('anthropic-beta', ''),
            'tool_error': any(c.get('is_error', False) for m in body.get('messages', [])
                for c in (m.get('content') if isinstance(m.get('content'), list) else [])
                if c.get('type') == 'tool_result'),
            'synthetic_tool_errors': [str(c.get('content', ''))[:400].replace(token, '[redacted]').replace(cap, '[redacted]')
                for m in body.get('messages', [])
                for c in (m.get('content') if isinstance(m.get('content'), list) else [])
                if c.get('type') == 'tool_result' and c.get('is_error')]})
        if scenario != 'auth':
            agent = len(records) == 1 and scenario != 'resume'
            content = ({'type': 'tool_use', 'id': 'toolu_synthetic', 'name': 'Agent',
                'input': {'description': 'Synthetic child', 'prompt': 'Return synthetic text.',
                'subagent_type': 'general-purpose',
                **({'model': 'opus'} if scenario == 'reviewer' else {})}} if agent else {'type': 'text', 'text': 'synthetic response'})
            if agent and scenario == 'policy':
                content = {'type': 'tool_use', 'id': 'toolu_synthetic', 'name': 'Bash',
                    'input': {'command': '/usr/bin/touch /home/test/forbidden-tool-ran', 'description': 'Synthetic policy test'}}
            message = {'id': 'msg_synthetic', 'type': 'message', 'role': 'assistant',
                'model': body.get('model'), 'content': [], 'stop_reason': None,
                'usage': {'input_tokens': 10, 'output_tokens': 1}}
            events = [('message_start', {'type': 'message_start', 'message': message}),
                ('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': content}),
                ('content_block_stop', {'type': 'content_block_stop', 'index': 0}),
                ('message_delta', {'type': 'message_delta', 'delta': {'stop_reason': 'tool_use' if agent else 'end_turn',
                    'stop_sequence': None}, 'usage': {'output_tokens': 5}}),
                ('message_stop', {'type': 'message_stop'})]
            if agent:
                tool_input = content['input']
                content['input'] = {}
                events.insert(2, ('content_block_delta', {'type': 'content_block_delta', 'index': 0,
                    'delta': {'type': 'input_json_delta', 'partial_json': json.dumps(tool_input)}}))
            else:
                content['text'] = ''
                events.insert(2, ('content_block_delta', {'type': 'content_block_delta', 'index': 0,
                    'delta': {'type': 'text_delta', 'text': 'synthetic response'}}))
            data = ''.join(f'event: {name}\ndata: {json.dumps(event)}\n\n' for name, event in events).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        data = json.dumps({'type': 'error', 'error': {'type': 'invalid_request_error',
            'message': 'SYNTHETIC_OFFLINE_STOP'}}).encode()
        self.send_response(400)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
env = dict(os.environ, ANTHROPIC_BASE_URL=f'http://127.0.0.1:{server.server_port}',
    ANTHROPIC_CUSTOM_HEADERS='x-claude-dual-capability: ' + cap,
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1',
    CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK='1',
    CLAUDE_CODE_SUBAGENT_MODEL='gpt-6-astra[1m]')
args = ['/client', '--tools', '',
    '--no-session-persistence', '--model', 'claude-opus-5-5', '-p', 'Synthetic offline probe.']
if scenario in ('general-purpose', 'reviewer'):
    args[args.index('--tools') + 1] = 'Agent'
    args.extend(['--permission-mode', 'dontAsk', '--allowedTools', 'Agent'])
    args.extend(['--agents', json.dumps({'reviewer': {'description': 'Synthetic reviewer',
        'prompt': 'Return synthetic text.', 'model': 'opus'}})])
if scenario == 'resume':
    args.remove('--no-session-persistence')
    args.extend(['--session-id', '00000000-0000-4000-8000-000000000099'])
if scenario == 'policy':
    args[args.index('--tools') + 1] = 'Bash'
    args.extend(['--permission-mode', 'dontAsk', '--allowedTools', 'Bash'])
try:
    result = subprocess.run(args, env=env, cwd=work, capture_output=True, timeout=35)
    first_exit = result.returncode
    if scenario == 'resume':
        args[args.index('--session-id')] = '--resume'
        result = subprocess.run(args, env=env, cwd=work, capture_output=True, timeout=35)
    print(json.dumps({'scenario': scenario, 'first_exit': first_exit, 'client_exit': result.returncode, 'requests': records,
        'safety_hook_ran': (home / 'hook-ran').exists(),
        'forbidden_tool_ran': (home / 'forbidden-tool-ran').exists(),
        'recognized_mock_stop': b'SYNTHETIC_OFFLINE_STOP' in result.stdout + result.stderr,
        'output_contains_capability': cap.encode() in result.stdout + result.stderr,
        'output_contains_oauth': token.encode() in result.stdout + result.stderr}))
    assert records and all(r['saved_oauth_preserved'] and r['capability_preserved'] and r['oauth_beta_present'] and r['api_key_absent'] for r in records)
    assert cap.encode() not in result.stdout + result.stderr and token.encode() not in result.stdout + result.stderr
    if scenario == 'auth':
        assert result.returncode != 0 and b'SYNTHETIC_OFFLINE_STOP' in result.stdout + result.stderr
    else:
        assert first_exit == 0 and result.returncode == 0
    if scenario == 'general-purpose':
        assert any(r['model'] == 'gpt-6-astra' and r['one_million_beta_present'] for r in records)
        assert not any(r['tool_error'] for r in records)
    if scenario == 'reviewer':
        assert len(records) >= 3 and all(r['model'] == 'claude-opus-5-5' and not r['tool_error'] for r in records)
    if scenario == 'resume':
        assert any(r['synthetic_assistant_history'] for r in records)
    if scenario == 'policy':
        assert (home / 'hook-ran').exists() and not (home / 'forbidden-tool-ran').exists()
except subprocess.TimeoutExpired:
    print(json.dumps({'timeout': True, 'requests': records}))
    raise SystemExit(1)
finally:
    server.shutdown()
