"""Tests of the runner itself, without cargo: a small program stands in
for a cargo run. Run with `python3 -m unittest discover -s mutation`."""

import contextlib
import io
import os
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import run  # noqa: E402

# What `cargo test --no-fail-fast` prints when the first test binary has
# a test that fails and other binaries run after it: the name and the
# cause of the failure come first, and more than 3000 characters follow,
# on stdout and on stderr. A line end of Windows and a byte that is not
# UTF-8 are among them, as a test may print.
CARGO = r'''
import sys
out, err = sys.stdout.buffer, sys.stderr.buffer
err.write(b"   Compiling monolith-core v0.1.0\n")
err.write(b"    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.00s\n")
err.write(b"     Running tests/early.rs (target/debug/deps/early-0123)\n")
out.write(b"\nrunning 2 tests\n")
out.write(b"test early_passing ... ok\n")
out.write(b"test early_failing ... FAILED\n\n")
out.write(b"failures:\n\n---- early_failing stdout ----\n")
out.write(b"thread 'early_failing' panicked at tests/early.rs:7:5:\n")
out.write(b"the early cause\r\n\xff\n\n")
out.write(b"failures:\n    early_failing\n\n")
out.write(b"test result: FAILED. 1 passed; 1 failed; 0 ignored\n\n")
for binary in range(20):
    err.write(b"     Running tests/later_%d.rs (target/debug/deps/later_%d-4567)\n" % (binary, binary))
    out.write(b"\nrunning 10 tests\n")
    for test in range(10):
        out.write(b"test later_%d_test_%d ... ok\n" % (binary, test))
    out.write(b"\ntest result: ok. 10 passed; 0 failed; 0 ignored\n\n")
err.write(b"   Doc-tests monolith_core\n")
err.write(b"error: 1 target failed:\n    `-p monolith-core --test early`\n")
sys.exit(101)
'''

# A run that prints, then hangs past its limit.
HANGS = r'''
import sys, time
print("started", flush=True)
time.sleep(120)
print("never")
'''

# A run that prints a line, waits for a file to appear, then ends.
WAITS = r'''
import os, sys, time
print("first line", flush=True)
while not os.path.exists(sys.argv[1]):
    time.sleep(0.05)
'''


def program(text, *arguments):
    return [sys.executable, "-c", text, *arguments]


