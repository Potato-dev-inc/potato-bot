"""Contract checks for apply-recipe.py's speed work: the mirror pick, the parallel downloads, the progress numbers.

No distro, no network, no root: the pieces are pure functions or run against fakes. The real thing is timed in the sandbox
(see the speed notes in recipes/SCHEMA.md).

Run: python -I -m unittest discover -s . -p "test_*.py"
"""

import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
import subprocess
import sys
import threading
import time
import unittest
import unittest.mock
from datetime import datetime, timedelta, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
RECIPES = os.path.join(HERE, "..", "..", "recipes")


def _load():
    loader = importlib.machinery.SourceFileLoader("apply_recipe", os.path.join(HERE, "apply-recipe.py"))
    spec = importlib.util.spec_from_loader("apply_recipe", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


ar = _load()


def recipe(name):
    with open(os.path.join(RECIPES, name + ".json"), encoding="utf-8") as handle:
        return json.load(handle)


def captured(fn, *args, **kwargs):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        result = fn(*args, **kwargs)
    return result, out.getvalue()


class PrintUrisTests(unittest.TestCase):
    TEXT = (
        "'http://free.nchc.org.tw/ubuntu/pool/main/f/foo/foo_1.0-1_amd64.deb' foo_1.0-1_amd64.deb 12345 SHA256:" + "a" * 64 + "\n"
        "'http://x/ubuntu/pool/main/b/bar/bar_2%3a1.0_all.deb' bar_2%3a1.0_all.deb 99 MD5Sum:abcd\n"
        "WARNING: something\n"
        "'ftp://x/y.deb' y.deb 1 SHA256:" + "b" * 64 + "\n"
        "'http://x/evil' ../evil.deb 1 SHA256:" + "c" * 64 + "\n"
    )

    def test_items_are_read_and_bad_lines_dropped(self):
        items = ar.parse_print_uris(self.TEXT)
        self.assertEqual([i["name"] for i in items], ["foo_1.0-1_amd64.deb", "bar_2%3a1.0_all.deb"])
        self.assertEqual(items[0]["size"], 12345)
        self.assertEqual(items[0]["sha256"], "a" * 64)
        self.assertIsNone(items[1]["sha256"], "only SHA256 is checked here; apt checks the rest itself")

    def test_the_install_count_is_the_inst_lines(self):
        text = "Reading package lists...\nInst libfoo (1.0 Ubuntu:24.04/noble [amd64])\nInst bar (2 x)\nConf libfoo (1.0)\n"
        self.assertEqual(ar.count_installs(text), 2)


class MirrorTests(unittest.TestCase):
    def test_the_list_keeps_only_http_archives_for_amd64(self):
        text = "\n".join([
            "http://free.nchc.org.tw/ubuntu/", "https://mirror.twds.com.tw/ubuntu-ports/", "ftp://old.example/ubuntu/",
            "https://mirror.twds.com.tw/ubuntu", "garbage", "http://free.nchc.org.tw/ubuntu/", ""])
        self.assertEqual(ar.parse_mirror_list(text), ["http://free.nchc.org.tw/ubuntu/", "https://mirror.twds.com.tw/ubuntu/"])

    def test_the_release_date_is_read(self):
        text = "-----BEGIN PGP SIGNED MESSAGE-----\nOrigin: Ubuntu\nSuite: noble-updates\nDate: Sat, 01 Nov 2026 12:30:00 UTC\n"
        self.assertEqual(ar.release_date(text), datetime(2026, 11, 1, 12, 30, tzinfo=timezone.utc))
        self.assertIsNone(ar.release_date("Origin: Ubuntu\n"))

    def test_a_stale_mirror_is_dropped_and_the_fastest_sample_wins(self):
        now = datetime(2026, 11, 1, 12, 0, tzinfo=timezone.utc)
        stage1 = [
            {"base": "A", "latency": 0.05, "date": now - timedelta(days=3)},   # fastest, but three days behind
            {"base": "B", "latency": 0.30, "date": now - timedelta(hours=2)},
            {"base": "C", "latency": 0.10, "date": now - timedelta(hours=1)},
            {"base": "D", "latency": 1.50, "date": now},
        ]
        best, why = ar.choose_best(stage1, {"B": 0.15, "C": 0.40})
        self.assertEqual(best["base"], "B", why)
        best, _ = ar.choose_best(stage1, {})
        self.assertEqual(best["base"], "C", "without samples the first answer decides")
        self.assertIsNone(ar.choose_best([], {})[0])
        self.assertIsNone(ar.choose_best([{"base": "X", "latency": 1, "date": None}], {})[0])

    def test_the_sources_file_is_pointed_at_the_mirror_and_nothing_else_changes(self):
        text = (
            "## header comment\nTypes: deb\nURIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble noble-updates noble-backports\n"
            "Components: main universe restricted multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n"
            "## security\nTypes: deb\nURIs: http://security.ubuntu.com/ubuntu/\nSuites: noble-security\n"
            "Components: main universe restricted multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n"
            "Types: deb\nURIs: https://example.com/other\nSuites: stable\nComponents: main\n")
        new = ar.rewrite_sources(text, "noble", "http://free.nchc.org.tw/ubuntu/", "http://security.ubuntu.com/ubuntu/")
        self.assertIn("URIs: http://free.nchc.org.tw/ubuntu/\nSuites: noble noble-updates noble-backports", new)
        self.assertIn("URIs: http://security.ubuntu.com/ubuntu/\nSuites: noble-security", new)
        self.assertIn("URIs: https://example.com/other", new, "a stanza that is not Ubuntu's is left alone")
        self.assertIn("## header comment", new)
        self.assertIn("Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg", new)
        same_mirror = ar.rewrite_sources(text, "noble", "http://m/ubuntu/", "http://m/ubuntu/")
        self.assertEqual(same_mirror.count("URIs: http://m/ubuntu/"), 2)
        self.assertNotIn("archive.ubuntu.com", same_mirror)

    def test_the_parallel_runner_gives_up_on_a_slow_job(self):
        def fast():
            return "fast"

        def slow():
            time.sleep(3)
            return "slow"

        started = time.monotonic()
        results = ar.run_parallel([fast, slow], 0.4)
        self.assertEqual(results, ["fast"])
        self.assertLess(time.monotonic() - started, 1.5)

    def test_a_job_that_raises_is_not_a_candidate(self):
        def boom():
            raise OSError("down")

        self.assertEqual(ar.run_parallel([boom, lambda: 1], 1.0), [1])


class LookAheadTests(unittest.TestCase):
    def unit(self, key, apt=("a",), pre=(), early=False, session=None, debs=()):
        u = {"key": key, "title": key, "pre": list(pre), "apt": list(apt), "debs": list(debs), "post": []}
        if early:
            u["early_pre"] = True
        if session:
            u["session"] = session
        return u

    def test_a_unit_without_pre_lines_may_start_early(self):
        nxt = self.unit("xfce")
        self.assertIs(ar.lookahead_candidate([nxt, self.unit("coding", pre=["x"])]), nxt)

    def test_post_only_and_session_units_are_passed_over(self):
        post_only = self.unit("pnpm", apt=())
        nxt = self.unit("coding", pre=["curl ..."], early=True)
        self.assertIs(ar.lookahead_candidate([post_only, self.unit("s", apt=(), session="xfce4"), nxt]), nxt)

    def test_pre_lines_without_the_flag_stop_the_chain(self):
        self.assertIsNone(ar.lookahead_candidate([self.unit("zorin", pre=["add-apt-repository"]), self.unit("later")]))
        self.assertIsNone(ar.lookahead_candidate([]))

    def test_the_zorin_look_adds_its_ppa_before_the_one_shared_update(self):
        # A fresh distro has no apt lists when the look's pre lines run, so the first pre line refreshes them itself.
        # The PPA line must come after the tool that adds it is installed.
        pre = recipe("zorin-look")["pre"]
        self.assertTrue("apt-get update" in pre[0] and "apt-get install" in pre[0], "the first pre line refreshes apt lists")
        ppa = [i for i, line in enumerate(pre) if "add-apt-repository" in line and "ppa:zorinos/stable" in line]
        self.assertEqual(len(ppa), 1, "the PPA is added once")
        self.assertTrue(any("apt-get install" in line for line in pre[:ppa[0]]), "the tool that adds the PPA is installed first")

    def test_the_coding_recipe_says_its_pre_lines_may_run_early(self):
        coding = recipe("coding")
        self.assertIs(coding.get("early_pre"), True)
        text = " ".join(coding["pre"])
        for tool in ("install", "curl", "gpg", "echo"):
            self.assertIn(tool, text)  # only tools a stock Ubuntu has
        self.assertNotIn("apt-get", text)

    def test_the_flag_comes_through_the_unit_and_does_not_change_its_digest(self):
        unit = ar.make_unit("coding", "Coding tools", recipe("coding"))
        self.assertTrue(unit["early_pre"])
        plain = {k: v for k, v in unit.items() if k != "early_pre"}
        self.assertEqual(ar.unit_digest(unit, "h"), ar.unit_digest(plain, "h"))
        with self.assertRaises(ar.UsageError):
            ar.check_block({"early_pre": "yes"}, "x")


class RunnerTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.log = os.path.join(self.dir.name, "log", "apply.log")
        self.runner = ar.Runner(False, "/x/assets", self.log)

    def test_stream_hands_over_each_line_logs_it_and_reports_failure(self):
        seen = []
        self.runner.stream([sys.executable, "-c", "print('Unpacking a');print('Setting up a')"], "t", "test", on_line=seen.append)
        self.assertEqual(seen, ["Unpacking a", "Setting up a"])
        with open(self.log, encoding="utf-8") as handle:
            self.assertIn("Setting up a", handle.read())
        with self.assertRaises(ar.StepError):
            self.runner.stream([sys.executable, "-c", "import sys;sys.exit(100)"], "t", "test")

    def test_stream_ticks_while_a_command_is_quiet(self):
        ticks = []
        self.runner.stream([sys.executable, "-c", "import time;time.sleep(0.35)"], "t", "test", on_tick=lambda: ticks.append(1), tick=0.1)
        self.assertGreaterEqual(len(ticks), 2)

    def test_the_dpkg_audit_names_half_installed_and_unconfigured_packages(self):
        audit = (
            "The following packages are in a mess due to serious problems during" + chr(10) +
            "installation.  They must be reinstalled for them (and any packages" + chr(10) +
            "that depend on them) to function properly:" + chr(10) +
            " libxkbregistry0:amd64 library to query available RMLVO" + chr(10) + chr(10) +
            "The following packages have been unpacked but not yet configured." + chr(10) +
            "They must be configured using dpkg --configure or the configure" + chr(10) +
            "menu option in dselect for them to work:" + chr(10) +
            " evince               Document viewer" + chr(10))
        from unittest import mock
        done = subprocess.CompletedProcess([], 0, stdout=audit, stderr="")
        with mock.patch.object(subprocess, "run", return_value=done):
            result = self.runner._dpkg_audit()
        self.assertEqual(result, {"mess": ["libxkbregistry0:amd64"], "unconfigured": ["evince"]})
        with mock.patch.object(subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout="", stderr="")):
            self.assertEqual(self.runner._dpkg_audit(), {"mess": [], "unconfigured": []})

    def test_a_failed_mirror_puts_the_original_sources_back(self):
        sources = os.path.join(self.dir.name, "ubuntu.sources")
        orig = os.path.join(self.dir.name, "orig")
        with open(sources, "w", encoding="utf-8") as handle:
            handle.write("URIs: http://mirror.example/ubuntu/" + chr(10))
        with open(orig, "w", encoding="utf-8") as handle:
            handle.write("URIs: http://archive.ubuntu.com/ubuntu/" + chr(10))
        old = (ar.SOURCES_FILE, ar.SOURCES_ORIG)
        ar.SOURCES_FILE, ar.SOURCES_ORIG = sources, orig
        self.addCleanup(lambda: setattr(ar, "SOURCES_FILE", old[0]) or setattr(ar, "SOURCES_ORIG", old[1]))
        self.assertFalse(self.runner.fall_back_to_archive(), "no mirror was picked: nothing to undo")
        self.runner.mirror = "http://mirror.example/ubuntu/"
        self.assertTrue(self.runner.fall_back_to_archive())
        with open(sources, encoding="utf-8") as handle:
            self.assertIn("archive.ubuntu.com", handle.read())
        self.assertIsNone(self.runner.mirror)
        self.assertFalse(self.runner.fall_back_to_archive(), "only once")


