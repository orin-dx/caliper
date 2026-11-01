#!/usr/bin/env python3
"""Tests for the review fixes to the weekly record: pairing by name, order-free merging, wrapped default exports and
the measuring helper's errors. Run: python3 scripts/test_weekly_review.py"""
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import corpora
import weekly

REACT = '/r/node_modules/@types/react/index.d.ts'


def prop(parent=''):
    return {'parent': parent, 'type': ''}


class Tree(unittest.TestCase):
    FILES = {}

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.root = os.path.realpath(self.dir.name)
        for rel, text in self.FILES.items():
            path = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, 'w') as f:
                f.write(text)

    def entry(self, name, rel, props):
        return {'name': name, 'file': os.path.join(self.root, rel), 'props': props}


class PairsOnlyByName(Tree):
    FILES = {'a.tsx': 'const Helper = () => null;\nexport default Helper;\n', 'index.tsx': 'export { default } from "./a";\n'}

    def test_default_export_of_another_name_does_not_pair(self):
        # RDT's `index` entry resolves to the local Helper in a.tsx; Caliper reports only `other` there.
        found = {('a.tsx', 'other'): {'x'}}
        items, stats = weekly.reference([self.entry('index', 'index.tsx', {'x': prop()})], found, self.root)
        self.assertEqual(stats['paired'], {})
        self.assertEqual(stats['unpaired'], 1)
        self.assertEqual(weekly.recall(items, found)['components'], [0, 1])

    def test_default_export_of_the_same_name_pairs(self):
        found = {('a.tsx', 'helper'): {'x'}}
        items, stats = weekly.reference([self.entry('index', 'index.tsx', {'x': prop()})], found, self.root)
        self.assertEqual(stats['paired'], {'default_export': 1})
        self.assertEqual(items[0]['keys'], [('index.tsx', 'index'), ('a.tsx', 'helper')])


class MergeIsOrderFree(Tree):
    FILES = {'a.tsx': 'export const Box = () => null;\n', 'b.tsx': 'export { Box } from "./a";\n',
             'c.tsx': 'export { Box } from "./a";\n'}

    def merged(self, order):
        entries = {'b': self.entry('Box', 'b.tsx', {'p1': prop()}), 'c': self.entry('Box', 'c.tsx', {'p2': prop(REACT)})}
        found = {('a.tsx', 'box'): set()}
        items, stats = weekly.reference([entries[k] for k in order], found, self.root)
        self.assertEqual((stats['reference'], stats['merged']), (1, 1))
        return items[0]['props']

    def test_repeats_without_an_exact_match_are_united(self):
        self.assertEqual(self.merged('bc'), {'p1': 'own', 'p2': 'react'})
        self.assertEqual(self.merged('bc'), self.merged('cb'))

    def test_exact_match_wins_in_either_order(self):
        found = {('a.tsx', 'box'): set()}
        exact, repeat = self.entry('Box', 'a.tsx', {'p0': prop()}), self.entry('Box', 'b.tsx', {'p1': prop()})
        for order in ((exact, repeat), (repeat, exact)):
            items, _ = weekly.reference(list(order), found, self.root)
            self.assertEqual(items[0]['props'], {'p0': 'own'})


class WrappedDefaultExport(Tree):
    def test_unwraps_calls_by_shape(self):
        source = lambda text: weekly.export_source(text, 'default')
        self.assertEqual(source('export default memo(Foo);\n'), ('local', 'Foo'))
        self.assertEqual(source('export default forwardRef<Ref, Props>(Foo)\n'), ('local', 'Foo'))
        self.assertEqual(source('export default observer(memo(Foo));\n'), ('local', 'Foo'))
        self.assertEqual(source('export default Foo;\n'), ('local', 'Foo'))
        self.assertEqual(source('export default memo(Foo, areEqual);\n'), ('local', 'Foo'))
        self.assertIsNone(source('export default memo(() => null);\n'))
        self.assertIsNone(source('export default { x };\n'))

    FILES = {'src/index.tsx': 'import Foo from "./foo";\nexport default memo(Foo);\n',
             'src/foo.tsx': 'export default function Foo() {}\n'}

    def test_wrapped_export_pairs(self):
        found = {('src/foo.tsx', 'foo'): {'x'}}
        items, stats = weekly.reference([self.entry('index', 'src/index.tsx', {'x': prop()})], found, self.root)
        self.assertEqual(stats['paired'], {'default_export': 1})
        self.assertEqual(items[0]['keys'][-1], ('src/foo.tsx', 'foo'))


class MeasureHelperErrors(unittest.TestCase):
    def test_command_that_cannot_run_names_itself(self):
        with self.assertRaisesRegex(RuntimeError, r'no-such-command-here exited with 127: cannot run no-such-command-here'):
            corpora.run_once(['no-such-command-here'])

    def test_exit_code_and_stderr_are_reported(self):
        with self.assertRaisesRegex(RuntimeError, r'exited with 3: oops'):
            corpora.run_once([sys.executable, '-c', 'import sys; sys.stderr.write("oops"); sys.exit(3)'])

    def test_helper_failure_is_a_runtime_error(self):
        failed = subprocess.CompletedProcess([], 1, stderr='Traceback: boom\n')
        with mock.patch.object(corpora.subprocess, 'run', return_value=failed):
            with self.assertRaisesRegex(RuntimeError, r'helper failed \(exit 1\) running x y: Traceback: boom'):
                corpora.run_once(['x', 'y'])

    def test_missing_result_is_a_runtime_error(self):
        wrote_nothing = subprocess.CompletedProcess([], 0, stderr='')
        with mock.patch.object(corpora.subprocess, 'run', return_value=wrote_nothing):
            with self.assertRaisesRegex(RuntimeError, r'wrote no result for x'):
                corpora.run_once(['x'])

    def test_result_is_parsed(self):
        def write(command, **_):
            with open(command[command.index('measure') + 1], 'w') as f:
                f.write('[1.5, 42.0, 0]')
            return subprocess.CompletedProcess([], 0, stderr='')
        with mock.patch.object(corpora.subprocess, 'run', side_effect=write):
            self.assertEqual(corpora.run_once(['x']), (1.5, 42.0))


if __name__ == '__main__':
    unittest.main()
