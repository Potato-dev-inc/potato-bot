#!/usr/bin/env python3
# Puppet Master: tests for the MCP hub (pm-sbx-mcp-hub), the built-in computer server (pm-sbx-computer-mcp) and the
# /v1/mcp routes of pm-sbx-control. Written for this repo. A fake stdio MCP server stands in for real servers; no X
# server is needed (the computer tools are checked as argv builders and through the stdio protocol).
"""Run with: python -I -m unittest discover -s <this folder> -p "test_*.py"
"""

import importlib.machinery
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
TOKEN = "t0k3n-for-mcp-tests-0123456789"

FAKE_SERVER = r'''
import json, sys, time
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
TOOLS = [
    {"name": "echo", "description": "echo text", "inputSchema": {"type": "object"}},
    {"name": "crash", "description": "exit now", "inputSchema": {"type": "object"}},
    {"name": "hang", "description": "never answer", "inputSchema": {"type": "object"}},
    {"name": "big", "description": "too much output", "inputSchema": {"type": "object"}},
]
for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg:
        continue
    rid, method, params = msg["id"], msg["method"], msg.get("params") or {}
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
              "serverInfo": {"name": "fake", "version": "1"}}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}})
    elif method == "tools/call":
        name, args = params.get("name"), params.get("arguments") or {}
        if name == "echo":
            send({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": args.get("text", "")}], "isError": False}})
        elif name == "crash":
            sys.exit(3)
        elif name == "hang":
            time.sleep(3600)
        elif name == "big":
            send({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": "x" * (5 * 1024 * 1024)}]}})
        else:
            send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32602, "message": "unknown tool"}})
    else:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}})
'''