class ReporterTests(unittest.TestCase):
    def lines(self, out):
        rows = []
        for line in out.splitlines():
            parts = line.split(" ", 2)
            n, total = parts[1].split("/")
            rows.append((int(n), int(total), parts[2] if len(parts) > 2 else ""))
        return rows

    def test_the_number_never_goes_down_and_is_1000_only_at_the_end(self):
        clock = [0.0]
        rep = ar.Reporter([10.0, 30.0, 60.0], clock=lambda: clock[0])

        def run():
            rep.update(0.0, "start", True)
            for index, fractions in enumerate([(0.2, 0.9, 1.0), (0.5, 0.4, 1.0), (0.1, 1.0)]):
                rep.begin(index)
                for fraction in fractions:
                    clock[0] += 1.0
                    rep.update(fraction, "x%d %s" % (index, fraction))
            rep.finish("done")

        _, out = captured(run)
        rows = self.lines(out)
        numbers = [r[0] for r in rows]
        self.assertEqual(numbers, sorted(numbers))
        self.assertTrue(all(r[1] == 1000 for r in rows))
        self.assertEqual(rows[-1][:2], (1000, 1000))
        self.assertTrue(all(n < 1000 for n in numbers[:-1]))

    def test_lines_are_not_repeated_or_flooded(self):
        clock = [0.0]
        rep = ar.Reporter([1.0], clock=lambda: clock[0])
        _, out = captured(lambda: [rep.update(0.5, "same"), rep.update(0.5, "same"), rep.update(0.51, "other"),
                                   rep.update(0.52, "forced", True)])
        texts = [r[2] for r in self.lines(out)]
        self.assertEqual(texts, ["same", "forced"], "the same line twice and a line inside the gap are dropped")

    def test_units_weigh_what_they_install_and_a_recipe_can_say_so(self):
        small = {"apt": ["a"], "debs": [], "post": [], "pre": []}
        big = {"apt": ["a"] * 20, "debs": [{"url": "https://x/y.deb"}], "post": ["x"] * 10, "pre": []}
        self.assertLess(ar.unit_weight(small), ar.unit_weight(big))
        self.assertEqual(ar.unit_weight(dict(small, size_mb=123)), 123.0)

    def test_stages_follow_what_the_unit_has(self):
        names = lambda u: [n for n, _ in ar.unit_stages(u)]
        self.assertEqual(names({"pre": [], "apt": ["a"], "debs": [], "post": []}), ["download", "install"])
        self.assertEqual(names({"pre": ["l"], "apt": ["a"], "debs": [], "post": ["p"]}), ["pre", "download", "install", "post"])
        self.assertEqual(names({"pre": [], "apt": [], "debs": [], "post": ["p"]}), ["post"])

    def test_the_unit_text_names_the_unit(self):
        rep = ar.Reporter([1.0])
        up = ar.UnitProgress(rep, "XFCE desktop", ar.unit_stages({"pre": [], "apt": ["a"], "debs": [], "post": []}))
        _, out = captured(lambda: up.at("download", 0.5, "Downloading 1 of 2 MB", True))
        self.assertIn("XFCE desktop: Downloading 1 of 2 MB", out)
        self.assertTrue(out.startswith("PROGRESS "))
        n = int(out.split()[1].split("/")[0])
        self.assertTrue(0 < n < 1000)


