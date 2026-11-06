"""Tests for pm-sbx-limits that need no cgroup: the numbers it writes and the rules it follows, against a temp folder."""
import importlib.machinery
import importlib.util
import os
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load():
    loader = importlib.machinery.SourceFileLoader("pm_sbx_limits", str(HERE / "pm-sbx-limits"))
    spec = importlib.util.spec_from_loader("pm_sbx_limits", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class LimitsTest(unittest.TestCase):
    def setUp(self):
        self.m = load()
        self.tmp = tempfile.TemporaryDirectory()
        self.m.WORK = self.tmp.name
        self.m.cpu_count = lambda: 8
        self.m.mem_total_mb = lambda: 16000

    def tearDown(self):
        self.tmp.cleanup()

    def read(self, name):
        return Path(self.tmp.name, name).read_text()

    def test_cpu_percent_of_the_whole_pc_becomes_a_quota_over_a_100ms_period(self):
        self.m.set_limits(50, 4096)
        self.assertEqual(self.read("cpu.max"), "400000 100000")  # 4 of 8 cpus
        self.m.set_limits(100, 4096)
        self.assertEqual(self.read("cpu.max"), "800000 100000")

    def test_memory_high_is_the_limit_and_max_sits_a_little_above(self):
        cpu, mem, ceiling = self.m.set_limits(50, 4096)
        self.assertEqual((cpu, mem, ceiling), (50, 4096, 4608))
        self.assertEqual(int(self.read("memory.high")), 4096 * 1048576)
        self.assertEqual(int(self.read("memory.max")), 4608 * 1048576)
        self.assertGreater(int(self.read("memory.max")), int(self.read("memory.high")))

    def test_numbers_outside_what_makes_sense_are_kept_inside(self):
        cpu, mem, ceiling = self.m.set_limits(0, 1)
        self.assertEqual((cpu, mem), (5, 512))
        cpu, mem, ceiling = self.m.set_limits(900, 999999)
        self.assertEqual((cpu, mem), (100, 16000))
        self.assertLessEqual(ceiling, 16000)

    def test_status_reads_what_was_written(self):
        self.m.set_limits(25, 2048)
        Path(self.tmp.name, "cgroup.procs").write_text("1\n2\n3\n")
        Path(self.tmp.name, "memory.current").write_text(str(300 * 1048576))
        Path(self.tmp.name, "cpu.stat").write_text("usage_usec 1234\nthrottled_usec 56\n")
        Path(self.tmp.name, "memory.events").write_text("low 0\nhigh 7\nmax 1\noom 0\noom_kill 0\n")
        s = self.m.status()
        self.assertEqual((s["cpu_percent"], s["memory_mb"], s["memory_ceiling_mb"]), (25, 2048, 2304))
        self.assertEqual((s["processes"], s["memory_used_mb"], s["cpu_usage_usec"], s["cpu_throttled_usec"]), (3, 300, 1234, 56))
        self.assertEqual((s["memory_high_events"], s["memory_max_events"], s["oom_kills"]), (7, 1, 0))

    def test_only_root_may_run_it_and_pm_gets_no_command_that_changes_a_limit(self):
        text = (HERE / "pm-sbx-limits").read_text()
        self.assertIn("os.geteuid() != 0", text)
        self.assertEqual(text.count("chown("), 1)  # the one loop that hands pm exactly two cgroup.procs files
        self.assertIn('os.path.join(WORK, "cgroup.procs")', text)
        self.assertNotIn('"cpu.max"), os.chown', text)


if __name__ == "__main__":
    unittest.main()
