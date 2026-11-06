"""Checks for the egress probe's internet check, and the boot hook's offline verdict.

The internet check is cut out of pm-sbx-egress-probe.sh and run in bash with a fake curl, so no network is used.
The boot hook is pinned by reading it: exit 3 from the probe must become reason=offline in boot.failed.

Run: python -I -m unittest discover -s . -p "test_*.py"
"""

import os
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PROBE = os.path.join(HERE, "pm-sbx-egress-probe.sh")
BOOT = os.path.join(HERE, "..", "desktop", "pm-sbx-boot.sh")
BASH = shutil.which("bash")


def read(path):
    with open(path, encoding="utf-8", newline="") as handle:
        return handle.read()


def function(text, name):
    found = re.search(r"^%s\(\) \{[^\n]*\n.*?^\}\n" % re.escape(name), text, re.S | re.M)
    if not found:
        raise AssertionError(f"{name} is not in the probe")
    return found.group(0)


FAKE_CURL = """#!/bin/bash
url="${@: -1}"
echo "$url" >>"$CALLS"
for ok in $FAKE_OK; do
  if [ "$url" = "https://$ok/" ]; then exit 0; fi
done
echo "curl: (6) Could not resolve host" >&2
exit 6
"""


def posix(path):
    return path.replace("\\", "/")


@unittest.skipUnless(BASH, "bash is not installed")
class InternetCheck(unittest.TestCase):
    def run_check(self, ok_hosts):
        folder = tempfile.mkdtemp(prefix="probe-net-")
        self.addCleanup(shutil.rmtree, folder, True)
        curl = os.path.join(folder, "fake-curl.sh")
        with open(curl, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(FAKE_CURL)
        calls = os.path.join(folder, "calls.txt")
        status_dir = os.path.join(folder, "status")
        os.makedirs(status_dir)
        text = read(PROBE)
        harness = "\n".join(
            [
                'log() { echo "pm-sbx-egress-probe: $*" >&2; }',
                'tcp_connect() { return 1; }',
                'STATUS_DIR="$PM_TEST_STATUS"',
                'PROBE_STATUS="$STATUS_DIR/egress-probe.status"',
                'CURL="$PM_TEST_CURL"',
                # the real hosts and rounds, but no real pause
                "INTERNET_HOSTS=(example.com www.cloudflare.com www.microsoft.com)",
                "INTERNET_ROUNDS=3",
                "INTERNET_PAUSE=0",
                function(text, "write_probe_status"),
                function(text, "fail_offline"),
                function(text, "check_internet"),
                "check_internet",
                'echo "PASSED"',
            ]
        )
        env = dict(os.environ, PM_TEST_STATUS=posix(status_dir), PM_TEST_CURL=posix(curl), CALLS=posix(calls), FAKE_OK=" ".join(ok_hosts))
        result = subprocess.run([BASH, "-c", harness], capture_output=True, text=True, timeout=60, env=env)
        made = read(calls).splitlines() if os.path.exists(calls) else []
        status = read(os.path.join(status_dir, "egress-probe.status")) if os.path.exists(os.path.join(status_dir, "egress-probe.status")) else ""
        return result, made, status

    def test_the_first_host_that_answers_passes_without_trying_the_rest(self):
        result, calls, status = self.run_check(["www.cloudflare.com"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASSED", result.stdout)
        self.assertEqual(calls, ["https://example.com/", "https://www.cloudflare.com/"])
        self.assertEqual(status, "")

    def test_the_last_host_alone_is_enough_and_it_takes_every_try(self):
        result, calls, _ = self.run_check(["www.microsoft.com"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 3)

    def test_no_host_answering_in_three_rounds_is_offline_exit_3(self):
        result, calls, status = self.run_check([])
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(len(calls), 9, "three hosts, three rounds")
        self.assertIn("state=failed\n", status)
        self.assertIn("reason=offline\n", status)
        self.assertIn("offline", result.stderr)

    def test_the_production_settings_are_three_hosts_three_rounds_two_seconds(self):
        text = read(PROBE)
        self.assertIn("INTERNET_HOSTS=(example.com www.cloudflare.com www.microsoft.com)", text)
        self.assertIn("INTERNET_ROUNDS=3", text)
        self.assertIn("INTERNET_PAUSE=2", text)


class BootVerdict(unittest.TestCase):
    def test_the_probe_exit_3_becomes_reason_offline_and_other_failures_keep_their_text(self):
        boot = read(BOOT)
        self.assertIn('if [ "$step" = pm-sbx-egress-probe.sh ] && [ "$rc" -eq 3 ]; then fail offline; fi', boot)
        self.assertIn('fail "$step failed (exit $rc)"', boot)

    def test_the_probe_uses_exit_3_for_offline(self):
        self.assertIn("exit 3", function(read(PROBE), "fail_offline"))


if __name__ == "__main__":
    unittest.main()