class DownloadTests(unittest.TestCase):
    def test_the_line_has_sizes_speed_and_time_left(self):
        mb = 1048576
        text = ar.download_line(120 * mb, 340 * mb, 4.1 * mb)
        self.assertTrue(text.startswith("Downloading 120 of 340 MB"), text)
        self.assertIn("4.1 MB/s", text)
        self.assertIn("about 53 s left", text)
        self.assertEqual(ar.download_line(0, 0, 0), "Downloading")
        self.assertNotIn("left", ar.download_line(1 * mb, 340 * mb, 0))

    def test_the_fetcher_runs_tasks_side_by_side_and_counts_bytes(self):
        started = []
        gate = threading.Barrier(3, timeout=5)

        def make(size):
            def run(on_bytes):
                started.append(size)
                gate.wait()  # three tasks must be running at the same moment
                on_bytes(size)
            return ar.FetchTask("t%d" % size, size, run, required=False)

        fetcher = ar.Fetcher([make(10), make(30), make(20)], workers=3)
        self.assertEqual(fetcher.total, 60)
        fetcher.start()
        seen = []
        ar.wait_fetchers([fetcher], lambda done, total, speed: seen.append((done, total)), interval=0.05)
        self.assertEqual(fetcher.done, 60)
        self.assertEqual(seen[-1], (60, 60))
        self.assertEqual(sorted(started), [10, 20, 30])

    def test_biggest_first_and_a_failed_task_is_recorded_not_raised(self):
        order = []
        lock = threading.Lock()

        def make(name, size, fail=False):
            def run(on_bytes):
                with lock:
                    order.append(name)
                if fail:
                    raise ar.StepError("no route")
                on_bytes(size)
            return ar.FetchTask(name, size, run, required=False)

        tasks = [make("small", 1), make("big", 100, fail=True), make("mid", 10)]
        fetcher = ar.Fetcher(tasks, workers=1)
        fetcher.start()
        ar.wait_fetchers([fetcher], lambda *a: None, interval=0.02)
        self.assertEqual(order, ["big", "mid", "small"])
        self.assertEqual([t.error for t in tasks], [None, "no route", None])

    def test_a_retry_takes_its_bytes_back(self):
        # on_bytes(-n) after a failed attempt keeps the bar honest
        fetcher = ar.Fetcher([], workers=1)
        fetcher._on_bytes(50)
        fetcher._on_bytes(-50)
        self.assertEqual(fetcher.done, 0)


