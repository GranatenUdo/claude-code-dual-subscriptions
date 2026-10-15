"""Linux PTY integration; only a fake Claude child and temporary HOME are used."""
import errno
import os
from pathlib import Path
import pty
import select
import signal
import tempfile
import time
import unittest

BINARY = Path(__file__).resolve().parents[1] / 'target/debug/claude-code-dual'

class TtyTests(unittest.TestCase):
    def run_case(self, interrupt, background=False):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            client = root / 'claude'
            client.write_text('''#!/usr/bin/python3
import os,sys
if '--version' in sys.argv:
    print('2.1.299 (Claude Code)'); sys.exit(0)
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

if __name__ == '__main__':
    unittest.main()
