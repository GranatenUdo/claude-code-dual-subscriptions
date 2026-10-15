"""Linux PTY integration; only a fake Claude child and temporary HOME are used."""
import errno
import os
from pathlib import Path
import pty
import re
import select
import signal
import tempfile
import time
import unittest

BINARY = Path(os.environ.get('CCD_BINARY') or Path(
    os.environ.get('CARGO_TARGET_DIR') or Path(__file__).resolve().parents[1] / 'target')
    / 'debug/claude-code-dual')

JOB_CLIENT = '''#!/usr/bin/python3
import os,sys,time
if '--version' in sys.argv:
    print('2.1.300 (Claude Code)'); sys.exit(0)
open('client.pid','w').write(f'{os.getpid()} {os.getppid()}')
mode=open('mode').read().strip()
print('TTY_READY',flush=True)
if mode == 'read':
    value=input()
    sys.exit(17 if value=='hello' else 82)
while not os.path.exists('exit-now'):
    time.sleep(.05)
sys.exit(23)
'''


def proc_state(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
    except FileNotFoundError:
        return None


class Pty:
    """An interactive bash with job control on a fresh pseudo-terminal."""

    def __init__(self, root):
        self.root = root
        self.output = b''
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(root)
            os.execve('/bin/bash', ['bash', '--norc', '--noprofile', '-i'],
                {'HOME': str(root), 'PATH': '/usr/bin:/bin', 'TERM': 'dumb', 'PS1': 'PROMPT> '})

    def read_until(self, needle, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if needle in self.output:
                return True
            if select.select([self.fd], [], [], .1)[0]:
                try:
                    self.output += os.read(self.fd, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    return needle in self.output
        return needle in self.output

    def read_match(self, pattern, timeout=10):
        """Matches command output, not the terminal's echo of the command."""
        regex = re.compile(pattern)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            match = regex.search(self.output)
            if match:
                return match
            self.read_until(b'\0', timeout=.1)
        return regex.search(self.output)

    def send(self, data):
        os.write(self.fd, data)

    def mark(self):
        self.output = b''

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(self.pid, 0)
        os.close(self.fd)

class TtyTests(unittest.TestCase):
    def run_case(self, interrupt, background=False):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            client = root / 'claude'
            client.write_text('''#!/usr/bin/python3
import os,sys
if '--version' in sys.argv:
    print('2.1.300 (Claude Code)'); sys.exit(0)
if not os.isatty(0) or os.tcgetpgrp(0) != os.getpgrp(): sys.exit(81)
print('TTY_READY',flush=True)
try:
    value=input()
    sys.exit(17 if value=='hello' else 82)
except KeyboardInterrupt:
    sys.exit(130)
''')
            client.chmod(0o700)
            pid, fd = pty.fork()
            if pid == 0:
                os.chdir(root)
                if background:
                    launched = os.fork()
                    if launched:
                        _, child_status = os.waitpid(launched, 0)
                        preserved = os.tcgetpgrp(0) == os.getpgrp()
                        os.write(1, b'BACKGROUND_REFUSED\n')
                        os._exit(0 if preserved and os.waitstatus_to_exitcode(child_status) != 0 else 85)
                    os.setpgid(0, 0)
                os.execve(BINARY, [str(BINARY), 'launch', '--claude', str(client)],
                    {'HOME': folder, 'PATH': '/usr/bin:/bin', 'TERM': 'xterm-256color'})
            output = b''
            sent = False
            status = None
            try:
                deadline = time.monotonic() + 12
                while time.monotonic() < deadline:
                    if select.select([fd], [], [], .1)[0]:
                        try:
                            output += os.read(fd, 4096)
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                        if b'TTY_READY' in output and not sent:
                            if interrupt == 'term':
                                os.kill(pid, signal.SIGTERM)
                            else:
                                os.write(fd, b'\x03' if interrupt else b'hello\n')
                            sent = True
                        if background and b'BACKGROUND_REFUSED' in output:
                            sent = True
                    done, value = os.waitpid(pid, os.WNOHANG)
                    if done:
                        status = value
                        break
                self.assertTrue(sent, output.decode(errors='replace'))
                self.assertIsNotNone(status, 'launcher did not finish')
                expected = 0 if background else 143 if interrupt == 'term' else 130 if interrupt else 17
                self.assertEqual(os.waitstatus_to_exitcode(status), expected)
            finally:
                if status is None:
                    os.kill(pid, signal.SIGTERM)
                    os.waitpid(pid, 0)
                os.close(fd)

    def test_foreground_tty_input(self):
        self.run_case(False)

    def test_ctrl_c_reaches_owned_child(self):
        self.run_case(True)

    def test_wrapper_sigterm_returns_143(self):
        self.run_case('term')

    def test_background_launcher_does_not_steal_terminal(self):
        self.run_case(False, background=True)


class JobControlTests(unittest.TestCase):
    def start(self, mode):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        root = Path(folder.name)
        client = root / 'claude'
        client.write_text(JOB_CLIENT)
        client.chmod(0o700)
        (root / 'mode').write_text(mode)
        shell = Pty(root)
        self.addCleanup(shell.close)
        self.assertTrue(shell.read_until(b'PROMPT> '), shell.output)
        shell.send(f"'{BINARY}' launch --claude '{client}'\n".encode())
        self.assertTrue(shell.read_until(b'TTY_READY'), shell.output)
        child, wrapper = (int(value) for value in (root / 'client.pid').read_text().split())
        return root, shell, child, wrapper

    def suspend(self, shell, child, wrapper):
        shell.mark()
        shell.send(b'\x1a')
        self.assertTrue(shell.read_until(b'Stopped'), shell.output)
        self.assertEqual(proc_state(child), 'T')
        self.assertEqual(proc_state(wrapper), 'T')

    def exit_code(self, shell):
        shell.mark()
        shell.send(b'echo EXIT=$?\n')
        match = shell.read_match(rb'EXIT=(\d+)')
        self.assertIsNotNone(match, shell.output)
        return int(match.group(1))

    def test_repeated_suspend_and_foreground_resume_the_session(self):
        _, shell, child, wrapper = self.start('read')
        for _ in range(3):
            self.suspend(shell, child, wrapper)
            shell.mark()
            shell.send(b'fg\n')
            deadline = time.monotonic() + 5
            while proc_state(child) == 'T' and time.monotonic() < deadline:
                time.sleep(.05)
            self.assertNotEqual(proc_state(child), 'T')
        shell.send(b'hello\n')
        self.assertTrue(shell.read_until(b'PROMPT> '), shell.output)
        self.assertEqual(self.exit_code(shell), 17)

    def test_background_job_finishes_and_shell_keeps_the_terminal(self):
        root, shell, child, wrapper = self.start('wait')
        self.suspend(shell, child, wrapper)
        shell.send(b'bg\n')
        self.assertTrue(shell.read_until(b'PROMPT> '), shell.output)
        (root / 'exit-now').write_text('')
        shell.mark()
        shell.send(b'wait %1; echo WAITED=$?; echo FG_OK\n')
        waited = shell.read_match(rb'WAITED=(\d+)')
        self.assertIsNotNone(waited, shell.output)
        self.assertEqual(int(waited.group(1)), 23)
        # The shell still reads and writes the terminal in the foreground.
        self.assertIsNotNone(shell.read_match(rb'(?m)^FG_OK\r?$'), shell.output)

    def test_killing_a_suspended_job_terminates_the_session(self):
        _, shell, child, wrapper = self.start('read')
        self.suspend(shell, child, wrapper)
        shell.mark()
        shell.send(b'kill %1; wait %1; echo KILLED=$?\n')
        killed = shell.read_match(rb'KILLED=(\d+)')
        self.assertIsNotNone(killed, shell.output)
        self.assertEqual(int(killed.group(1)), 143)
        deadline = time.monotonic() + 3
        while proc_state(child) not in (None, 'Z') and time.monotonic() < deadline:
            time.sleep(.05)
        self.assertIn(proc_state(child), (None, 'Z'))

    def test_hangup_ends_the_session_with_129(self):
        _, shell, child, wrapper = self.start('read')
        shell.mark()
        os.kill(wrapper, signal.SIGHUP)
        self.assertTrue(shell.read_until(b'PROMPT> '), shell.output)
        self.assertEqual(self.exit_code(shell), 129)

    def test_orphaned_launcher_resumes_the_child_instead_of_hanging(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            client = root / 'claude'
            client.write_text(JOB_CLIENT)
            client.chmod(0o700)
            (root / 'mode').write_text('read')
            pid, fd = pty.fork()
            if pid == 0:
                os.chdir(root)
                os.execve(BINARY, [str(BINARY), 'launch', '--claude', str(client)],
                    {'HOME': folder, 'PATH': '/usr/bin:/bin', 'TERM': 'dumb'})
            output = b''
            status = None
            try:
                deadline = time.monotonic() + 12
                stage = 0
                while time.monotonic() < deadline:
                    if select.select([fd], [], [], .1)[0]:
                        try:
                            output += os.read(fd, 4096)
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                    if stage == 0 and b'TTY_READY' in output:
                        os.write(fd, b'\x1a')
                        stage = 1
                        resumed_by = time.monotonic() + 3
                    elif stage == 1:
                        child = int((root / 'client.pid').read_text().split()[0])
                        if proc_state(child) not in ('T', None) or time.monotonic() > resumed_by:
                            os.write(fd, b'hello\n')
                            stage = 2
                    done, value = os.waitpid(pid, os.WNOHANG)
                    if done:
                        status = value
                        break
                self.assertEqual(stage, 2, output.decode(errors='replace'))
                self.assertIsNotNone(status, 'orphaned launcher hung after Ctrl-Z')
                self.assertEqual(os.waitstatus_to_exitcode(status), 17)
            finally:
                if status is None:
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                os.close(fd)

if __name__ == '__main__':
    unittest.main()
