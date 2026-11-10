#!/usr/bin/env python3
# Written for Puppet Master: unit tests for the lend-an-app endpoints and viewer server in pm-sbx-control.
# No X server and no Chrome: the viewer launcher is a fake.
"""Run with: python -I -m unittest test_lend  (from this folder)"""

import base64
import importlib.machinery
import importlib.util
import json
import os
import threading
import time
import unittest
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))


def _load(module_name, filename):
    loader = importlib.machinery.SourceFileLoader(module_name, os.path.join(HERE, filename))
    spec = importlib.util.spec_from_loader(module_name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


control = _load("pm_sbx_control_lend", "pm-sbx-control")
TOKEN = "t0k3n-for-tests-0123456789"
JPEG = b"\xff\xd8\xff\xe0" + b"x" * 64


class FakeProc:
    def __init__(self):
        self.alive, self.terminated = True, False

    def poll(self):
        return None if self.alive else 0

    def terminate(self):
        self.alive, self.terminated = False, True


class HelperTests(unittest.TestCase):
    def test_ids(self):
        for ok in ("a", "line-1", "a" * 32, "0"):
            self.assertTrue(control.lend_valid_id(ok))
        for bad in ("", "A", "a" * 33, "a/b", "a b", "../x", None, 5):
            self.assertFalse(control.lend_valid_id(bad))

    def test_title_is_cleaned_and_capped(self):
        self.assertEqual(control.lend_clean_title("Li\x00ne\n\t\x7f  chat"), "Line chat")
        self.assertEqual(len(control.lend_clean_title("a" * 500)), 80)
        self.assertEqual(control.lend_clean_title("\x01\x02"), "App")
        self.assertEqual(control.lend_clean_title(None), "App")

    def test_event_validation(self):
        clean = control.lend_clean_event
        self.assertEqual(clean({"t": "click", "x": 3.7, "y": 4, "button": "right", "count": 2}),
                         {"t": "click", "x": 3, "y": 4, "button": "right", "count": 2})
        self.assertEqual(clean({"t": "click", "x": 1, "y": 2})["button"], "left")
        self.assertIsNone(clean({"t": "click", "x": 1, "y": 2, "count": 3}))
        self.assertIsNone(clean({"t": "click", "x": 1, "y": 2, "button": "middle"}))
        self.assertIsNone(clean({"t": "click", "x": "1", "y": 2}))
        self.assertIsNone(clean({"t": "click", "x": True, "y": 2}))
        self.assertEqual(clean({"t": "text", "s": "hi"}), {"t": "text", "s": "hi"})
        self.assertIsNone(clean({"t": "text", "s": ""}))
        self.assertIsNone(clean({"t": "text", "s": "x" * 5000}))
        self.assertEqual(clean({"t": "key", "k": "pagedown", "extra": 1}), {"t": "key", "k": "pagedown"})
        self.assertIsNone(clean({"t": "key", "k": "f5"}))
        self.assertEqual(clean({"t": "scroll", "x": 1, "y": 2, "dy": -3}), {"t": "scroll", "x": 1, "y": 2, "dy": -3})
        self.assertIsNone(clean({"t": "scroll", "x": 1, "y": 2}))
        self.assertIsNone(clean({"t": "drag"}))
        self.assertIsNone(clean([]))

    def test_page_escapes_the_title(self):
        page = control.lend_page('</script><b>"x"&', "abc").decode()
        self.assertNotIn("</script><b>", page)
        self.assertIn("document.title=TITLE", page)
        self.assertIn("Waiting for ", page)


class HubTests(unittest.TestCase):
    def make(self):
        procs = []

        def launch(lend_id, title, w, h):
            procs.append((lend_id, title, w, h, FakeProc()))
            return procs[-1][4]

        return control.LendHub(launch), procs

    def test_open_is_idempotent_and_close_stops_the_viewer(self):
        hub, procs = self.make()
        hub.open("a", "A", 10, 20)
        hub.open("a", "A", 10, 20)
        self.assertEqual(len(procs), 1)
        procs[0][4].alive = False  # the viewer window was closed: open again relaunches
        hub.open("a", "A", 10, 20)
        self.assertEqual(len(procs), 2)
        hub.close("a")
        self.assertTrue(procs[1][4].terminated)
        self.assertIsNone(hub.title("a"))

    def test_frames_drop_with_close_and_need_open(self):
        hub, _ = self.make()
        with self.assertRaises(control.RequestError):
            hub.put_frame("zz", 1, 2, 2, JPEG)
        hub.open("a", "A")
        hub.put_frame("a", 2, 4, 5, JPEG)
        hub.put_frame("a", 1, 9, 9, b"old")  # an older seq is ignored
        self.assertEqual(hub.wait_frame("a", 0, 0.01), (2, 4, 5, JPEG))
        self.assertIsNone(hub.wait_frame("a", 2, 0.01))
        hub.close("a")
        self.assertIsNone(hub.wait_frame("a", 0, 0.01))

    def test_long_poll_wakes_on_new_frame(self):
        hub, _ = self.make()
        hub.open("a", "A")
        result = []
        thread = threading.Thread(target=lambda: result.append(hub.wait_frame("a", -1, 5)))
        thread.start()
        time.sleep(0.05)
        hub.put_frame("a", 1, 2, 2, JPEG)
        thread.join(3)
        self.assertEqual(result, [(1, 2, 2, JPEG)])

    def test_long_poll_answers_within_200ms_of_the_stored_frame(self):
        hub, _ = self.make()
        hub.open("a", "A")
        result = []
        waiting = threading.Event()

        def wait():
            waiting.set()
            result.append((hub.wait_frame("a", 3, 10), time.monotonic()))

        thread = threading.Thread(target=wait)
        thread.start()
        waiting.wait(2)
        time.sleep(0.1)  # the waiter is parked on the condition
        hub.put_frame("a", 3, 2, 2, JPEG)  # not newer than after=3: must not wake it with a frame
        hub.put_frame("a", 4, 2, 2, JPEG)
        stored = time.monotonic()
        thread.join(3)
        frame, answered = result[0]
        self.assertEqual(frame, (4, 2, 2, JPEG))
        self.assertLess(answered - stored, 0.2)

    def test_raw_frame_stores_like_the_json_one_with_the_sent_stamp(self):
        hub, _ = self.make()
        hub.open("a", "A")
        headers = {"X-Lend-Id": "a", "X-Seq": "5", "X-W": "30", "X-H": "20", "X-Sent-Ms": "999"}
        self.assertEqual(control.lend_frame_raw(hub, headers, JPEG), {"ok": True})
        self.assertEqual(hub.wait_frame("a", 0, 0.01), (5, 30, 20, JPEG))
        self.assertEqual(hub.sent_ms("a"), 999)
        for bad in ({**headers, "X-Lend-Id": "../x"}, {**headers, "X-Seq": "x"}, {k: v for k, v in headers.items() if k != "X-W"}):
            with self.assertRaises(control.RequestError):
                control.lend_frame_raw(hub, bad, JPEG)
        with self.assertRaises(control.RequestError):
            control.lend_frame_raw(hub, headers, b"notjpeg")
        with self.assertRaises(control.RequestError):
            control.lend_frame_raw(hub, headers, JPEG + b"y" * control.LEND_MAX_FRAME_BYTES)

    def test_events_are_capped_and_drained(self):
        hub, _ = self.make()
        hub.open("a", "A")
        for i in range(250):
            self.assertTrue(hub.push_event("a", {"t": "key", "k": "tab", "n": i}))
        events = hub.drain("a")
        self.assertEqual(len(events), 200)
        self.assertEqual(events[-1]["n"], 249)
        self.assertEqual(hub.drain("a"), [])
        self.assertFalse(hub.push_event("nope", {}))

    def test_drain_long_poll_returns_as_soon_as_an_event_arrives(self):
        hub, _ = self.make()
        hub.open("a", "A")
        result = []
        started = time.monotonic()
        thread = threading.Thread(target=lambda: result.append((hub.drain("a", 5), time.monotonic() - started)))
        thread.start()
        time.sleep(0.1)
        hub.push_event("a", {"t": "key", "k": "tab"})
        thread.join(3)
        events, took = result[0]
        self.assertEqual(events, [{"t": "key", "k": "tab"}])
        self.assertLess(took, 2, "answered when the event arrived, not after the wait")

    def test_drain_long_poll_times_out_empty_and_ends_when_closed(self):
        hub, _ = self.make()
        hub.open("a", "A")
        started = time.monotonic()
        self.assertEqual(hub.drain("a", 0.15), [])
        self.assertGreaterEqual(time.monotonic() - started, 0.1)
        result = []
        thread = threading.Thread(target=lambda: result.append(hub.drain("a", 5)))
        thread.start()
        time.sleep(0.05)
        hub.close("a")
        thread.join(3)
        self.assertEqual(result, [[]])

    def test_frame_capture_stamp_is_kept_for_the_age_header(self):
        hub, _ = self.make()
        hub.open("a", "A")
        control.lend_frame(hub, {"id": "a", "seq": 1, "w": 2, "h": 2, "jpeg_b64": base64.b64encode(JPEG).decode(), "ts": 1234})
        self.assertEqual(hub.captured_ms("a"), 1234)

    def test_dblclick_event_is_validated(self):
        clean = control.lend_clean_event
        self.assertEqual(clean({"t": "dblclick", "x": 3.7, "y": 4}), {"t": "dblclick", "x": 3, "y": 4})
        self.assertIsNone(clean({"t": "dblclick", "x": 3}))

    def test_events_route_wait_is_capped(self):
        hub, _ = self.make()
        hub.open("a", "A")
        started = time.monotonic()
        self.assertEqual(control.lend_events(hub, {"id": "a", "wait_ms": 100}), {"events": []})
        self.assertLess(time.monotonic() - started, 1.5)
        with self.assertRaises(control.RequestError):
            control.lend_events(hub, {"id": "a", "wait_ms": "x"})

    def test_page_sends_clicks_at_once_and_double_as_its_own_event(self):
        page = control.lend_page("T", "a").decode()
        self.assertNotIn("230", page)
        self.assertIn("dblclick", page)


class HttpTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.settings = control.Settings(
            token=TOKEN, port=0, default_display=":99", launchers={}, focus_wrapper="x", lend_view="/nonexistent/lend-view"
        )
        cls.procs = []

        def launch(lend_id, title, w, h):
            cls.procs.append((lend_id, title, w, h))
            return FakeProc()

        cls.lend = control.LendHub(launch)
        hub = type("H", (), {"snapshot": lambda self: {}})()
        cls.api = control.ThreadingHTTPServer(("127.0.0.1", 0), control.make_handler(cls.settings, hub, cls.lend))
        cls.viewer = control.ViewerServer(("127.0.0.1", 0), control.make_viewer_handler(cls.lend))
        for server in (cls.api, cls.viewer):
            threading.Thread(target=server.serve_forever, daemon=True).start()
        cls.api_port, cls.viewer_port = cls.api.server_address[1], cls.viewer.server_address[1]

    @classmethod
    def tearDownClass(cls):
        for server in (cls.api, cls.viewer):
            server.shutdown()
            server.server_close()

    def call(self, port, path, payload=None, auth="Bearer " + TOKEN, method="POST", raw=None):
        headers = {"Content-Type": "application/json"}
        if auth is not None:
            headers["Authorization"] = auth
        data = raw if raw is not None else (json.dumps(payload).encode() if payload is not None else None)
        request = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                return response.status, response.read(), response.headers
        except urllib.error.HTTPError as error:
            with error:
                return error.code, error.read(), error.headers

    def api_post(self, path, payload=None, **kw):
        status, body, _ = self.call(self.api_port, path, payload if payload is not None else {}, **kw)
        return status, json.loads(body or b"{}")

    def frame_payload(self, lend_id="app1", seq=1, jpeg=JPEG):
        return {"id": lend_id, "seq": seq, "w": 640, "h": 480, "jpeg_b64": base64.b64encode(jpeg).decode()}

    def test_all_four_need_the_token(self):
        for path in ("open", "close", "frame", "events"):
            self.assertEqual(self.api_post(f"/v1/lend/{path}", {"id": "a"}, auth=None)[0], 401)
            self.assertEqual(self.api_post(f"/v1/lend/{path}", {"id": "a"}, auth="Bearer wrong")[0], 401)

    def test_bad_ids_and_frames_are_400(self):
        self.assertEqual(self.api_post("/v1/lend/open", {"id": "../x", "title": "t"})[0], 400)
        self.assertEqual(self.api_post("/v1/lend/open", {"id": "A"})[0], 400)
        self.assertEqual(self.api_post("/v1/lend/events", {})[0], 400)
        self.api_post("/v1/lend/open", {"id": "badf", "title": "t", "w": 10, "h": 10})
        self.assertEqual(self.api_post("/v1/lend/frame", self.frame_payload("badf", jpeg=b"notjpeg"))[0], 400)
        bad = self.frame_payload("badf")
        bad["jpeg_b64"] = "!!!"
        self.assertEqual(self.api_post("/v1/lend/frame", bad)[0], 400)
        self.assertEqual(self.api_post("/v1/lend/frame", self.frame_payload("never-opened"))[0], 400)

    def test_frame_body_cap_is_4mb_only_for_frames(self):
        self.api_post("/v1/lend/open", {"id": "big", "title": "t", "w": 10, "h": 10})
        big = JPEG + b"y" * 1_500_000
        self.assertEqual(self.api_post("/v1/lend/frame", self.frame_payload("big", jpeg=big))[0], 200)  # over the 256 KB cap
        self.assertEqual(self.api_post("/v1/lend/open", {"id": "big", "title": "x" * 300_000})[0], 400)  # general cap holds
        huge = {"id": "big", "seq": 9, "w": 1, "h": 1, "jpeg_b64": "A" * (4 * 1024 * 1024 + 10)}
        self.assertEqual(self.api_post("/v1/lend/frame", huge)[0], 400)

    def test_full_round_trip(self):
        status, _ = self.api_post("/v1/lend/open", {"id": "rt1", "title": "My\x00 <App>", "w": 640, "h": 480})
        self.assertEqual(status, 200)
        self.assertEqual(self.procs[-1], ("rt1", "My <App>", 640, 480))
        status, body, _ = self.call(self.viewer_port, "/lend/rt1", method="GET", auth=None)
        self.assertEqual(status, 200)
        self.assertIn(b"My &lt;App&gt;", body)
        self.assertEqual(self.call(self.viewer_port, "/lend/nope", method="GET", auth=None)[0], 404)
        original = control.LEND_POLL_SEC
        control.LEND_POLL_SEC = 0.2
        try:
            self.assertEqual(self.call(self.viewer_port, "/lend/rt1/frame?after=0", method="GET", auth=None)[0], 204)
        finally:
            control.LEND_POLL_SEC = original
        self.assertEqual(self.api_post("/v1/lend/frame", self.frame_payload("rt1", seq=7))[0], 200)
        status, body, headers = self.call(self.viewer_port, "/lend/rt1/frame?after=0", method="GET", auth=None)
        self.assertEqual((status, body, headers["X-Seq"], headers["X-W"], headers["X-H"]), (200, JPEG, "7", "640", "480"))
        ok = {"t": "click", "x": 5, "y": 6, "button": "left", "count": 1}
        self.assertEqual(self.call(self.viewer_port, "/lend/rt1/input", ok, auth=None)[0], 200)
        self.assertEqual(self.call(self.viewer_port, "/lend/rt1/input", {"t": "key", "k": "f5"}, auth=None)[0], 400)
        self.assertEqual(self.call(self.viewer_port, "/lend/rt1/input", raw=b"{not json", auth=None)[0], 400)
        self.assertEqual(self.call(self.viewer_port, "/lend/other/input", ok, auth=None)[0], 404)
        self.assertEqual(self.api_post("/v1/lend/events", {"id": "rt1"}), (200, {"events": [ok]}))
        self.assertEqual(self.api_post("/v1/lend/events", {"id": "rt1"}), (200, {"events": []}))
        self.assertEqual(self.api_post("/v1/lend/close", {"id": "rt1"})[0], 200)
        self.assertEqual(self.call(self.viewer_port, "/lend/rt1", method="GET", auth=None)[0], 404)
        self.assertEqual(self.api_post("/v1/lend/events", {"id": "rt1"}), (200, {"events": []}))

    def raw_post(self, payload, headers=None, auth="Bearer " + TOKEN):
        head = {"Content-Type": "application/octet-stream", "X-Lend-Id": "raw1", "X-Seq": "3", "X-W": "640", "X-H": "480", "X-Sent-Ms": "1700000000123"}
        head.update(headers or {})
        if auth is not None:
            head["Authorization"] = auth
        request = urllib.request.Request(f"http://127.0.0.1:{self.api_port}/v1/lend/frame-raw", data=payload, headers=head, method="POST")
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            with error:
                return error.code, error.read()

    def test_frame_raw_needs_the_token(self):
        self.assertEqual(self.raw_post(JPEG, auth=None)[0], 401)
        self.assertEqual(self.raw_post(JPEG, auth="Bearer wrong")[0], 401)

    def test_frame_raw_stores_and_the_viewer_gets_the_stamp(self):
        self.api_post("/v1/lend/open", {"id": "raw1", "title": "t", "w": 10, "h": 10})
        self.assertEqual(self.raw_post(JPEG), (200, b'{"ok":true}'))
        status, body, headers = self.call(self.viewer_port, "/lend/raw1/frame?after=0", method="GET", auth=None)
        self.assertEqual((status, body, headers["X-Seq"], headers["X-W"], headers["X-H"]), (200, JPEG, "3", "640", "480"))
        self.assertEqual(headers["X-Sent-Ms"], "1700000000123")

    def test_frame_raw_rejects_bad_input_and_caps_the_body_at_4mb(self):
        self.api_post("/v1/lend/open", {"id": "raw2", "title": "t", "w": 10, "h": 10})
        self.assertEqual(self.raw_post(b"notjpeg", {"X-Lend-Id": "raw2"})[0], 400)
        self.assertEqual(self.raw_post(JPEG, {"X-Lend-Id": "Bad Id"})[0], 400)
        self.assertEqual(self.raw_post(JPEG, {"X-Lend-Id": "raw2", "X-Seq": "nope"})[0], 400)
        self.assertEqual(self.raw_post(JPEG, {"X-Lend-Id": "never-opened"})[0], 400)
        self.assertEqual(self.raw_post(JPEG + b"y" * 1_500_000, {"X-Lend-Id": "raw2"})[0], 200)  # past the 256 KB json cap
        try:
            status = self.raw_post(JPEG + b"y" * (4 * 1024 * 1024 + 10), {"X-Lend-Id": "raw2"})[0]
        except OSError:
            status = 400  # the server refused on the length alone and hung up before the client finished sending
        self.assertEqual(status, 400)

    def test_old_json_frame_still_works_and_has_no_sent_stamp(self):
        self.api_post("/v1/lend/open", {"id": "old1", "title": "t", "w": 10, "h": 10})
        self.assertEqual(self.api_post("/v1/lend/frame", self.frame_payload("old1", seq=2))[0], 200)
        _, _, headers = self.call(self.viewer_port, "/lend/old1/frame?after=0", method="GET", auth=None)
        self.assertEqual(headers["X-Sent-Ms"], "0")

    def test_viewer_wakes_at_once_when_a_frame_is_stored(self):
        self.api_post("/v1/lend/open", {"id": "wake1", "title": "t", "w": 10, "h": 10})
        out = []
        thread = threading.Thread(target=lambda: out.append((self.call(self.viewer_port, "/lend/wake1/frame?after=0", method="GET", auth=None)[0], time.monotonic())))
        thread.start()
        time.sleep(0.3)
        stored = time.monotonic()
        self.assertEqual(self.api_post("/v1/lend/frame", self.frame_payload("wake1", seq=1))[0], 200)
        thread.join(5)
        self.assertEqual(out[0][0], 200)
        self.assertLess(out[0][1] - stored, 0.2)

    def test_page_logs_input_to_frame_latency(self):
        page = control.lend_page("T", "a").decode()
        self.assertIn("__lendLatency", page)

    def test_viewer_does_not_serve_control_routes(self):
        self.assertEqual(self.call(self.viewer_port, "/v1/desktop", {"steps": []}, auth=None)[0], 404)


if __name__ == "__main__":
    unittest.main()
