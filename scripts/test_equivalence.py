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

    def test_the_checked_in_list_is_valid(self):
        import json

        with open(equivalence.DEFAULT_ALLOW) as f:
            self.assertEqual(equivalence.allow_problems(json.load(f)['allow']), [])


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


class Allowed(unittest.TestCase):
    PATTERNS = [
        'chakra/deps* prop-added *.accessKey',
        'chakra/deps* prop-added *.aria-label.declarations',
        "shadcn/deps* diagnostic-added info OPAQUE_TYPE 'Foo*",
        'chakra/deps* component-field-changed Box.inheritance',
    ]

    def matches_like_fnmatch(self, label, kind, item):
        return any(equivalence.fnmatch.fnmatchcase(f'{label} {kind} {item}', p) for p in self.PATTERNS)

    def test_a_prop_name_entry_allows_that_prop_of_any_component_and_nothing_else(self):
        cases = [
            ('chakra/deps', 'prop-added', 'AccordionItem.accessKey', True),
            ('chakra/deps+html-full', 'prop-added', 'Card (/src/card.tsx).accessKey', True),
            ('chakra/deps', 'prop-added', 'AccordionItem.aria-label.declarations', True),
            ('chakra/deps', 'prop-added', 'AccordionItem.accessKeys', False),
            ('chakra/deps', 'prop-removed', 'AccordionItem.accessKey', False),
            ('shadcn/deps', 'prop-added', 'AccordionItem.accessKey', False),
            ('chakra/deps', 'component-field-changed', 'Box.inheritance', True),
            ('chakra/deps', 'component-field-changed', 'Flex.inheritance', False),
        ]
        for label, kind, item, expected in cases:
            with self.subTest(label=label, kind=kind, item=item):
                self.assertEqual(equivalence.allowed(label, kind, item, self.PATTERNS), expected)
                self.assertEqual(self.matches_like_fnmatch(label, kind, item), expected)
