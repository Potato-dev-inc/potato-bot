#!/usr/bin/env python3
# Puppet Master: tests for pm-sbx-secret-hub, the root process that owns MCP servers which hold secrets, and for the way
# the plain hub no longer takes any. Written for this repo. Pure-logic tests run anywhere; the tests that drop to another
# uid and look at /proc need Linux and root and are skipped elsewhere (they were run in a scratch WSL distro as root).
"""Run with: python -I -m unittest discover -s <this folder> -p "test_*.py"
"""

import importlib.machinery
import importlib.util
import io
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import unittest.mock

HERE = os.path.dirname(os.path.abspath(__file__))
FAKE_TOKEN = "fake-token-4c1e9a07d2b8"
# The config rules refuse spaces in a command, and a Windows interpreter path has some.
PYTHON = sys.executable if os.name != "nt" else "python"
ON_LINUX_ROOT = sys.platform.startswith("linux") and hasattr(os, "geteuid") and os.geteuid() == 0

FAKE_SERVER = r'''
import json, os, sys, time
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
def text(value):
    return {"content": [{"type": "text", "text": value}], "isError": False}
for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg:
        continue
    rid, method, params = msg["id"], msg["method"], msg.get("params") or {}
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": rid, "result": {"tools": [{"name": n, "description": n, "inputSchema": {"type": "object"}} for n in ("whoami", "has_token", "crash", "write_home")]}})
    elif method == "tools/call":
        name = params.get("name")
        if name == "has_token":
            send({"jsonrpc": "2.0", "id": rid, "result": text("yes" if os.environ.get("FAKE_SECRET") else "no")})
        elif name == "whoami":
            try:
                import pwd
                user = pwd.getpwuid(os.getuid()).pw_name
            except KeyError:
                user = "none"
            send({"jsonrpc": "2.0", "id": rid, "result": text(json.dumps({"uid": os.getuid(), "gid": os.getgid(), "groups": os.getgroups(), "home": os.environ.get("HOME"), "user": user, "display": os.environ.get("DISPLAY")}))})
        elif name == "write_home":
            path = os.path.join(os.environ["HOME"], "tmp", "x")
            open(path, "w").write("ok")
            send({"jsonrpc": "2.0", "id": rid, "result": text(path)})
        elif name == "crash":
            sys.exit(3)
        else:
            send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32602, "message": "unknown tool"}})
    else:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}})
'''


