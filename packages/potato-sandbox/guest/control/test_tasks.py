#!/usr/bin/env python3
# Written for Puppet Master (not a copy of any upstream test file).
"""Tests for the background tasks of pm-sbx-control (/v1/task/*).

No real sandbox: the process table (/proc), the signals and the spawn are replaced by fakes, and the wrapper that records the
exit code is run in-process against a real throw-away child. Run with:
  python -I -m unittest discover -s <this folder> -p "test_*.py"
"""

import importlib.machinery
import importlib.util
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def _load(module_name, filename):
    path = os.path.join(HERE, filename)
    loader = importlib.machinery.SourceFileLoader(module_name, path)
    spec = importlib.util.spec_from_loader(module_name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


control = _load("pm_sbx_control_tasks", "pm-sbx-control")
TOKEN = "t0k3n-for-tests-0123456789"
POSIX = os.name == "posix"


class FakeWorld:
    """A fake process table: pid -> start ticks. Records the signals sent."""

    def __init__(self):
        self.alive = {}
        self.next_pid = 1000
        self.signals = []
        self.spawned = []
        self.die_on_term = True
        self.clock = 1_000_000

    def spawn(self, argv, cwd, env, log_fd, exit_fd):
        self.next_pid += 1
        pid = self.next_pid
        self.alive[pid] = 5000 + pid
        self.spawned.append({"argv": argv, "cwd": cwd, "env": env, "pid": pid})
        return mock.Mock(pid=pid)

    def ticks(self, pid):
        return self.alive.get(pid)

    def killpg(self, pid, sig):
        self.signals.append((pid, sig))
        if sig == getattr(signal, "SIGKILL", signal.SIGTERM) or self.die_on_term:
            self.alive.pop(pid, None)

    def now(self):
        self.clock += 1
        return self.clock


class TaskTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = os.path.join(self._tmp.name, "tasks")
        self.world = FakeWorld()

    def manager(self, **over):
        args = dict(spawn=self.world.spawn, ticks=self.world.ticks, killpg=self.world.killpg, now=self.world.now,
                    boot_id="boot-1", grace=0.0)
        args.update(over)
        return control.TaskManager(self.dir, **args)

    def start(self, mgr, argv=None, **extra):
        body = {"argv": argv or ["sleep", "100"], "cwd": "/home/pm"}
        body.update(extra)
        return mgr.start(body)["task"]

    def write_exit(self, task_id, code):
        with open(os.path.join(self.dir, task_id, "exit"), "w", encoding="ascii") as handle:
            json.dump({"code": code}, handle)

    def write_log(self, task_id, data):
        with open(os.path.join(self.dir, task_id, "log"), "wb") as handle:
            handle.write(data)


class StartValidationTests(TaskTestCase):
    def test_only_argv_cwd_and_label_are_accepted_so_no_shell_string_gets_in(self):
        mgr = self.manager()
        for body in (
            {"command": "ls | wc", "cwd": "/home/pm"},
            {"argv": ["ls"], "cwd": "/home/pm", "shell": True},
            {"argv": ["ls"], "cwd": "/home/pm", "env": {"A": "b"}},
        ):
            with self.assertRaises(control.RequestError):
                mgr.start(body)
        self.assertEqual(self.world.spawned, [])

    def test_argv_must_be_a_short_list_of_plain_strings(self):
        mgr = self.manager()
        bad = [
            "ls -la", [], [""], ["ls", 3], ["ls", None], ["a\0b"], ["x"] * (control.MAX_ARGV + 1),
            ["x" * (control.MAX_ARG_LEN + 1)], {"0": "ls"},
        ]
        for argv in bad:
            with self.assertRaises(control.RequestError, msg=repr(argv)[:40]):
                mgr.start({"argv": argv, "cwd": "/home/pm"})
        self.assertEqual(self.world.spawned, [])

    def test_cwd_must_be_an_absolute_path_without_dot_dot(self):
        mgr = self.manager()
        for cwd in (None, "", "work", "../etc", "/home/pm/../../etc", "/home/\0pm", 5):
            with self.assertRaises(control.RequestError, msg=repr(cwd)):
                mgr.start({"argv": ["ls"], "cwd": cwd})

    def test_a_label_is_cleaned_and_cut(self):
        mgr = self.manager()
        task = self.start(mgr, label="build\x1b[31m\nit " + "x" * 200)
        self.assertNotIn("\x1b", task["label"])
        self.assertNotIn("\n", task["label"])
        self.assertLessEqual(len(task["label"]), 80)


class StartTests(TaskTestCase):
    def test_a_started_task_is_recorded_with_its_pid_and_start_time(self):
        mgr = self.manager()
        task = self.start(mgr, argv=["python3", "build.py", "--fast"], label="build")
        self.assertEqual(task["state"], "running")
        self.assertRegex(task["id"], r"^t-[0-9a-f]{8}$")
        self.assertEqual(task["argv"], ["python3", "build.py", "--fast"])
        spawned = self.world.spawned[0]
        self.assertEqual(spawned["argv"], ["python3", "build.py", "--fast"])
        self.assertEqual(spawned["cwd"], "/home/pm")
        with open(os.path.join(self.dir, "tasks.json"), encoding="utf-8") as handle:
            row = json.load(handle)["tasks"][0]
        self.assertEqual((row["pid"], row["ticks"], row["boot"]), (spawned["pid"], 5000 + spawned["pid"], "boot-1"))

    def test_the_task_environment_is_built_fresh_and_has_none_of_the_daemons(self):
        mgr = self.manager()
        with mock.patch.dict(os.environ, {"PM_SBX_CONTROL_TOKEN_FILE": "/secret/token", "SECRET_THING": "x"}):
            self.start(mgr)
        env = self.world.spawned[0]["env"]
        self.assertNotIn("PM_SBX_CONTROL_TOKEN_FILE", env)
        self.assertNotIn("SECRET_THING", env)
        self.assertEqual(env["USER"], "pm")
        self.assertIn("/home/pm/.local/bin", env["PATH"])
        self.assertEqual(env["DISPLAY"], control.DEFAULT_DISPLAY)

    def test_at_most_four_tasks_run_at_once(self):
        mgr = self.manager()
        ids = [self.start(mgr)["id"] for _ in range(control.MAX_RUNNING_TASKS)]
        with self.assertRaises(control.RequestError) as caught:
            self.start(mgr)
        self.assertIn("already running", str(caught.exception))
        self.assertEqual(len(self.world.spawned), control.MAX_RUNNING_TASKS)
        mgr.stop({"id": ids[0]})
        self.start(mgr)  # a free slot again

    def test_a_task_that_dies_by_itself_frees_its_slot(self):
        mgr = self.manager()
        first = self.start(mgr)
        for _ in range(3):
            self.start(mgr)
        self.world.alive.pop(self.world.spawned[0]["pid"])
        self.write_exit(first["id"], 0)
        self.start(mgr)

    def test_a_failed_spawn_leaves_nothing_behind(self):
        def broken(*_args):
            raise FileNotFoundError(2, "No such file or directory")

        mgr = self.manager(spawn=broken)
        with self.assertRaises(RuntimeError):
            self.start(mgr)
        self.assertEqual(mgr.status({})["tasks"], [])
        self.assertEqual([n for n in os.listdir(self.dir) if n != "tasks.json"], [])

    def test_old_finished_entries_are_forgotten_beyond_the_table_size(self):
        mgr = self.manager(max_running=1)
        first = self.start(mgr)
        for _ in range(control.MAX_TASK_ENTRIES + 3):
            mgr.stop({"id": mgr.status({})["tasks"][-1]["id"]})
            self.start(mgr)
        ids = [t["id"] for t in mgr.status({})["tasks"]]
        self.assertLessEqual(len(ids), control.MAX_TASK_ENTRIES)
        self.assertNotIn(first["id"], ids)
        self.assertFalse(os.path.exists(os.path.join(self.dir, first["id"])))


class StateTests(TaskTestCase):
    def test_a_task_with_an_exit_file_is_finished_with_its_code(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.world.alive.clear()
        self.write_exit(task["id"], 3)
        got = mgr.status({"id": task["id"]})["task"]
        self.assertEqual((got["state"], got["exitCode"]), ("finished", 3))

    def test_a_task_that_is_gone_without_an_exit_file_is_interrupted_not_finished(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.world.alive.clear()
        got = mgr.status({"id": task["id"]})["task"]
        self.assertEqual(got["state"], "interrupted")
        self.assertNotIn("exitCode", got)

    def test_a_reused_pid_is_not_the_task(self):
        mgr = self.manager()
        task = self.start(mgr)
        pid = self.world.spawned[0]["pid"]
        self.world.alive[pid] = 99  # the kernel gave the same PID to another process, started later
        got = mgr.status({"id": task["id"]})["task"]
        self.assertEqual(got["state"], "interrupted")
        # and stopping it signals nobody
        self.assertEqual(mgr.stop({"id": task["id"]})["task"]["state"], "interrupted")
        self.assertEqual(self.world.signals, [])

    def test_a_reboot_makes_every_old_pid_stale(self):
        first = self.manager()
        task = self.start(first)
        second = self.manager(boot_id="boot-2")
        second.recover()
        self.assertEqual(second.status({"id": task["id"]})["task"]["state"], "interrupted")

    def test_a_running_process_stays_running(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.assertEqual(mgr.status({"id": task["id"]})["task"]["state"], "running")

    def test_status_without_an_id_lists_every_task(self):
        mgr = self.manager()
        a, b = self.start(mgr), self.start(mgr, argv=["x" * 500])
        listing = mgr.status({})
        self.assertEqual([t["id"] for t in listing["tasks"]], [a["id"], b["id"]])
        self.assertLessEqual(len(listing["tasks"][1]["argv"][0]), control.TASK_LIST_ARG_CHARS)
        self.assertEqual(listing["maxRunning"], control.MAX_RUNNING_TASKS)

    def test_unknown_and_malformed_ids_are_refused(self):
        mgr = self.manager()
        for task_id in ("t-00000000", "../x", 5, None, "t-ZZZZZZZZ"):
            for call in (mgr.status, mgr.tail, mgr.stop):
                with self.assertRaises(control.RequestError, msg=repr(task_id)):
                    call({"id": task_id})


class RestartTests(TaskTestCase):
    def test_the_table_survives_a_restart_and_live_tasks_are_adopted(self):
        first = self.manager()
        alive = self.start(first, label="keeps running")
        dead_ok = self.start(first)
        dead_lost = self.start(first)
        self.world.alive.pop(self.world.spawned[1]["pid"])
        self.world.alive.pop(self.world.spawned[2]["pid"])
        self.write_exit(dead_ok["id"], 0)

        second = self.manager()
        second.recover()
        states = {t["id"]: t["state"] for t in second.status({})["tasks"]}
        self.assertEqual(states, {alive["id"]: "running", dead_ok["id"]: "finished", dead_lost["id"]: "interrupted"})
        self.assertEqual(second.status({"id": alive["id"]})["task"]["label"], "keeps running")

    def test_recover_writes_the_new_states_to_disk(self):
        first = self.manager()
        task = self.start(first)
        self.world.alive.clear()
        second = self.manager()
        second.recover()
        with open(os.path.join(self.dir, "tasks.json"), encoding="utf-8") as handle:
            row = json.load(handle)["tasks"][0]
        self.assertEqual((row["id"], row["state"]), (task["id"], "interrupted"))

    def test_a_broken_table_file_means_an_empty_table(self):
        os.makedirs(self.dir, mode=0o700)
        with open(os.path.join(self.dir, "tasks.json"), "w", encoding="utf-8") as handle:
            handle.write("{not json")
        self.assertEqual(self.manager().status({})["tasks"], [])

    def test_a_row_with_a_bad_id_in_the_table_is_ignored(self):
        os.makedirs(self.dir, mode=0o700)
        with open(os.path.join(self.dir, "tasks.json"), "w", encoding="utf-8") as handle:
            json.dump({"tasks": [{"id": "../../etc", "state": "running"}, "x", 3]}, handle)
        self.assertEqual(self.manager().status({})["tasks"], [])


class StopTests(TaskTestCase):
    def test_stop_signals_the_whole_group_with_term(self):
        mgr = self.manager()
        task = self.start(mgr)
        pid = self.world.spawned[0]["pid"]
        got = mgr.stop({"id": task["id"]})["task"]
        self.assertEqual(got["state"], "stopped")
        self.assertEqual(self.world.signals, [(pid, signal.SIGTERM)])

    def test_a_task_that_ignores_term_is_killed_after_the_grace_time(self):
        self.world.die_on_term = False
        mgr = self.manager()
        task = self.start(mgr)
        pid = self.world.spawned[0]["pid"]
        with mock.patch.object(control.signal, "SIGKILL", 9, create=True):  # Windows has no SIGKILL
            got = mgr.stop({"id": task["id"]})["task"]
        self.assertEqual(got["state"], "stopped")
        self.assertEqual([s for _, s in self.world.signals], [signal.SIGTERM, 9])
        self.assertTrue(all(p == pid for p, _ in self.world.signals))

    def test_a_stopped_task_is_not_called_finished_even_if_an_exit_file_was_written(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.write_exit(task["id"], -15)  # the wrapper saw the command die of the TERM
        got = mgr.stop({"id": task["id"]})["task"]
        self.assertEqual(got["state"], "stopped")
        self.assertEqual(got["exitCode"], -15)

    def test_stopping_a_finished_task_signals_nobody(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.world.alive.clear()
        self.write_exit(task["id"], 0)
        self.assertEqual(mgr.stop({"id": task["id"]})["task"]["state"], "finished")
        self.assertEqual(self.world.signals, [])


class TailTests(TaskTestCase):
    def test_tail_returns_the_end_of_the_log(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.write_log(task["id"], b"first line\n" + b"m" * 100 + b"\nlast line\n")
        got = mgr.tail({"id": task["id"], "bytes": 20})["task"]
        self.assertEqual(got["text"], "m" * 9 + "\nlast line\n")
        self.assertTrue(got["cutStart"])
        self.assertEqual(got["logBytes"], 11 + 100 + 11)
        self.assertEqual(got["state"], "running")

    def test_a_small_log_comes_back_whole(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.write_log(task["id"], "héllo\n".encode("utf-8"))
        got = mgr.tail({"id": task["id"]})["task"]
        self.assertEqual(got["text"], "héllo\n")
        self.assertFalse(got["cutStart"])

    def test_tail_size_is_capped_and_validated(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.write_log(task["id"], b"x" * (control.TASK_TAIL_MAX + 500))
        got = mgr.tail({"id": task["id"], "bytes": 10**9})["task"]
        self.assertEqual(len(got["text"]), control.TASK_TAIL_MAX)
        with self.assertRaises(control.RequestError):
            mgr.tail({"id": task["id"], "bytes": "lots"})

    def test_a_missing_log_is_empty_not_an_error(self):
        mgr = self.manager()
        task = self.start(mgr)
        os.unlink(os.path.join(self.dir, task["id"], "log"))
        self.assertEqual(mgr.tail({"id": task["id"]})["task"]["text"], "")

    def test_broken_utf8_in_a_log_is_replaced_not_fatal(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.write_log(task["id"], b"ok \xff\xfe end")
        self.assertIn("end", mgr.tail({"id": task["id"]})["task"]["text"])

    @unittest.skipUnless(POSIX, "needs symbolic links")
    def test_a_log_that_was_swapped_for_a_link_is_not_followed(self):
        mgr = self.manager()
        task = self.start(mgr)
        secret = os.path.join(self._tmp.name, "secret.txt")
        with open(secret, "w", encoding="utf-8") as handle:
            handle.write("top secret")
        log = os.path.join(self.dir, task["id"], "log")
        os.unlink(log)
        os.symlink(secret, log)
        self.assertEqual(mgr.tail({"id": task["id"]})["task"]["text"], "")

    @unittest.skipUnless(POSIX, "needs fifos")
    def test_a_log_that_is_not_a_plain_file_is_not_read(self):
        mgr = self.manager()
        task = self.start(mgr)
        log = os.path.join(self.dir, task["id"], "log")
        os.unlink(log)
        os.mkfifo(log)
        self.assertEqual(mgr.tail({"id": task["id"]})["task"]["text"], "")


class TaskFolderSafetyTests(TaskTestCase):
    """The folder is root's, outside anything the pm user can write. If it is not what it should be, nothing is written."""

    def assertOff(self, mgr, action="start"):
        with self.assertRaisesRegex(RuntimeError, "background tasks are off: the task folder is not safe"):
            if action == "start":
                self.start(mgr)
            else:
                mgr.status({})
        self.assertEqual(self.world.spawned, [])

    def test_the_default_folder_depends_on_who_runs_the_daemon(self):
        self.assertEqual(control.default_task_dir(True), "/var/lib/pm-sbx-tasks")
        self.assertFalse(control.default_task_dir(True).startswith("/var/lib/pm-sbx/"))
        self.assertEqual(control.default_task_dir(False), "/var/lib/pm-sbx/tasks")
        self.assertEqual(control.DEFAULT_TASK_DIR, control.default_task_dir(control.RUNS_AS_ROOT))

    @unittest.skipUnless(POSIX and os.geteuid() != 0, "the pm case: a daemon that is not root")
    def test_tasks_work_when_the_daemon_is_pm_with_the_default_folder_logic(self):
        # The folder is made by the daemon inside a pm-owned parent, the way /var/lib/pm-sbx/tasks is.
        parent = os.path.join(self._tmp.name, "pm-sbx")
        os.mkdir(parent, 0o700)
        mgr = control.TaskManager(os.path.join(parent, "tasks"), spawn=self.world.spawn, ticks=self.world.ticks,
                                  killpg=self.world.killpg, now=self.world.now, boot_id="boot-1", grace=0.0)
        task = self.start(mgr)
        self.assertEqual(task["state"], "running")
        self.assertEqual(os.stat(os.path.join(parent, "tasks")).st_mode & 0o777, 0o700)
        self.assertEqual(os.stat(os.path.join(parent, "tasks")).st_uid, os.geteuid())
        self.assertIn("task", mgr.status({"id": task["id"]}))

    def test_a_missing_folder_is_created_private_and_works(self):
        task = self.start(self.manager())
        self.assertEqual(task["state"], "running")
        self.assertTrue(os.path.isdir(self.dir))
        if POSIX:
            self.assertEqual(os.stat(self.dir).st_mode & 0o777, 0o700)

    def test_a_file_in_place_of_the_folder_is_refused(self):
        with open(self.dir, "w", encoding="utf-8") as handle:
            handle.write("x")
        self.assertOff(self.manager())
        self.assertOff(self.manager(), "status")

    @unittest.skipUnless(POSIX, "needs symlinks and unix permissions")
    def test_a_folder_that_is_a_link_is_refused_and_nothing_is_written_through_it(self):
        elsewhere = os.path.join(self._tmp.name, "elsewhere")
        os.mkdir(elsewhere, 0o700)
        os.symlink(elsewhere, self.dir)
        self.assertOff(self.manager())
        self.assertOff(self.manager(), "status")
        self.assertEqual(os.listdir(elsewhere), [])

    @unittest.skipUnless(POSIX, "needs unix permissions")
    def test_a_folder_owned_by_another_user_is_refused(self):
        os.mkdir(self.dir, 0o700)
        with mock.patch.object(control.os, "geteuid", return_value=os.geteuid() + 1):
            self.assertOff(self.manager())
        self.assertEqual(os.listdir(self.dir), [])

    @unittest.skipUnless(POSIX and hasattr(os, "chown") and os.geteuid() == 0, "needs root to hand the folder to another uid")
    def test_a_folder_really_chowned_to_another_uid_is_refused(self):
        os.mkdir(self.dir, 0o700)
        os.chown(self.dir, 12345, 12345)
        self.assertOff(self.manager())
        self.assertEqual(os.listdir(self.dir), [])

    @unittest.skipUnless(POSIX, "needs unix permissions")
    def test_a_folder_open_to_group_or_others_is_refused(self):
        os.mkdir(self.dir, 0o700)
        os.chmod(self.dir, 0o750)
        self.assertOff(self.manager())
        os.chmod(self.dir, 0o707)
        self.assertOff(self.manager())
        self.assertEqual(os.listdir(self.dir), [])

    @unittest.skipUnless(POSIX, "needs unix permissions")
    def test_a_parent_that_others_can_write_is_refused(self):
        os.chmod(self._tmp.name, 0o777)
        self.assertOff(self.manager())
        self.assertFalse(os.path.exists(self.dir))

    @unittest.skipUnless(POSIX and os.geteuid() != 0, "root may sit under root-owned folders")
    def test_a_parent_owned_by_another_user_is_refused(self):
        with mock.patch.object(control.os, "geteuid", return_value=os.geteuid() + 4242):
            self.assertOff(self.manager())
        self.assertFalse(os.path.exists(self.dir))

    @unittest.skipUnless(POSIX, "needs symlinks")
    def test_a_link_planted_as_the_next_run_folder_is_not_followed(self):
        victim = os.path.join(self._tmp.name, "victim")
        os.mkdir(victim, 0o700)
        os.mkdir(self.dir, 0o700)
        os.symlink(victim, os.path.join(self.dir, "t-01020304"))
        picks = iter([bytes([1, 2, 3, 4]), bytes([5, 6, 7, 8])])
        with mock.patch.object(control.os, "urandom", side_effect=lambda n: next(picks)):
            task = self.start(self.manager())
        self.assertEqual(task["id"], "t-05060708")
        self.assertEqual(os.listdir(victim), [])

    @unittest.skipUnless(POSIX, "needs symlinks")
    def test_a_link_planted_as_the_table_or_its_temp_file_is_not_followed(self):
        victim = os.path.join(self._tmp.name, "victim.txt")
        with open(victim, "w", encoding="utf-8") as handle:
            handle.write("keep me")
        os.mkdir(self.dir, 0o700)
        os.symlink(victim, os.path.join(self.dir, "tasks.json.tmp"))
        os.symlink(victim, os.path.join(self.dir, "tasks.json"))
        mgr = self.manager()
        self.assertEqual(mgr.status({})["tasks"], [])  # the linked table is not read
        self.start(mgr)
        with open(victim, encoding="utf-8") as handle:
            self.assertEqual(handle.read(), "keep me")
        self.assertFalse(os.path.islink(os.path.join(self.dir, "tasks.json")))
        self.assertFalse(os.path.exists(os.path.join(self.dir, "tasks.json.tmp")))

    @unittest.skipUnless(POSIX, "needs symlinks")
    def test_a_link_in_place_of_a_log_is_not_read(self):
        mgr = self.manager()
        task = self.start(mgr)
        secret = os.path.join(self._tmp.name, "secret.txt")
        with open(secret, "w", encoding="utf-8") as handle:
            handle.write("top secret")
        log = os.path.join(self.dir, task["id"], "log")
        os.unlink(log)
        os.symlink(secret, log)
        self.assertEqual(mgr.tail({"id": task["id"]})["task"]["text"], "")

    @unittest.skipUnless(POSIX, "needs symlinks")
    def test_a_link_in_place_of_a_run_folder_is_not_read(self):
        mgr = self.manager()
        task = self.start(mgr)
        elsewhere = os.path.join(self._tmp.name, "elsewhere")
        os.mkdir(elsewhere)
        with open(os.path.join(elsewhere, "log"), "w", encoding="utf-8") as handle:
            handle.write("not yours")
        run = os.path.join(self.dir, task["id"])
        os.unlink(os.path.join(run, "log"))
        os.unlink(os.path.join(run, "exit"))
        os.rmdir(run)
        os.symlink(elsewhere, run)
        self.assertEqual(mgr.tail({"id": task["id"]})["task"]["text"], "")

    def test_start_status_tail_and_stop_still_work_in_a_safe_folder(self):
        mgr = self.manager()
        task = self.start(mgr)
        self.write_log(task["id"], b"hi\n")
        self.assertEqual(mgr.status({"id": task["id"]})["task"]["state"], "running")
        self.assertEqual(mgr.tail({"id": task["id"]})["task"]["text"], "hi\n")
        self.assertEqual(mgr.stop({"id": task["id"]})["task"]["state"], "stopped")
        self.assertEqual(len(mgr.status({})["tasks"]), 1)

    def test_recover_in_an_unsafe_folder_does_not_raise_and_writes_nothing(self):
        with open(self.dir, "w", encoding="utf-8") as handle:
            handle.write("x")
        self.manager().recover()
        with open(self.dir, encoding="utf-8") as handle:
            self.assertEqual(handle.read(), "x")


class RealSpawnTests(TaskTestCase):
    def test_the_command_runs_in_its_own_session_behind_the_wrapper_with_no_shell(self):
        mgr = control.TaskManager(self.dir, boot_id="b", ticks=lambda pid: 1, now=self.world.now, python="/usr/bin/python3")
        with mock.patch.object(control.subprocess, "Popen") as popen, mock.patch.object(control.threading, "Thread"):
            popen.return_value.pid = 4242
            self.start(mgr, argv=["bash", "script.sh", "a b"])
        command = popen.call_args.args[0]
        kwargs = popen.call_args.kwargs
        self.assertEqual(command[:3], ["/usr/bin/python3", "-I", "-c"])
        self.assertEqual(command[3], control.TASK_WRAPPER_SRC)
        self.assertEqual(command[-3:], ["bash", "script.sh", "a b"])
        self.assertNotIn("shell", kwargs)
        self.assertTrue(kwargs["start_new_session"])
        self.assertEqual(kwargs["cwd"], "/home/pm")
        self.assertIs(kwargs["stdin"], subprocess.DEVNULL)
        self.assertEqual(len(kwargs["pass_fds"]), 2)

    def test_when_the_daemon_is_root_the_task_runs_as_the_pm_user_without_extra_groups(self):
        with mock.patch.object(control.mcp_hub, "_pm_ids", return_value=(1001, 1001)):
            mgr = control.TaskManager(self.dir, boot_id="b", ticks=lambda pid: 1, now=self.world.now)
        with mock.patch.object(control.subprocess, "Popen") as popen, mock.patch.object(control.threading, "Thread"), \
                mock.patch.object(control.os, "fchown", create=True) as chown:
            popen.return_value.pid = 4242
            self.start(mgr)
        kwargs = popen.call_args.kwargs
        self.assertEqual((kwargs["user"], kwargs["group"], kwargs["extra_groups"]), (1001, 1001, []))
        self.assertEqual(chown.call_count, 2)  # the log and the exit file belong to pm
        self.assertTrue(all(c.args[1:] == (1001, 1001) for c in chown.call_args_list))

    def test_the_real_popen_is_the_one_that_puts_children_in_the_work_cgroup(self):
        # The daemon replaces subprocess.Popen at import time; tasks use it by name, so they join the work cgroup too.
        self.assertTrue(issubclass(control.subprocess.Popen, control._RealPopen))
        self.assertIsNot(control.subprocess.Popen, control._RealPopen)


class WrapperTests(unittest.TestCase):
    """The wrapper source, run in-process with real files and a real (python) child."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.log = os.path.join(self._tmp.name, "log")
        self.exit = os.path.join(self._tmp.name, "exit")
        flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_BINARY", 0)
        self.log_fd = os.open(self.log, flags, 0o600)
        self.exit_fd = os.open(self.exit, flags, 0o600)
        self.addCleanup(self._close)
        self.ns = {"__name__": "wrapper_under_test"}
        exec(compile(control.TASK_WRAPPER_SRC, "wrapper", "exec"), self.ns)

    def _close(self):
        for fd in (self.log_fd, self.exit_fd):
            try:
                os.close(fd)
            except OSError:
                pass

    def run_wrapper(self, argv, cap=control.TASK_LOG_CAP, keep=control.TASK_LOG_KEEP):
        return self.ns["main"]([str(self.log_fd), str(self.exit_fd), str(cap), str(keep), *argv])

    def read(self, path):
        with open(path, "rb") as handle:
            return handle.read()

    def test_output_goes_to_the_log_and_the_exit_code_to_the_exit_file(self):
        self.run_wrapper([sys.executable, "-c", "import sys; print('out'); print('err', file=sys.stderr); sys.exit(3)"])
        text = self.read(self.log).decode().replace("\r\n", "\n")
        self.assertIn("out\n", text)
        self.assertIn("err\n", text)
        self.assertEqual(json.loads(self.read(self.exit)), {"code": 3})

    def test_a_clean_exit_is_code_zero(self):
        self.run_wrapper([sys.executable, "-c", "print('hi')"])
        self.assertEqual(json.loads(self.read(self.exit)), {"code": 0})

    def test_a_program_that_does_not_exist_is_127_with_a_reason_in_the_log(self):
        self.run_wrapper(["/no/such/program-xyz"])
        self.assertEqual(json.loads(self.read(self.exit)), {"code": 127})
        self.assertIn(b"could not start", self.read(self.log))

    def test_the_log_never_grows_past_its_cap_and_keeps_the_newest_output(self):
        script = "import sys\nfor i in range(400):\n    sys.stdout.write('line %04d\\n' % i)\n    sys.stdout.flush()\n"
        self.run_wrapper([sys.executable, "-c", script], cap=1000, keep=300)
        data = self.read(self.log)
        self.assertLessEqual(len(data), 1000)
        text = data.decode().replace("\r\n", "\n")
        self.assertTrue(text.rstrip().endswith("line 0399"), text[-60:])
        self.assertIn("earlier output dropped", text)
        self.assertEqual(json.loads(self.read(self.exit)), {"code": 0})

    def test_the_command_gets_no_input(self):
        self.run_wrapper([sys.executable, "-c", "import sys; print(repr(sys.stdin.read()))"])
        self.assertIn("''", self.read(self.log).decode())

    @unittest.skipUnless(POSIX, "signals")
    def test_a_command_killed_by_a_signal_is_recorded_as_minus_the_signal(self):
        self.run_wrapper([sys.executable, "-c", "import os, signal; os.kill(os.getpid(), signal.SIGKILL)"])
        self.assertEqual(json.loads(self.read(self.exit)), {"code": -9})

    @unittest.skipUnless(POSIX, "signals")
    def test_the_wrapper_survives_the_term_that_stop_sends_to_the_whole_group(self):
        before = signal.getsignal(signal.SIGTERM)
        self.addCleanup(signal.signal, signal.SIGTERM, before)
        self.run_wrapper([sys.executable, "-c", "print('x')"])
        handler = signal.getsignal(signal.SIGTERM)
        self.assertTrue(callable(handler))  # a handler, not SIG_IGN: the command does not inherit an ignored signal
        self.assertNotIn(handler, (signal.SIG_IGN, signal.SIG_DFL))


class HttpTaskTests(TaskTestCase):
    def setUp(self):
        super().setUp()
        self.settings = control.Settings(
            token=TOKEN, port=0, default_display=":99", launchers=dict(control.DEFAULT_LAUNCHERS),
            focus_wrapper="/x", task_dir=self.dir,
        )
        self.mgr = self.manager()
        hub = mock.Mock()
        lend = mock.Mock()
        self.server = control.ThreadingHTTPServer(
            ("127.0.0.1", 0), control.make_handler(self.settings, hub, lend, self.mgr)
        )
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def post(self, path, body, token=TOKEN):
        request = urllib.request.Request(
            self.url + path, data=json.dumps(body).encode(), method="POST",
            headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {token}"} if token else {})},
        )
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                return response.status, json.loads(response.read())
        except urllib.error.HTTPError as error:
            return error.code, json.loads(error.read())

    def test_the_routes_need_the_token(self):
        for route in ("start", "status", "tail", "stop"):
            self.assertEqual(self.post(f"/v1/task/{route}", {}, token=None)[0], 401)
            self.assertEqual(self.post(f"/v1/task/{route}", {}, token="wrong-token-value-123")[0], 401)
        self.assertEqual(self.world.spawned, [])

    def test_start_status_tail_and_stop_over_http(self):
        status, answer = self.post("/v1/task/start", {"argv": ["sleep", "5"], "cwd": "/home/pm"})
        self.assertEqual(status, 200)
        task_id = answer["task"]["id"]
        self.write_log(task_id, b"hello\n")
        self.assertEqual(self.post("/v1/task/status", {"id": task_id})[1]["task"]["state"], "running")
        self.assertEqual(self.post("/v1/task/tail", {"id": task_id})[1]["task"]["text"], "hello\n")
        self.assertEqual(self.post("/v1/task/stop", {"id": task_id})[1]["task"]["state"], "stopped")
        self.assertEqual(len(self.post("/v1/task/status", {})[1]["tasks"]), 1)

    def test_bad_requests_are_400_and_a_shell_string_is_refused(self):
        self.assertEqual(self.post("/v1/task/start", {"command": "rm -rf /", "cwd": "/home/pm"})[0], 400)
        self.assertEqual(self.post("/v1/task/start", {"argv": "ls", "cwd": "/home/pm"})[0], 400)
        self.assertEqual(self.post("/v1/task/status", {"id": "nope"})[0], 400)
        self.assertEqual(self.world.spawned, [])


class SettingsTests(unittest.TestCase):
    def test_the_task_folder_comes_from_the_environment_or_a_default(self):
        with tempfile.NamedTemporaryFile("w", delete=False) as handle:
            handle.write(TOKEN)
        self.addCleanup(os.unlink, handle.name)
        base = {"PM_SBX_CONTROL_TOKEN_FILE": handle.name}
        self.assertEqual(control.load_settings(base).task_dir, control.DEFAULT_TASK_DIR)
        self.assertEqual(control.load_settings({**base, "PM_SBX_TASK_DIR": "/tmp/t"}).task_dir, "/tmp/t")

    def test_ticks_reader_counts_fields_after_the_last_parenthesis(self):
        fields = ["S"] + ["0"] * 18 + ["12345"] + ["0"] * 5
        raw = ("42 (a) b ) c) " + " ".join(fields)).encode()
        with mock.patch("builtins.open", mock.mock_open(read_data=raw)):
            self.assertEqual(control.proc_start_ticks(42), 12345)
        zombie = ("42 (x) Z " + " ".join(["0"] * 30)).encode()
        with mock.patch("builtins.open", mock.mock_open(read_data=zombie)):
            self.assertIsNone(control.proc_start_ticks(42))
        with mock.patch("builtins.open", side_effect=FileNotFoundError):
            self.assertIsNone(control.proc_start_ticks(42))


if __name__ == "__main__":
    unittest.main()
