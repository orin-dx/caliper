#!/usr/bin/env python3
"""Tests for the stage limits in scripts/equivalence.py: `python3 -I -m unittest discover -s scripts`."""
import os
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import equivalence  # noqa: E402


class RunBounded(unittest.TestCase):
    def test_a_command_that_finishes_returns_its_output(self):
        result = equivalence.run_bounded(['sh', '-c', 'echo out; echo err >&2'], stage='echo', timeout=30)
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, 'out\n', 'err\n'))

    def test_a_failing_command_raises_when_checked_and_returns_when_not(self):
        with self.assertRaises(equivalence.subprocess.CalledProcessError):
            equivalence.run_bounded(['sh', '-c', 'exit 3'], stage='fail', timeout=30)
        result = equivalence.run_bounded(['sh', '-c', 'exit 3'], stage='fail', timeout=30, check=False)
        self.assertEqual(result.returncode, 3)

    def test_a_command_over_its_limit_is_killed_and_named(self):
        started = time.time()
        with self.assertRaises(equivalence.StageTimeout) as caught:
            equivalence.run_bounded(['sh', '-c', 'echo before the hang; sleep 60'], stage='the slow stage', timeout=1)
        self.assertLess(time.time() - started, 30)
        message = str(caught.exception)
        self.assertIn('the slow stage did not finish within 1s', message)
        self.assertIn('before the hang', message)

    def test_a_child_that_outlives_its_parent_is_killed_with_the_group(self):
        marker = f'equivalence-test-{os.getpid()}'
        with self.assertRaises(equivalence.StageTimeout):
            # The shell backgrounds a long sleep and waits: only killing the process group ends the sleep.
            equivalence.run_bounded(['sh', '-c', f'sleep 61 & echo {marker}; wait'], stage='children', timeout=1)
        time.sleep(0.5)
        survivors = equivalence.subprocess.run(['pgrep', '-f', '^sleep 61$'], capture_output=True, text=True).stdout
        self.assertEqual(survivors.split(), [])

    def test_a_command_that_reads_stdin_does_not_wait_for_it(self):
        result = equivalence.run_bounded(['sh', '-c', 'cat; echo done'], stage='stdin', timeout=30)
        self.assertEqual(result.stdout, 'done\n')


if __name__ == '__main__':
    unittest.main()


class AllowList(unittest.TestCase):

    def test_specific_entries_pass(self):
        entries = [
            'antd/* prop-added Button.color',
            'shadcn/deps* prop-type-changed Combobox*',
            "mantine/deps* diagnostic-added info OPAQUE_TYPE 'Payload*",
            'chakra/deps* component-field-changed *Group.composes',
        ]
        self.assertEqual(equivalence.allow_problems(entries), [])

    def test_a_blanket_entry_is_named(self):
        entries = ['antd/* prop-added *', 'shadcn/deps* notable-added *', '*/* prop-added Button.x', 'antd/* * Button.x']
        problems = equivalence.allow_problems(entries)
        self.assertEqual(len(problems), 4)
        self.assertIn("'antd/* prop-added *': names no component, prop or diagnostic", problems[0])
        self.assertIn('names no corpus', problems[2])
        self.assertIn('names no kind of difference', problems[3])

    def test_a_malformed_entry_is_named(self):
        self.assertEqual(len(equivalence.allow_problems(['prop-added Button.x', 7])), 2)

    def test_the_checked_in_entries_are_valid(self):
        patterns = [pattern for _, pattern in equivalence.load_allow(equivalence.DEFAULT_ALLOW)]
        self.assertEqual(equivalence.allow_problems(patterns), [])

    def test_every_file_in_the_directory_counts_in_name_order_and_nothing_else_does(self):
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            files = {'b-feature.json': ['b/* prop-added B.x'], 'a-fix.json': ['a/* prop-added A.y', 'a/* prop-added A.z']}
            for name, allow in files.items():
                with open(os.path.join(tmp, name), 'w') as f:
                    json.dump({'allow': allow}, f)
            with open(os.path.join(tmp, 'notes.txt'), 'w') as f:
                f.write('not an allow-list')
            expected = [(name, pattern) for name in sorted(files) for pattern in files[name]]
            self.assertEqual(equivalence.load_allow(tmp), expected)

    def test_a_missing_directory_has_no_entries(self):
        self.assertEqual(equivalence.load_allow('/no/such/allow-list-directory'), [])


class ResolveBase(unittest.TestCase):

    def test_an_executable_path_is_taken_as_given(self):
        import stat
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            binary = os.path.join(tmp, 'caliper-base')
            with open(binary, 'w') as f:
                f.write('#!/bin/sh\n')
            os.chmod(binary, os.stat(binary).st_mode | stat.S_IXUSR)
            self.assertEqual(equivalence.resolve_base(binary, tmp, 5), binary)

    def test_a_missing_path_is_named_rather_than_handed_on(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(SystemExit) as raised:
                equivalence.resolve_base(os.path.join(tmp, 'no-such-binary'), tmp, 5)
            self.assertIn('not an executable and not a commit', str(raised.exception))