class LoggedRuns(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.logged = os.path.join(self.dir, "run")

    def test_names_a_failing_test_that_comes_before_long_output(self):
        code, timed_out = run.run_logged(program(CARGO), self.dir, dict(os.environ), 60, self.logged)
        self.assertEqual((code, timed_out), (101, False))
        output = run.output(self.logged)
        # The case the summary of the runner lost: the last 3000
        # characters of the output do not name the test.
        self.assertGreater(len(output) - output.index("early_failing ... FAILED"), 3000)
        self.assertNotIn("early_failing", output[-3000:])
        report = run.failure_report(self.logged, code, timed_out)
        self.assertIn("exit code 101", report)
        self.assertIn("    early_failing\n", report)
        self.assertIn("thread 'early_failing' panicked at tests/early.rs:7:5:", report)
        self.assertIn("the early cause", report)
        self.assertIn("`-p monolith-core --test early`", report)
        self.assertIn(self.logged + ".stdout", report)
        self.assertNotIn("later_3_test_4", report)

    def test_keeps_the_whole_output_on_disk(self):
        run.run_logged(program(CARGO), self.dir, dict(os.environ), 60, self.logged)
        with open(self.logged + ".stdout", "rb") as handle:
            stdout = handle.read()
        with open(self.logged + ".stderr", "rb") as handle:
            stderr = handle.read()
        self.assertIn(b"the early cause\r\n\xff\n", stdout)
        self.assertIn(b"test later_19_test_9 ... ok\n", stdout)
        self.assertIn(b"Running tests/later_19.rs", stderr)
        self.assertTrue(stderr.endswith(b"`-p monolith-core --test early`\n"))

    def test_classifies_the_output_the_pipes_gave(self):
        code, timed_out = run.run_logged(program(CARGO), self.dir, dict(os.environ), 60, self.logged)
        piped = subprocess.run(
            program(CARGO), capture_output=True, text=True, encoding="utf-8", errors="replace"
        )
        self.assertEqual(run.output(self.logged), piped.stdout + piped.stderr)
        self.assertEqual(
            run.classify(code, run.output(self.logged), timed_out),
            run.classify(piped.returncode, piped.stdout + piped.stderr, False),
        )
        self.assertEqual(run.classify(code, run.output(self.logged), timed_out), ("caught", ["early_failing"]))

    def test_writes_the_output_while_the_command_runs(self):
        done = os.path.join(self.dir, "done")
        result = []
        runner = threading.Thread(
            target=lambda: result.append(
                run.run_logged(program(WAITS, done), self.dir, dict(os.environ), 60, self.logged)
            )
        )
        runner.start()
        try:
            deadline = time.monotonic() + 30
            seen = b""
            while time.monotonic() < deadline and b"first line" not in seen:
                time.sleep(0.05)
                if os.path.exists(self.logged + ".stdout"):
                    with open(self.logged + ".stdout", "rb") as handle:
                        seen = handle.read()
            self.assertIn(b"first line", seen)
            self.assertTrue(runner.is_alive())
        finally:
            open(done, "w").close()
            runner.join(30)
        self.assertEqual(result, [(0, False)])

    def test_reports_a_run_that_left_no_files(self):
        report = run.failure_report(self.logged, 101, False)
        self.assertIn("exit code 101", report)
        self.assertIn("tests that failed: 0", report)
        self.assertEqual(run.output(self.logged), "")

    def test_ends_a_run_past_its_limit_and_keeps_what_it_printed(self):
        started = time.monotonic()
        code, timed_out = run.run_logged(program(HANGS), self.dir, dict(os.environ), 2, self.logged)
        self.assertEqual((code, timed_out), (None, True))
        self.assertLess(time.monotonic() - started, 60)
        self.assertEqual(run.output(self.logged).strip(), "started")
        self.assertIn("timed out", run.failure_report(self.logged, code, timed_out))


@unittest.skipIf(run.WINDOWS, "a program named cargo found in PATH is a script here only on Unix")
class UnmodifiedRun(unittest.TestCase):
    """The unmodified run of a copy, with a stand-in for cargo first in
    PATH: the command it gets and what the runner prints when it fails."""

    def test_a_failure_names_the_test_and_its_cause(self):
        bin_dir = tempfile.mkdtemp()
        tree = tempfile.mkdtemp()
        arguments = os.path.join(bin_dir, "arguments")
        cargo = os.path.join(bin_dir, "cargo")
        with open(cargo, "w", encoding="utf-8") as handle:
            handle.write(f"#!{sys.executable}\n")
            handle.write(f"import sys\nopen({arguments!r}, 'w').write(' '.join(sys.argv[1:]))\n")
            handle.write(CARGO)
        os.chmod(cargo, os.stat(cargo).st_mode | stat.S_IXUSR)
        logged = os.path.join(tree, "tree0-unmodified")
        printed = io.StringIO()
        with mock.patch.dict(os.environ, {"PATH": bin_dir + os.pathsep + os.environ["PATH"]}):
            with contextlib.redirect_stdout(printed), self.assertRaises(SystemExit) as stopped:
                run.unmodified_run(tree, ["monolith-core", "monolith-ui"], logged)
        self.assertEqual(stopped.exception.code, 3)
        with open(arguments, encoding="utf-8") as handle:
            self.assertEqual(
                handle.read(),
                "test --locked --no-fail-fast -p monolith-core -p monolith-ui --features monolith-ui/fixtures",
            )
        text = printed.getvalue()
        self.assertIn("exit code 101", text)
        self.assertIn("    early_failing\n", text)
        self.assertIn("the early cause", text)
        self.assertIn("`-p monolith-core --test early`", text)
        self.assertIn(logged + ".stdout", text)
        self.assertTrue(text.rstrip().endswith("the unmodified tree does not pass its tests"))


if __name__ == "__main__":
    unittest.main()