def _load(module_name, filename):
    path = os.path.join(HERE, filename)
    loader = importlib.machinery.SourceFileLoader(module_name, path)
    spec = importlib.util.spec_from_loader(module_name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


control = _load("pm_sbx_control", "pm-sbx-control")
hub_mod = _load("pm_sbx_mcp_hub_tests", "pm-sbx-mcp-hub")
computer = _load("pm_sbx_computer_mcp", "pm-sbx-computer-mcp")
COMPUTER_SCRIPT = os.path.join(HERE, "pm-sbx-computer-mcp")


def _wait_for(predicate, seconds=20.0):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(0.05)
    return False


class FakeServerFixture(unittest.TestCase):
    """A temp folder with the fake server, a config file, and a hub that runs the fake as user server 'fake'."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="pm-mcp-test-")
        self.fake = os.path.join(self.dir, "fake_mcp.py")
        with open(self.fake, "w", encoding="utf-8") as handle:
            handle.write(FAKE_SERVER)
        self.config = os.path.join(self.dir, "state", "mcp.json")
        self.logs = os.path.join(self.dir, "logs")
        self.hub = hub_mod.McpHub(
            display=":99",
            log_dir=self.logs,
            config_path=self.config,
            computer_script=COMPUTER_SCRIPT,
            call_timeout=1.0,
            start_timeout=10,
        )

    def tearDown(self):
        self.hub.close()

    def add_fake(self, name="fake", extra_env=None):
        # Installed directly: the Windows test interpreter path can contain spaces, which the config rules refuse.
        specs = [hub_mod.ServerSpec(name, (sys.executable, "-I", self.fake), dict(extra_env or {}))]
        hub_mod.save_config(self.config, specs)
        self.hub._install(specs)

    def status_of(self, name):
        for server in self.hub.snapshot()["servers"]:
            if server["name"] == name:
                return server
        return None

    def wait_ready(self, name="fake"):
        return _wait_for(lambda: (self.status_of(name) or {}).get("status") == "ready")


class HubTests(FakeServerFixture):
    def test_list_and_call(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        server = self.status_of("fake")
        self.assertEqual({tool["name"] for tool in server["tools"]}, {"echo", "crash", "hang", "big"})
        reply = self.hub.call("fake", "echo", {"text": "hi"})
        self.assertEqual(reply, {"content": [{"type": "text", "text": "hi"}], "isError": False})

    def test_builtin_computer_server_is_always_present(self):
        self.hub.start()
        self.assertTrue(_wait_for(lambda: (self.status_of("computer") or {}).get("status") == "ready"))
        names = {tool["name"] for tool in self.status_of("computer")["tools"]}
        self.assertEqual(names, computer.TOOL_NAMES)

    def test_hung_child_times_out_then_restarts_once(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        started = time.monotonic()
        with self.assertRaises(RuntimeError) as caught:
            self.hub.call("fake", "hang", {})
        self.assertIn("timed out", str(caught.exception))
        self.assertLess(time.monotonic() - started, 8)
        self.assertTrue(self.wait_ready())
        self.assertEqual(self.hub.call("fake", "echo", {"text": "again"})["content"][0]["text"], "again")

    def test_crash_restarts_once_then_stays_failed(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        with self.assertRaises(RuntimeError):
            self.hub.call("fake", "crash", {})
        self.assertTrue(self.wait_ready())
        self.assertEqual(self.hub.call("fake", "echo", {"text": "up"})["content"][0]["text"], "up")
        with self.assertRaises(RuntimeError):
            self.hub.call("fake", "crash", {})
        self.assertTrue(_wait_for(lambda: self.status_of("fake")["status"] == "failed"))
        with self.assertRaises(RuntimeError) as caught:
            self.hub.call("fake", "echo", {"text": "x"})
        self.assertIn("not running", str(caught.exception))

    def test_oversize_output_kills_child(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        with self.assertRaises(RuntimeError):
            self.hub.call("fake", "big", {})
        self.assertTrue(self.wait_ready())
        self.assertEqual(self.hub.call("fake", "echo", {"text": "ok"})["content"][0]["text"], "ok")

    def test_unknown_server_and_tool_are_refused(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        with self.assertRaises(hub_mod.InvalidRequest):
            self.hub.call("nope", "echo", {})
        with self.assertRaises(hub_mod.InvalidRequest):
            self.hub.call("fake", "nope", {})
        with self.assertRaises(hub_mod.InvalidRequest):
            self.hub.call("Bad Name", "echo", {})
        with self.assertRaises(hub_mod.InvalidRequest):
            self.hub.call("fake", "echo", ["not", "an", "object"])

    def test_config_is_saved_root_only_and_reloaded(self):
        self.add_fake(extra_env={"MY_SETTING": "value"})
        self.assertTrue(os.path.exists(self.config))
        if os.name != "nt":
            self.assertEqual(os.stat(self.config).st_mode & 0o777, 0o600)
        with open(self.config, encoding="utf-8") as handle:
            saved = json.load(handle)
        self.assertEqual([entry["name"] for entry in saved["servers"]], ["fake"])
        self.assertEqual(saved["servers"][0]["env"], {"MY_SETTING": "value"})
        self.assertEqual(saved["servers"][0]["command"], sys.executable)
        # A saved file that the rules refuse is ignored at start, never loaded half way.
        with open(self.config, "w", encoding="utf-8") as handle:
            json.dump({"servers": [{"name": "computer", "command": "x"}]}, handle)
        self.assertEqual(hub_mod.load_config(self.config), [])

    def test_stderr_goes_to_a_per_server_file(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        self.assertTrue(os.path.exists(os.path.join(self.logs, "fake.stderr.log")))

    def test_close_reaps_children(self):
        self.add_fake()
        self.assertTrue(self.wait_ready())
        child = self.hub._user["fake"].child
        self.assertIsNotNone(child)
        self.hub.close()
        self.assertIsNotNone(child.proc.poll())


class ValidationTests(unittest.TestCase):
    def check_refused(self, raw):
        with self.assertRaises(hub_mod.InvalidRequest):
            hub_mod.validate_servers(raw)

    def test_bad_names_are_refused(self):
        for name in ("Bad Name", "UPPER", "", "x" * 33, "dots.here", "computer"):
            with self.subTest(name=name):
                self.check_refused([{"name": name, "command": "srv"}])

    def test_duplicate_names_and_too_many_servers(self):
        self.check_refused([{"name": "a", "command": "srv"}, {"name": "a", "command": "srv"}])
        self.check_refused([{"name": f"s{i}", "command": "srv"} for i in range(17)])

    def test_loading_env_keys_are_refused(self):
        for key in ("LD_PRELOAD", "LD_LIBRARY_PATH", "PYTHONPATH", "PATH"):
            with self.subTest(key=key):
                with self.assertRaises(hub_mod.InvalidRequest) as caught:
                    hub_mod.validate_servers([{"name": "a", "command": "srv", "env": {key: "/x"}}])
                self.assertIn(key, str(caught.exception))

    def test_bad_env_keys_values_commands_and_args(self):
        self.check_refused([{"name": "a", "command": "srv", "env": {"1BAD": "x"}}])
        self.check_refused([{"name": "a", "command": "srv", "env": {"OK": 5}}])
        self.check_refused([{"name": "a", "command": "srv", "env": {"OK": "a\0b"}}])
        self.check_refused([{"name": "a", "command": "sh -c x"}])
        self.check_refused([{"name": "a", "command": "srv;rm"}])
        self.check_refused([{"name": "a", "command": "srv", "args": ["ok", 3]}])

    def test_good_entry_passes(self):
        specs = hub_mod.validate_servers([{"name": "ok-1_x", "command": "/usr/bin/node", "args": ["a"], "env": {"K": "v"}}])
        self.assertEqual(specs[0].argv, ("/usr/bin/node", "a"))
        self.assertEqual(specs[0].env, {"K": "v"})

    def test_child_env_pins_path_and_display(self):
        hub = hub_mod.McpHub(display=":99", log_dir="x", config_path="", computer_script=COMPUTER_SCRIPT)
        spec = hub_mod.ServerSpec("a", ("srv",), {"DISPLAY": ":5", "PATH": "/evil", "TOKEN_X": "v"})
        env = hub.child_env(spec)
        self.assertEqual(env["DISPLAY"], ":99")
        self.assertEqual(env["PATH"], hub_mod.CHILD_PATH)
        self.assertEqual(env["TOKEN_X"], "v")


class ComputerServerTests(unittest.TestCase):
    def test_argv_shapes(self):
        self.assertEqual(computer.xdotool_steps("click", {"x": 4, "y": 5, "button": "right"}),
                         [["xdotool", "mousemove", "--", "4", "5", "click", "3"]])
        self.assertEqual(computer.xdotool_steps("double_click", {"x": 1, "y": 2})[0][-3:], ["--repeat", "2", "1"])
        self.assertEqual(computer.xdotool_steps("type", {"text": "-rf"})[0][-2:], ["--", "-rf"])
        self.assertEqual(computer.xdotool_steps("key", {"combo": "ctrl+c"})[0][-1], "ctrl+c")
        steps = computer.xdotool_steps("scroll", {"x": 0, "y": 0, "direction": "down", "amount": 2})
        self.assertEqual(steps[1], ["xdotool", "click", "--repeat", "2", "5"])

    def test_bad_arguments_are_refused(self):
        bad = [
            ("click", {"x": True, "y": 1}),
            ("click", {"x": -1, "y": 1}),
            ("click", {"x": 1, "y": 1, "button": "side"}),
            ("type", {"text": ""}),
            ("type", {"text": "a\0b"}),
            ("key", {"combo": "ctrl+c;rm"}),
            ("scroll", {"x": 1, "y": 1, "direction": "up", "amount": 0}),
            ("scroll", {"x": 1, "y": 1, "direction": "north"}),
            ("nope", {}),
        ]
        for name, args in bad:
            with self.subTest(name=name, args=args):
                with self.assertRaises(computer.ToolError):
                    computer.xdotool_steps(name, args)

    def run_server(self, lines):
        payload = "".join(json.dumps(line) + "\n" for line in lines).encode("utf-8")
        result = subprocess.run(
            [sys.executable, "-I", COMPUTER_SCRIPT], input=payload, capture_output=True, timeout=30,
            env={"PATH": os.environ.get("PATH", ""), "DISPLAY": ":99"},
        )
        return [json.loads(line) for line in result.stdout.decode("utf-8").splitlines() if line.strip()]

    def test_stdio_handshake_and_tools_list(self):
        replies = self.run_server([
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        ])
        self.assertEqual(len(replies), 2)
        self.assertEqual(replies[0]["result"]["protocolVersion"], "2025-06-18")
        names = {tool["name"] for tool in replies[1]["result"]["tools"]}
        self.assertEqual(names, {"screenshot", "click", "double_click", "type", "key", "scroll", "cursor_position"})

    def test_bad_tool_call_is_a_tool_error_not_a_crash(self):
        replies = self.run_server([
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "click", "arguments": {"x": 1, "y": 1, "button": "x"}}},
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "rm", "arguments": {}}},
            {"jsonrpc": "2.0", "id": 5, "method": "ping"},
        ])
        self.assertTrue(replies[0]["result"]["isError"])
        self.assertIn("error", replies[1])
        self.assertEqual(replies[2]["result"], {})


class HttpTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp(prefix="pm-mcp-http-")
        cls.fake = os.path.join(cls.dir, "fake_mcp.py")
        with open(cls.fake, "w", encoding="utf-8") as handle:
            handle.write(FAKE_SERVER)
        cls.settings = control.Settings(
            token=TOKEN, port=0, default_display=":99", launchers=dict(control.DEFAULT_LAUNCHERS),
            focus_wrapper="x", mcp_config=os.path.join(cls.dir, "state", "mcp.json"),
            mcp_log_dir=os.path.join(cls.dir, "logs"),
        )
        cls.hub = control.mcp_hub.McpHub(display=":99", log_dir=cls.settings.mcp_log_dir,
                                 config_path=cls.settings.mcp_config, computer_script=COMPUTER_SCRIPT,
                                 call_timeout=1.0, start_timeout=10)
        cls.hub.start()
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), control.make_handler(cls.settings, cls.hub))
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.hub.close()

    def request(self, method, path, body=None, token=TOKEN, auth=True):
        headers = {"Content-Type": "application/json"}
        if auth:
            headers["Authorization"] = f"Bearer {token}"
        data = None if body is None else json.dumps(body).encode("utf-8")
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                return response.status, json.loads(response.read() or b"null")
        except urllib.error.HTTPError as error:
            return error.code, json.loads(error.read() or b"null")

    def test_unauthorised_requests_are_rejected(self):
        self.assertEqual(self.request("GET", "/v1/mcp/tools", auth=False)[0], 401)
        self.assertEqual(self.request("GET", "/v1/mcp/tools", token="wrong-token-0000000000")[0], 401)
        self.assertEqual(self.request("POST", "/v1/mcp/call", {"server": "computer", "tool": "x"}, auth=False)[0], 401)
        self.assertEqual(self.request("PUT", "/v1/mcp/config", {"servers": []}, auth=False)[0], 401)
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}/v1/mcp/tools",
                                     headers={"Authorization": TOKEN}, method="GET")
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(req, timeout=10)
        self.assertEqual(caught.exception.code, 401)

    def test_tools_list_route_shows_computer(self):
        self.assertTrue(_wait_for(lambda: any(s["name"] == "computer" and s["status"] == "ready"
                                              for s in self.request("GET", "/v1/mcp/tools")[1]["servers"])))
        status, body = self.request("GET", "/v1/mcp/tools")
        self.assertEqual(status, 200)
        self.assertEqual(body["servers"][0]["name"], "computer")

    def test_config_put_validates_and_call_route_runs_a_tool(self):
        self.assertEqual(self.request("PUT", "/v1/mcp/config", {"servers": [{"name": "computer", "command": "x"}]})[0], 400)
        self.ensure_fake_ready()
        status, body = self.request("PUT", "/v1/mcp/config", {"servers": [
            {"name": "fake", "command": "fake-srv-not-installed", "env": {}}]})
        self.assertEqual(status, 200)
        self.assertEqual([s["name"] for s in body["servers"]], ["computer", "fake"])
        status, body = self.request("PUT", "/v1/mcp/config", {"servers": [
            {"name": "fake", "command": "srv", "env": {"LD_PRELOAD": "/x.so"}}]})
        self.assertEqual(status, 400)
        self.assertIn("LD_PRELOAD", body["error"])
        self.ensure_fake_ready()
        status, body = self.request("POST", "/v1/mcp/call",
                                    {"server": "fake", "tool": "echo", "arguments": {"text": "via http"}})
        self.assertEqual(status, 200)
        self.assertEqual(body, {"content": [{"type": "text", "text": "via http"}], "isError": False})
        self.assertEqual(self.request("POST", "/v1/mcp/call", {"server": "fake", "tool": "echo", "arguments": []})[0], 400)
        self.assertEqual(self.request("POST", "/v1/mcp/call", {"server": 3, "tool": "echo"})[0], 400)

    def ensure_fake_ready(self):
        if self.request("GET", "/v1/mcp/tools")[1]["servers"][-1].get("status") != "ready":
            self.hub._install([control.mcp_hub.ServerSpec("fake", (sys.executable, "-I", self.fake), {})])
        self.assertTrue(_wait_for(lambda: self.request("GET", "/v1/mcp/tools")[1]["servers"][-1]["status"] == "ready"))

    def test_hung_call_returns_an_error_instead_of_hanging(self):
        self.ensure_fake_ready()
        started = time.monotonic()
        status, body = self.request("POST", "/v1/mcp/call", {"server": "fake", "tool": "hang", "arguments": {}})
        self.assertEqual(status, 500)
        self.assertIn("timed out", body["error"])
        self.assertLess(time.monotonic() - started, 15)

    def test_config_replace_refuses_computer_name_over_http(self):
        status, body = self.request("PUT", "/v1/mcp/config", {"servers": [{"name": "computer", "command": "x"}]})
        self.assertEqual(status, 400)
        self.assertIn("built in", body["error"])


class ComputerServerNoX(unittest.TestCase):
    def test_screenshot_without_import_is_a_tool_error(self):
        replies = ComputerServerTests().run_server([
            {"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "screenshot", "arguments": {}}},
        ])
        self.assertIn("result", replies[0])
        self.assertIn("isError", replies[0]["result"])


if __name__ == "__main__":
    unittest.main()