class PlanTests(unittest.TestCase):
    """The dry run prints the commands without root, a distro or a network."""

    def run_plan(self, *extra):
        script = os.path.join(HERE, "apply-recipe.py")
        proc = subprocess.run([sys.executable, "-I", script, "--recipes-dir", RECIPES, "--dry-run", "--recipe", "xfce",
                               "--recipe", "coding", "--opt", "coding.pkg=pnpm"] + list(extra),
                              capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return proc.stdout

    def test_every_apt_install_leaves_out_recommends_and_quiets_the_update_jobs(self):
        out = self.run_plan()
        installs = [l for l in out.splitlines() if "apt-get install" in l]
        self.assertTrue(installs)
        for line in installs:
            self.assertIn("--no-install-recommends", line)
        self.assertIn("mask packagekit, unattended-upgrades", out)
        self.assertIn("choose the fastest nearby Ubuntu mirror", out)
        self.assertLess(out.index("choose the fastest"), out.index("apt-get update"))
        self.assertLess(out.index("mask packagekit"), out.index("apt-get update"))

    def test_chrome_is_its_own_recipe_applied_right_after_base(self):
        self.assertEqual(recipe("base")["debs"], [])
        chrome = recipe("chrome")
        ar.validate_recipe("chrome", chrome)
        self.assertEqual(chrome["requires"], "base")
        self.assertIn("google-chrome-stable_current_amd64.deb", chrome["debs"][0]["url"])
        for lib in ("libnss3", "libgbm1", "libasound2t64", "libatk-bridge2.0-0t64", "libgtk-3-0t64", "xdg-utils",
                    "fonts-liberation", "libvulkan1", "wget"):
            self.assertIn(lib, recipe("base")["apt"])
        self.assertFalse(any("chrome" in line for line in recipe("base")["post"]))
        script = os.path.join(HERE, "apply-recipe.py")
        proc = subprocess.run([sys.executable, "-I", script, "--recipes-dir", RECIPES, "--dry-run",
                               "--recipe", "chrome", "--recipe", "xfce"],
                              capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        out = proc.stdout
        base_install = out.index("xvfb")
        chrome_deb = out.index("google-chrome-stable_current_amd64.deb")
        xfce = out.index("xfce4-session")
        self.assertLess(base_install, chrome_deb)
        self.assertLess(chrome_deb, xfce)

    def test_novnc_is_not_an_apt_dependency_of_the_base(self):
        base = recipe("base")
        self.assertNotIn("novnc", base["apt"])
        out = self.run_plan()
        self.assertIn("pm-sbx-novnc-lite.sh", out)
        self.assertNotRegex(out, r"apt-get install[^\n]* novnc( |$)")

    def test_the_desktop_things_the_spec_needs_are_still_in_the_base(self):
        base = recipe("base")
        for needed in ("xvfb", "x11vnc", "websockify", "xdotool", "imagemagick", "xclip", "fonts-noto-cjk",
                       "fonts-noto-color-emoji", "xterm", "pcmanfm", "mousepad", "evince"):
            self.assertIn(needed, base["apt"])
        xfce = recipe("xfce")
        self.assertIn("xfce4-session", xfce["apt"])

    def test_linux_mint_is_a_cinnamon_desktop(self):
        mint = recipe("mint")
        ar.validate_recipe("mint", mint)
        self.assertEqual(mint["exclusive"], "desktop")
        self.assertEqual(mint["session"], "cinnamon")
        self.assertIn("cinnamon", mint["apt"])
        script = os.path.join(HERE, "apply-recipe.py")
        proc = subprocess.run([sys.executable, "-I", script, "--recipes-dir", RECIPES, "--dry-run", "--recipe", "mint"],
                              capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("PM_SBX_SESSION=cinnamon", proc.stdout)

    def test_a_bad_size_estimate_is_refused(self):
        with self.assertRaises(ar.UsageError):
            ar.check_block({"size_mb": -1}, "x")
        with self.assertRaises(ar.UsageError):
            ar.check_block({"size_mb": "big"}, "x")
        ar.check_block({"size_mb": 12.5}, "x")

    def test_a_changed_estimate_does_not_make_a_finished_step_run_again(self):
        unit = {"key": "k", "title": "t", "pre": [], "apt": ["a"], "debs": [], "post": []}
        self.assertEqual(ar.unit_digest(unit, "h"), ar.unit_digest(dict(unit, size_mb=99), "h"))


class UnitDigestTests(unittest.TestCase):
    """Each unit's skip digest covers its own definition and only the guest files its lines name."""

    def setUp(self):
        import tempfile
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        # Layout like the installed copy: <guest>/desktop (the assets) next to <guest>/egress and <guest>/control.
        self.guest = self.dir.name
        self.assets = os.path.join(self.guest, "desktop")
        for rel, text in {"desktop/pm-no-wslg.sh": "a", "desktop/apply-recipe.py": "x",
                          "egress/pm-sbx-egress.sh": "e", "control/pm-sbx-control": "c"}.items():
            path = os.path.join(self.guest, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(text)

    def write(self, rel, text):
        with open(os.path.join(self.guest, rel), "w", encoding="utf-8") as fh:
            fh.write(text)

    def digest(self, unit):
        return ar.unit_digest(unit, ar.unit_files_digest(unit, self.assets))

    def unit(self, post):
        return {"key": "u", "title": "u", "pre": [], "apt": [], "debs": [], "post": post}

    def test_only_the_files_a_unit_names_change_its_digest(self):
        egress = self.unit(['tr -d "\\r" < "$PM_SBX_GUEST/egress/pm-sbx-egress.sh" > /x'])
        wslg = self.unit(['tr -d "\\r" < "$PM_SBX_ASSETS/pm-no-wslg.sh" > /y'])
        before = (self.digest(egress), self.digest(wslg))
        self.write("control/pm-sbx-control", "changed")
        self.write("desktop/apply-recipe.py", "changed")
        self.assertEqual((self.digest(egress), self.digest(wslg)), before)
        self.write("egress/pm-sbx-egress.sh", "changed")
        self.assertNotEqual(self.digest(egress), before[0])
        self.assertEqual(self.digest(wslg), before[1])

    def test_a_bare_folder_reference_hashes_the_whole_folder(self):
        bare = self.unit(['cp -r "$PM_SBX_GUEST/" /x/'])
        before = self.digest(bare)
        self.write("control/pm-sbx-control", "changed")
        self.assertNotEqual(self.digest(bare), before)

    def test_a_unit_with_no_file_lines_ignores_guest_files(self):
        plain = self.unit(["echo hello"])
        before = self.digest(plain)
        self.write("desktop/pm-no-wslg.sh", "changed")
        self.write("egress/pm-sbx-egress.sh", "changed")
        self.assertEqual(self.digest(plain), before)

    def test_a_changed_definition_reruns_the_unit(self):
        before = self.digest(self.unit(["echo one"]))
        self.assertNotEqual(self.digest(self.unit(["echo two"])), before)

    def test_a_missing_named_file_has_a_digest_and_is_not_an_error(self):
        gone = self.unit(['cat "$PM_SBX_ASSETS/absent.sh"'])
        self.assertTrue(self.digest(gone))
        self.write("desktop/pm-no-wslg.sh", "changed")
        self.assertTrue(self.digest(gone))


class DebRetryTests(unittest.TestCase):
    """A direct .deb download: 3 tries, waits 1 s then 3 s, a half-written file removed, bytes taken back off the bar."""

    def setUp(self):
        import tempfile
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.saved_cache = ar.DEB_CACHE
        ar.DEB_CACHE = os.path.join(self.dir.name, "debs")
        self.addCleanup(setattr, ar, "DEB_CACHE", self.saved_cache)
        self.runner = ar.Runner(False, "/x/assets", os.path.join(self.dir.name, "apply.log"))
        self.deb = {"url": "https://example.test/pkg_1.0_amd64.deb", "sha256": None}
        self.slept = []
        patcher = unittest.mock.patch.object(ar.time, "sleep", side_effect=self.slept.append)
        patcher.start()
        self.addCleanup(patcher.stop)

    def scripted(self, outcomes):
        """A _fetch_once stand-in: each call either raises the next OSError in `outcomes` or writes the file (sha of 'ok')."""
        calls = []

        def fake(runner, url, name, part, count):
            calls.append(part)
            count(7)
            outcome = outcomes.pop(0) if outcomes else None
            if isinstance(outcome, Exception):
                with open(part, "wb") as fh:
                    fh.write(b"half")
                raise outcome
            with open(part, "wb") as fh:
                fh.write(b"ok")
            return "f" * 64, 2

        return fake, calls

    def test_two_failures_then_success_waits_one_then_three_seconds(self):
        fake, calls = self.scripted([OSError("reset"), OSError("timeout")])
        seen = []
        with unittest.mock.patch.object(ar.Runner, "_fetch_once", fake):
            path = self.runner.fetch_deb(self.deb, on_bytes=seen.append)
        self.assertEqual(len(calls), 3)
        self.assertEqual(self.slept, [1.0, 3.0])
        self.assertTrue(os.path.isfile(path))
        self.assertFalse(os.path.exists(path + ".part"))
        self.assertEqual(sum(seen), 7 * 3 - 7 * 2)  # each failed try took its 7 bytes back

    def test_three_failures_give_up_with_a_clear_error_and_no_partial_file(self):
        fake, calls = self.scripted([OSError("a"), OSError("b"), OSError("c")])
        with unittest.mock.patch.object(ar.Runner, "_fetch_once", fake):
            with self.assertRaises(ar.StepError) as caught:
                self.runner.fetch_deb(self.deb)
        self.assertEqual(len(calls), 3)
        self.assertIn("download of", str(caught.exception))
        self.assertEqual(self.slept, [1.0, 3.0])
        self.assertFalse(os.path.exists(os.path.join(ar.DEB_CACHE, "pkg_1.0_amd64.deb.part")))

    def test_a_checksum_mismatch_is_tried_again_too(self):
        pinned = dict(self.deb, sha256="0" * 64)
        calls = []

        def fake(runner, url, name, part, count):
            calls.append(1)
            with open(part, "wb") as fh:
                fh.write(b"bad")
            return "1" * 64, 3

        with unittest.mock.patch.object(ar.Runner, "_fetch_once", fake):
            with self.assertRaises(ar.StepError) as caught:
                self.runner.fetch_deb(pinned)
        self.assertEqual(len(calls), 3)
        self.assertIn("checksum mismatch", str(caught.exception))

    def test_a_redirect_is_not_retried(self):
        def fake(runner, url, name, part, count):
            raise ar.StepError("redirected")

        with unittest.mock.patch.object(ar.Runner, "_fetch_once", fake):
            with self.assertRaises(ar.StepError):
                self.runner.fetch_deb(self.deb)
        self.assertEqual(self.slept, [])


class ImageBuildTests(unittest.TestCase):
    """--image-build: the exported distro carries no apt lists, caches or machine identity, the state file stays."""

    def setUp(self):
        import tempfile
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.root = self.dir.name
        for rel, text in {
            "etc/apt/sources.list.d/ubuntu.sources": "URIs: http://mirror.example/ubuntu/\n",
            "var/lib/pm-sbx/ubuntu.sources.orig": "URIs: http://archive.ubuntu.com/ubuntu/\n",
            "var/lib/apt/lists/archive_InRelease": "list",
            "var/cache/apt/archives/x.deb": "deb",
            "var/cache/pm-sbx/debs/google-chrome.deb": "deb",
            "etc/machine-id": "0123456789abcdef",
            "var/lib/dbus/machine-id": "0123",
            "etc/ssh/ssh_host_rsa_key": "secret",
            "root/.bash_history": "ls",
            "tmp/junk/file": "x",
            "var/log/syslog": "a line of log",
            "var/log/pm-sbx/apply-recipe.log": "a log",
            "var/lib/pm-sbx/recipes-state.json": "{\"base\": \"d\"}",
        }.items():
            path = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(text)

    def read(self, rel):
        with open(os.path.join(self.root, rel), encoding="utf-8") as fh:
            return fh.read()

    def test_the_image_is_cleaned_and_recorded_and_the_state_is_kept(self):
        calls = []
        runner = ar.Runner(False, "/x/assets", os.path.join(self.root, "log.txt"))
        runner.mirror = "http://mirror.example/ubuntu/"
        runner.image_clean("light", ["base", "light"], "abc", root=self.root,
                           run_apt_clean=lambda: calls.append("apt-get clean"))
        self.assertEqual(calls, ["apt-get clean"])
        self.assertEqual(self.read("etc/apt/sources.list.d/ubuntu.sources"), "URIs: http://archive.ubuntu.com/ubuntu/\n")
        self.assertFalse(os.path.exists(os.path.join(self.root, "var/lib/apt/lists/archive_InRelease")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "var/cache/apt/archives/x.deb")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "var/cache/pm-sbx/debs/google-chrome.deb")))
        self.assertEqual(self.read("etc/machine-id"), "")
        self.assertFalse(os.path.exists(os.path.join(self.root, "var/lib/dbus/machine-id")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "etc/ssh/ssh_host_rsa_key")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "root/.bash_history")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "tmp/junk")))
        self.assertEqual(self.read("var/log/syslog"), "")
        self.assertEqual(self.read("var/log/pm-sbx/apply-recipe.log"), "")
        self.assertEqual(json.loads(self.read("etc/pm-sbx/image.json")),
                         {"desktop": "light", "recipes": ["base", "light"], "guest_digest": "abc"})
        self.assertEqual(self.read("var/lib/pm-sbx/recipes-state.json"), "{\"base\": \"d\"}")

    def test_the_desktop_is_the_exclusive_desktop_recipe(self):
        self.assertEqual(ar.desktop_of(RECIPES, ["base", "light"]), "light")
        self.assertEqual(ar.desktop_of(RECIPES, ["base", "chrome"]), None)

    def test_the_dry_run_names_every_clean_step_and_changes_nothing(self):
        script = os.path.join(HERE, "apply-recipe.py")
        proc = subprocess.run([sys.executable, "-I", script, "--recipes-dir", RECIPES, "--dry-run", "--image-build",
                               "--recipe", "light"], capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        for word in ("archive.ubuntu.com", "apt-get clean", "/var/lib/apt/lists/*", "/etc/machine-id", "/etc/pm-sbx/image.json"):
            self.assertIn(word, proc.stdout)


class DpkgConfTests(unittest.TestCase):
    """The dpkg settings for the installs: fast, quiet, and the things a package still needs are kept."""

    def test_the_config_has_the_speed_flags_and_keeps_copyright_and_english(self):
        conf = ar.DPKG_CONF.splitlines()
        self.assertIn("force-unsafe-io", conf)
        for pattern in ("/usr/share/doc/*", "/usr/share/man/*", "/usr/share/info/*", "/usr/share/locale/*"):
            self.assertIn("path-exclude=" + pattern, conf)
        self.assertIn("path-include=/usr/share/doc/*/copyright", conf)
        self.assertIn("path-include=/usr/share/locale/en*", conf)
        self.assertIn("man-db man-db/auto-update boolean false", ar.MAN_DB_SELECTION)

    def test_the_runner_writes_the_file_and_tolerates_a_missing_debconf(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            target = os.path.join(tmp, "dpkg.cfg.d", "90pm-sbx")
            log = os.path.join(tmp, "log", "apply.log")
            saved = ar.DPKG_CONF_FILE
            ar.DPKG_CONF_FILE = target
            self.addCleanup(setattr, ar, "DPKG_CONF_FILE", saved)
            runner = ar.Runner(False, "/x/assets", log)
            real_run = subprocess.run

            def no_debconf(argv, *a, **k):
                if argv and argv[0] == "debconf-set-selections":
                    raise FileNotFoundError("debconf-set-selections")
                return real_run(argv, *a, **k)

            with unittest.mock.patch.object(ar.subprocess, "run", side_effect=no_debconf):
                runner.write_dpkg_conf()
            with open(target, encoding="utf-8") as fh:
                self.assertIn("force-unsafe-io", fh.read())
            with open(log, encoding="utf-8") as fh:
                self.assertIn("debconf", fh.read())

    def test_the_dry_run_says_what_it_would_write(self):
        script = os.path.join(HERE, "apply-recipe.py")
        proc = subprocess.run([sys.executable, "-I", script, "--recipes-dir", RECIPES, "--dry-run", "--recipe", "xfce"],
                              capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("/etc/dpkg/dpkg.cfg.d/90pm-sbx", proc.stdout)
        self.assertLess(proc.stdout.index("dpkg.cfg.d"), proc.stdout.index("apt-get install"))


class FakeRunner:
    """Records what a unit would run; no shell, no apt."""

    def __init__(self):
        self.calls = []
        self.dry_run = False
        self.early = {}
        self.looking = {}
        self.look_lock = threading.Lock()
        self.downloaded = 0

    def shell(self, line, what):
        self.calls.append(("shell", line))

    def apt_update(self, tell=None):
        self.calls.append(("update",))

    def __getattr__(self, name):
        raise AssertionError("unexpected runner use: " + name)


class OneAptUpdateTests(unittest.TestCase):
    """All pending units' pre lines run first; one list refresh follows; a finished unit's pre lines never run."""

    def test_a_unit_that_only_has_pre_lines_does_not_refresh_apt_itself(self):
        runner = FakeRunner()
        unit = {"key": "c", "title": "c", "pre": ["echo a"], "apt": [], "debs": [], "post": []}
        ar.run_unit(runner, unit)
        self.assertEqual(runner.calls, [])

    def test_pre_lines_of_all_pending_units_run_before_one_dry_run_update(self):
        units = [
            {"key": "a", "title": "A", "pre": ["echo a1", "echo a2"], "apt": ["x"], "debs": [], "post": []},
            {"key": "b", "title": "B", "pre": ["echo b1"], "apt": ["y"], "debs": [], "post": []},
        ]
        runner = FakeRunner()
        ar.Runner.run_pre_lines(runner, units)
        self.assertEqual(runner.calls,[("shell", "echo a1"), ("shell", "echo a2"), ("shell", "echo b1")])

    def test_the_dry_run_has_one_update_and_the_pre_lines_come_before_it(self):
        script = os.path.join(HERE, "apply-recipe.py")
        proc = subprocess.run([sys.executable, "-I", script, "--recipes-dir", RECIPES, "--dry-run", "--recipe", "xfce",
                               "--recipe", "coding", "--opt", "coding.pkg=pnpm"],
                              capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stdout.count("apt-get update"), 1)
        self.assertLess(proc.stdout.index("nodesource"), proc.stdout.index("apt-get update"))


class NetworkFallbackTests(unittest.TestCase):
    """A mirror that dies mid-install goes back to archive.ubuntu.com once; a broken package does not switch anything."""
    MIRROR = "http://ftp.mirror.tw/ubuntu/"
    NOVNC_FAIL = [
        "Get:1 http://ftp.mirror.tw/ubuntu noble/main amd64 novnc all 1.3.0-2 [1234 kB]",
        "Err:1 http://ftp.mirror.tw/ubuntu noble/main amd64 novnc all 1.3.0-2",
        "  Could not connect to ftp.mirror.tw:80 (203.0.113.9), connection timed out",
        "E: Failed to fetch http://ftp.mirror.tw/ubuntu/pool/main/n/novnc/novnc_1.3.0-2_all.deb  Could not connect to ftp.mirror.tw:80",
    ]

    def setUp(self):
        import tempfile
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.log = os.path.join(self.dir.name, "log", "apply.log")
        self.sources = os.path.join(self.dir.name, "ubuntu.sources")
        orig = os.path.join(self.dir.name, "orig")
        with open(self.sources, "w", encoding="utf-8") as handle:
            handle.write("URIs: " + self.MIRROR + chr(10))
        with open(orig, "w", encoding="utf-8") as handle:
            handle.write("URIs: http://archive.ubuntu.com/ubuntu/" + chr(10))
        old = (ar.SOURCES_FILE, ar.SOURCES_ORIG)
        ar.SOURCES_FILE, ar.SOURCES_ORIG = self.sources, orig
        self.addCleanup(lambda: setattr(ar, "SOURCES_FILE", old[0]) or setattr(ar, "SOURCES_ORIG", old[1]))
        self.runner = ar.Runner(False, "/x/assets", self.log)
        self.runner.mirror = self.MIRROR

    def fake_popen(self, script, calls):
        """Popen stand-in: script(argv) -> (output lines, exit code) for each apt call, in order."""
        def popen(argv, **kwargs):
            calls.append(list(argv))
            lines, code = script(argv)

            class Proc:
                stdout = iter([line + chr(10) for line in lines])

                def wait(self):
                    return code
            return Proc()
        return popen

    def sources_text(self):
        with open(self.sources, encoding="utf-8") as handle:
            return handle.read()

    def test_network_failure_on_install_switches_to_archive_and_retries_once(self):
        calls = []
        answers = [(self.NOVNC_FAIL, 100), ([], 0), (["Setting up novnc (1.3.0-2) ..."], 0)]

        def script(argv):
            return answers.pop(0)
        with unittest.mock.patch.object(ar.subprocess, "Popen", side_effect=self.fake_popen(script, calls)):
            self.runner.apt_install(["novnc"], "test unit")
        self.assertEqual([c[:2] for c in calls], [["apt-get", "install"], ["apt-get", "update"], ["apt-get", "install"]])
        self.assertIn("archive.ubuntu.com", self.sources_text())
        self.assertNotIn("ftp.mirror.tw", self.sources_text())
        self.assertIsNone(self.runner.mirror)
        with open(self.log, encoding="utf-8") as handle:
            self.assertIn("switching sources to the Ubuntu archive", handle.read())

    def test_a_broken_package_does_not_switch_mirrors(self):
        calls = []
        broken = ["E: Unable to correct problems, you have held broken packages."]

        def script(argv):
            return (broken, 100)
        with unittest.mock.patch.object(ar.subprocess, "Popen", side_effect=self.fake_popen(script, calls)):
            with self.assertRaises(ar.StepError):
                self.runner.apt_install(["novnc"], "test unit")
        self.assertEqual(len(calls), 1, "no retry, no apt-get update")
        self.assertIn("ftp.mirror.tw", self.sources_text())
        self.assertEqual(self.runner.mirror, self.MIRROR)

    def test_a_failed_update_on_the_mirror_switches_once_and_retries(self):
        calls = []
        answers = [(self.NOVNC_FAIL, 100), ([], 0)]

        def script(argv):
            return answers.pop(0)
        with unittest.mock.patch.object(ar.subprocess, "Popen", side_effect=self.fake_popen(script, calls)):
            self.runner.apt_update()
        self.assertEqual(len(calls), 2)
        self.assertIn("archive.ubuntu.com", self.sources_text())

    def test_the_archive_itself_is_never_switched(self):
        self.runner.mirror = ar.ARCHIVE_URL
        calls = []

        def script(argv):
            return (self.NOVNC_FAIL, 100)
        with unittest.mock.patch.object(ar.subprocess, "Popen", side_effect=self.fake_popen(script, calls)):
            with self.assertRaises(ar.StepError):
                self.runner.apt_install(["novnc"], "test unit")
        self.assertEqual(len(calls), 1)
        self.assertIn("ftp.mirror.tw", self.sources_text())


if __name__ == "__main__":
    unittest.main()