def _load(module_name, filename):
    loader = importlib.machinery.SourceFileLoader(module_name, os.path.join(HERE, filename))
    spec = importlib.util.spec_from_loader(module_name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


hub_mod = _load("pm_sbx_mcp_hub_sh_tests", "pm-sbx-mcp-hub")
secret = _load("pm_sbx_secret_hub", "pm-sbx-secret-hub")


def _wait_for(predicate, seconds=20.0):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(0.05)
    return False


def fake_files(tmp):
    path = os.path.join(tmp, "fake_mcp.py")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(FAKE_SERVER)
    os.chmod(tmp, 0o755) if os.name == "posix" else None
    os.chmod(path, 0o644) if os.name == "posix" else None
    return path


def config_request(fake, name="probe", rid=1):
    return {"id": rid, "op": "config", "servers": [{"name": name, "command": PYTHON, "args": ["-I", fake], "env": {"FAKE_SECRET": FAKE_TOKEN}}]}


class SessionTests(unittest.TestCase):
    """The request handling, with no uid drop (this machine cannot do one), over the real MCP hub code."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="pm-sechub-")
        self.fake = fake_files(self.dir)
        self.session = secret.Session(hub_mod, log_dir=os.path.join(self.dir, "logs"), drop=False, call_timeout=2.0, start_timeout=10)

    def tearDown(self):
        self.session.close()
        shutil.rmtree(self.dir, ignore_errors=True)

    def ready(self, name="probe"):
        return _wait_for(lambda: any(s["name"] == name and s["status"] == "ready" for s in self.session.handle({"id": 0, "op": "tools"})["servers"]))

    def test_config_gives_the_server_its_env_and_no_reply_carries_a_value(self):
        reply = self.session.handle(config_request(self.fake))
        self.assertTrue(reply["ok"])
        self.assertTrue(self.ready())
        tools = self.session.handle({"id": 2, "op": "tools"})
        called = self.session.handle({"id": 3, "op": "call", "server": "probe", "tool": "has_token", "arguments": {}})
        self.assertEqual(called["content"][0]["text"], "yes", "the server process has the value")
        for message in (reply, tools, called):
            self.assertNotIn(FAKE_TOKEN, json.dumps(message))
            self.assertNotIn("FAKE_SECRET", json.dumps(message))
            self.assertNotIn('"env"', json.dumps(message))

    def test_the_built_in_computer_server_is_not_here_and_cannot_be_called_or_replaced(self):
        self.session.handle(config_request(self.fake))
        names = [s["name"] for s in self.session.handle({"id": 1, "op": "tools"})["servers"]]
        self.assertEqual(names, ["probe"])
        bad = self.session.handle({"id": 2, "op": "call", "server": "computer", "tool": "x", "arguments": {}})
        self.assertFalse(bad["ok"])
        replaced = self.session.handle({"id": 3, "op": "config", "servers": [{"name": "computer", "command": "x"}]})
        self.assertFalse(replaced["ok"])

    def test_bad_requests_get_a_plain_error_and_never_end_the_hub(self):
        for request in ({"id": 1, "op": "nope"}, {"id": 2}, {"id": 3, "op": "call", "server": "ghost", "tool": "x", "arguments": {}}, [1], {"id": 4, "op": "config", "servers": "x"}):
            reply = self.session.handle(request)
            self.assertFalse(reply["ok"], request)
            self.assertIn(reply["status"], (400, 500))
        self.assertTrue(self.session.handle({"id": 9, "op": "ping"})["ok"])

    def test_a_crash_restarts_the_server_once_then_it_stays_failed(self):
        self.session.handle(config_request(self.fake))
        self.assertTrue(self.ready())
        first = self.session.handle({"id": 2, "op": "call", "server": "probe", "tool": "crash", "arguments": {}})
        self.assertFalse(first["ok"])
        self.assertTrue(self.ready(), "one restart")
        self.session.handle({"id": 3, "op": "call", "server": "probe", "tool": "crash", "arguments": {}})
        self.assertTrue(_wait_for(lambda: self.session.handle({"id": 4, "op": "tools"})["servers"][0]["status"] == "failed"))

    def test_serve_answers_one_line_per_request_and_returns_when_stdin_closes(self):
        lines = [json.dumps({"id": 1, "op": "ping"}), "not json", json.dumps({"id": 2, "op": "tools"})]
        out = io.BytesIO()
        secret.serve(self.session, io.BytesIO(("\n".join(lines) + "\n").encode()), out)
        replies = [json.loads(line) for line in out.getvalue().decode().splitlines()]
        self.assertEqual([r.get("ok") for r in replies], [True, False, True])
        self.assertEqual(replies[0]["id"], 1)

    def test_config_over_the_plain_hub_still_refuses_env(self):
        plain = hub_mod.McpHub(display=":99", log_dir=os.path.join(self.dir, "l2"), config_path=os.path.join(self.dir, "c.json"), computer_script="x")
        try:
            plain.replace_user_servers([{"name": "a", "command": "srv", "env": {"K": FAKE_TOKEN}}])
        except hub_mod.InvalidRequest:
            pass
        else:
            self.fail("the pm-side hub must never be handed a value")
        finally:
            plain.close()


class IdentityTests(unittest.TestCase):
    def test_a_uid_per_name_in_a_range_no_real_user_has(self):
        ids = {secret.uid_for(n) for n in ("github", "notion", "a", "b-2")}
        self.assertTrue(all(60000 <= i < 64000 for i in ids))
        self.assertEqual(secret.uid_for("github"), secret.uid_for("github"))
        self.assertNotEqual(secret.uid_for("github"), secret.uid_for("notion"))

    def test_the_agents_uid_is_outside_the_range_and_a_server_is_never_given_it(self):
        self.assertEqual(secret.AGENT_UID, 59999)
        self.assertFalse(secret.UID_BASE <= secret.AGENT_UID < secret.UID_BASE + secret.UID_SPAN, "the agent's user is not in the servers' range")
        was = secret.AGENT_UID
        try:
            secret.AGENT_UID = secret.uid_for("github")
            with self.assertRaises(ValueError):
                secret.uid_for("github")
            session = secret.Session(hub_mod, log_dir=tempfile.mkdtemp(prefix="pm-sechub-"), drop=False)
            try:
                reply = session.handle({"id": 1, "op": "config", "servers": [{"name": "github", "command": "true", "args": [], "env": {}}]})
            finally:
                session.close()
            self.assertFalse(reply["ok"])
            self.assertEqual(reply["status"], 400)
            self.assertIn("not allowed", reply["error"])
        finally:
            secret.AGENT_UID = was

    def test_a_passwd_and_group_line_is_written_once_and_never_over_a_real_user(self):
        tmp = tempfile.mkdtemp(prefix="pm-passwd-")
        try:
            passwd, group = os.path.join(tmp, "passwd"), os.path.join(tmp, "group")
            open(passwd, "w").write("root:x:0:0:root:/root:/bin/bash\npm:x:1000:1000::/home/pm:/bin/bash\n")
            open(group, "w").write("root:x:0:\npm:x:1000:\n")
            for _ in range(2):
                secret.ensure_passwd("github", passwd=passwd, group=group, home_root="/var/lib/pm-sbx-sec")
            uid = secret.uid_for("github")
            rows = open(passwd).read().splitlines()
            self.assertEqual(len(rows), 3)
            self.assertEqual(rows[2], "pmsec%d:x:%d:%d:pm secret server:/var/lib/pm-sbx-sec/github:/usr/sbin/nologin" % (uid, uid, uid))
            self.assertEqual(len(open(group).read().splitlines()), 3)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_the_hub_has_no_socket_and_no_sudo(self):
        text = open(os.path.join(HERE, "pm-sbx-secret-hub"), encoding="utf-8").read()
        for word in ("import socket", "bind(", "listen(", "HTTPServer", "sudo "):
            self.assertNotIn(word, text.split('"""', 2)[2], word)


@unittest.skipUnless(os.name == "posix" and hasattr(os, "fchown"), "needs unix permissions and symlinks")
class PrepareSafetyTests(unittest.TestCase):
    """The server's own user can swap its folders for links; root must not chown or chmod through one."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="pm-prep-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.home_root = os.path.join(self.dir, "home")
        self.logs = os.path.join(self.dir, "logs")
        self.passwd = os.path.join(self.dir, "passwd")
        self.group = os.path.join(self.dir, "group")
        for path in (self.passwd, self.group):
            open(path, "w").close()
        self.victim = os.path.join(self.dir, "victim")
        os.mkdir(self.victim, 0o755)
        # The test's own uid stands in for the server's, so a non-root run can give a folder to it.
        uid = os.geteuid()
        orig = secret.uid_for
        secret.uid_for = lambda name: uid
        self.addCleanup(setattr, secret, "uid_for", orig)

    def prepare(self):
        secret.prepare("probe", self.home_root, self.logs, self.passwd, self.group)

    def test_a_home_is_made_private_without_chown_by_path(self):
        with unittest.mock.patch.object(os, "chown", side_effect=AssertionError("no chown by path")),                 unittest.mock.patch.object(os, "chmod", side_effect=AssertionError("no chmod by path")):
            self.prepare()
        for folder in ("probe", "probe/tmp"):
            self.assertEqual(os.stat(os.path.join(self.home_root, folder)).st_mode & 0o777, 0o700)
        self.assertEqual(os.stat(self.logs).st_mode & 0o777, 0o700)

    def test_a_tmp_replaced_by_a_link_is_refused_and_the_target_untouched(self):
        self.prepare()
        tmp = os.path.join(self.home_root, "probe", "tmp")
        os.rmdir(tmp)
        os.symlink(self.victim, tmp)
        with self.assertRaises(OSError):
            self.prepare()
        self.assertEqual(os.stat(self.victim).st_mode & 0o777, 0o755)

    def test_a_home_replaced_by_a_link_is_refused(self):
        os.makedirs(self.home_root, mode=0o711)
        os.symlink(self.victim, os.path.join(self.home_root, "probe"))
        with self.assertRaises(OSError):
            self.prepare()
        self.assertEqual(os.stat(self.victim).st_mode & 0o777, 0o755)
        self.assertEqual(os.listdir(self.victim), [])


@unittest.skipUnless(ON_LINUX_ROOT, "needs Linux and root")
class RealHubTests(unittest.TestCase):
    """The real thing, as root: the hub process, a real uid drop, and an unprivileged user trying to get at it."""

    NOBODY = 65534

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="pm-sechub-real-")
        os.chmod(self.dir, 0o755)
        self.fake = fake_files(self.dir)
        env = {"PATH": "/usr/local/bin:/usr/bin:/bin", "PM_TEST_ONLY": "1"}
        self.proc = subprocess.Popen([sys.executable, "-I", os.path.join(HERE, "pm-sbx-secret-hub")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, env=env)
        self.next_id = 0

    def tearDown(self):
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        secret.reap_stale()
        shutil.rmtree(self.dir, ignore_errors=True)

    def ask(self, body):
        self.next_id += 1
        self.proc.stdin.write((json.dumps({"id": self.next_id, **body}) + "\n").encode())
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def server_pids(self):
        lo, hi = secret.UID_BASE, secret.UID_BASE + secret.UID_SPAN
        found = []
        for entry in os.listdir("/proc"):
            if entry.isdigit():
                try:
                    uid = int([l for l in open("/proc/%s/status" % entry) if l.startswith("Uid:")][0].split()[1])
                except (OSError, IndexError):
                    continue
                if lo <= uid < hi:
                    found.append(int(entry))
        return found

    def start_probe(self):
        self.assertTrue(self.ask(config_request(self.fake, rid=0))["ok"])
        self.assertTrue(_wait_for(lambda: self.ask({"op": "tools"})["servers"][0]["status"] == "ready"))
        return self.server_pids()

    def unprivileged(self, code):
        out = subprocess.run([sys.executable, "-I", "-c", code], user=self.NOBODY, group=self.NOBODY, extra_groups=[], capture_output=True, text=True)
        return out.stdout.split(), out.stderr

    def test_the_server_runs_as_its_own_uid_with_a_home_a_passwd_entry_and_no_display(self):
        self.start_probe()
        who = json.loads(self.ask({"op": "call", "server": "probe", "tool": "whoami", "arguments": {}})["content"][0]["text"])
        uid = secret.uid_for("probe")
        self.assertEqual((who["uid"], who["gid"], who["groups"]), (uid, uid, []))
        self.assertEqual(who["home"], "/var/lib/pm-sbx-sec/probe")
        self.assertEqual(who["user"], "pmsec%d" % uid, "a passwd entry exists, so a program that looks its user up works")
        self.assertIsNone(who["display"])
        written = self.ask({"op": "call", "server": "probe", "tool": "write_home", "arguments": {}})
        self.assertEqual(written["content"][0]["text"], "/var/lib/pm-sbx-sec/probe/tmp/x")

    def test_an_unprivileged_user_cannot_read_the_servers_environ_or_memory_and_root_can(self):
        pids = self.start_probe()
        self.assertEqual(len(pids), 1)
        pid = pids[0]
        self.assertIn(FAKE_TOKEN.encode(), open("/proc/%d/environ" % pid, "rb").read(), "the test is meaningful: root sees it")
        out, err = self.unprivileged(
            "import os\nfor p in ('/proc/%d/environ','/proc/%d/mem','/proc/%d/cmdline_x'):\n"
            " try:\n  open(p,'rb').read(1); print('READ')\n except OSError: print('DENIED')" % (pid, pid, pid))
        self.assertEqual(out, ["DENIED", "DENIED", "DENIED"], err)
        out, _ = self.unprivileged("import os,signal\ntry:\n os.kill(%d, 0); print('SIGNAL')\nexcept OSError: print('DENIED')" % pid)
        self.assertEqual(out, ["DENIED"], "it cannot even signal the server")

    def test_an_unprivileged_user_cannot_reach_the_hub_and_the_hub_has_no_socket_or_file_to_reach(self):
        self.start_probe()
        hub = self.proc.pid
        out, err = self.unprivileged(
            "for p in ('/proc/%d/environ','/proc/%d/fd/0','/proc/%d/fd/1','/proc/%d/mem'):\n"
            " try:\n  open(p,'rb'); print('OPENED')\n except OSError: print('DENIED')" % (hub, hub, hub, hub))
        self.assertEqual(out, ["DENIED"] * 4, err)
        kinds = [os.readlink("/proc/%d/fd/%s" % (hub, fd)) for fd in os.listdir("/proc/%d/fd" % hub)]
        self.assertFalse([k for k in kinds if k.startswith("socket:")], kinds)

    def test_a_crash_is_restarted_and_a_closed_pipe_leaves_no_server_behind(self):
        self.start_probe()
        self.ask({"op": "call", "server": "probe", "tool": "crash", "arguments": {}})
        self.assertTrue(_wait_for(lambda: self.ask({"op": "tools"})["servers"][0]["status"] == "ready"), "one restart")
        self.assertTrue(self.server_pids())
        self.proc.stdin.close()
        self.proc.wait(timeout=15)
        self.assertTrue(_wait_for(lambda: not self.server_pids(), 10), "no orphan after the host's pipe closed")

    def test_a_server_left_by_a_killed_hub_is_reaped_by_the_next_one(self):
        sleeper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], user=secret.uid_for("old"), group=secret.uid_for("old"), extra_groups=[])
        try:
            time.sleep(0.3)
            self.assertIn(sleeper.pid, self.server_pids())
            self.assertGreaterEqual(secret.reap_stale(), 1)
            self.assertIsNotNone(sleeper.wait(timeout=5))
            self.assertEqual(sleeper.returncode, -signal.SIGKILL)
        finally:
            sleeper.kill()


if __name__ == "__main__":
    unittest.main()
