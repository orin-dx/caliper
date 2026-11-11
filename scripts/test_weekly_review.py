#!/usr/bin/env python3
"""Tests for the review fixes to the weekly record: pairing by name, order-free merging, wrapped default exports, pairing
inside one file, the reference version and the measuring helper's errors. Run: python3 scripts/test_weekly_review.py"""
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

    def test_unwraps_assignments_assertions_and_dotted_wrappers(self):
        source = lambda text: weekly.export_source(text, 'default')
        self.assertEqual(source('export default Object.assign(Foo, { Item });\n'), ('local', 'Foo'))
        self.assertEqual(source('export default React.memo(React.forwardRef(Foo));\n'), ('local', 'Foo'))
        self.assertEqual(source('export default Foo as typeof Foo & { Item: Item };\n'), ('local', 'Foo'))
        self.assertEqual(source('export default Foo satisfies Component;\n'), ('local', 'Foo'))
        self.assertEqual(source('export default Foo!;\n'), ('local', 'Foo'))
        self.assertEqual(source('export default memo(Foo) as Bar;\n'), ('local', 'Foo'))
        # A member of something is not the thing.
        self.assertIsNone(source('export default Foo.Bar;\n'))

    FILES = {'src/index.tsx': 'import Foo from "./foo";\nexport default memo(Foo);\n',
             'src/foo.tsx': 'export default function Foo() {}\n'}

    def test_wrapped_export_pairs(self):
        found = {('src/foo.tsx', 'foo'): {'x'}}
        items, stats = weekly.reference([self.entry('index', 'src/index.tsx', {'x': prop()})], found, self.root)
        self.assertEqual(stats['paired'], {'default_export': 1})
        self.assertEqual(items[0]['keys'][-1], ('src/foo.tsx', 'foo'))


class PairsInOneFile(Tree):
    """RDT names a default export after the file or directory, and a component by its declaration; Caliper by its
    `displayName` where there is one. A name RDT does not give a default export is never paired with the file's."""
    FILES = {
        'src/button/index.tsx': 'const InternalButton = () => null;\nexport default InternalButton;\n',
        'src/form/Form.tsx': 'const Form = () => null;\nexport default Form;\n',
        'src/image/Progress.tsx': "const Progress = () => null;\nProgress.displayName = 'ImageProgress';\nexport default Progress;\n",
        'src/tabs/Pane.tsx': "const Pane = () => null;\nPane.displayName = 'TabPane';\nexport const Other = () => null;\nexport default Pane;\n",
        'src/w/index.tsx': 'const Inner = () => null;\nconst Outer = memo(Inner);\nexport default Outer;\n',
        'src/dd/index.tsx': "import Internal from './dropdown';\nconst Dropdown = Internal as typeof Internal & { x: 1 };\n"
                            'export default Dropdown;\n',
        'src/dd/dropdown.tsx': 'export default function Dropdown() {}\n',
        'src/a/index.tsx': 'export const Foo = () => null;\nexport default Foo;\n',
    }

    def pairs(self, rdt, found):
        items, stats = weekly.reference(rdt, found, self.root)
        return items, stats['paired']

    def test_default_export_under_another_name_pairs_with_the_directory_name(self):
        found = {('src/button/index.tsx', 'internalbutton'): {'x'}}
        items, paired = self.pairs([self.entry('button', 'src/button/index.tsx', {'x': prop()})], found)
        self.assertEqual(paired, {'default_alias': 1})
        self.assertEqual(items[0]['keys'], [('src/button/index.tsx', 'button'), ('src/button/index.tsx', 'internalbutton')])

    def test_a_member_name_is_not_the_default_export(self):
        found = {('src/form/Form.tsx', 'form'): {'x'}}
        items, paired = self.pairs([self.entry('List', 'src/form/Form.tsx', {'x': prop()})], found)
        self.assertEqual(paired, {})
        self.assertEqual(weekly.recall(items, found)['components'], [0, 1])

    def test_a_key_another_entry_matches_exactly_is_not_a_target(self):
        found = {('src/a/index.tsx', 'foo'): {'x'}}
        entries = [self.entry('Foo', 'src/a/index.tsx', {'x': prop()}), self.entry('a', 'src/a/index.tsx', {'x': prop()})]
        items, paired = self.pairs(entries, found)
        self.assertEqual(paired, {})
        self.assertEqual([i['keys'][0] for i in items], [('src/a/index.tsx', 'foo'), ('src/a/index.tsx', 'a')])

    def test_display_name_both_ways(self):
        # Caliper names the component by its displayName, RDT by the declaration.
        found = {('src/image/Progress.tsx', 'imageprogress'): {'x'}}
        _, paired = self.pairs([self.entry('Progress', 'src/image/Progress.tsx', {'x': prop()})], found)
        self.assertEqual(paired, {'default_alias': 1})
        # RDT names it by the displayName, Caliper by the declaration.
        found = {('src/tabs/Pane.tsx', 'pane'): {'x'}}
        items, paired = self.pairs([self.entry('TabPane', 'src/tabs/Pane.tsx', {'x': prop()})], found)
        self.assertEqual(paired, {'display_name': 1})
        self.assertEqual(items[0]['keys'][-1], ('src/tabs/Pane.tsx', 'pane'))

    def test_local_alias_chain_reaches_the_declaration(self):
        found = {('src/w/index.tsx', 'inner'): {'x'}}
        _, paired = self.pairs([self.entry('w', 'src/w/index.tsx', {'x': prop()})], found)
        self.assertEqual(paired, {'default_alias': 1})

    def test_alias_chain_follows_an_import(self):
        self.assertEqual(weekly.local_chain('const A = B as T;\nconst B = memo(C);\n', 'A'), ['A', 'B', 'C'])
        self.assertEqual(weekly.local_chain('const A = A;\n', 'A'), ['A'])
        found = {('src/dd/dropdown.tsx', 'dropdown'): {'x'}}
        _, paired = self.pairs([self.entry('Dropdown', 'src/dd/index.tsx', {'x': prop()})], found)
        self.assertEqual(paired, {'default_export': 1})


def record(reference_version=None, recall=0.9, ms=100):
    r = {'corpora': {'antd': {'caliper': {'ms': ms, 'rss_mib': 100},
                              'recall': {'caliper': {'components': [int(100 * recall), 100]}}}}}
    if reference_version is not None:
        r['reference_version'] = reference_version
    return r


class RecallIsNotComparedAcrossReferences(unittest.TestCase):
    def test_same_reference_compares_recall(self):
        self.assertEqual(weekly.changes(record(weekly.REFERENCE_VERSION, 0.5), record(weekly.REFERENCE_VERSION, 0.9)),
                         ['antd: recall of components 90.0% -> 50.0%'])

    def test_another_reference_skips_recall_but_not_time(self):
        now, before = record(weekly.REFERENCE_VERSION, 0.5, ms=200), record(weekly.REFERENCE_VERSION - 1, 0.9)
        self.assertEqual(weekly.changes(now, before), ['antd: time 100 -> 200'])
        self.assertFalse(weekly.same_reference(now, before))

    def test_a_record_without_a_version_is_version_one(self):
        self.assertTrue(weekly.same_reference(record(1), record()))
        self.assertFalse(weekly.same_reference(record(weekly.REFERENCE_VERSION), record()))

    def universe(self, version, wrong, **extra):
        """A corpus entry whose oracle counted the universe of `version`, with a headline of `wrong` wrong props."""
        oracle = {'tools': {}, **({'universe_version': version} if version else {})}
        return {'caliper': {'ms': 100, 'rss_mib': 100}, 'oracle': oracle, 'recall': {'caliper': {}},
                'headline': {'components': 1, 'props': 2000, 'correct': 1900, 'less_precise': 0, 'wrong': wrong,
                             'no_checker_type': 0, 'missing': 0, 'named_only': 0}, **extra}

    def test_a_rise_in_wrong_types_is_not_read_against_another_universe(self):
        now = {'corpora': {'antd': self.universe(2, 1)}}
        before = {'corpora': {'antd': self.universe(None, 0)}}
        self.assertEqual(weekly.findings(now, before), [])
        again = {'corpora': {'antd': self.universe(2, 0)}}
        self.assertEqual(len(weekly.findings(now, again)), 1)

    def test_the_pairing_by_export_is_universe_version_3_and_is_not_compared_with_2(self):
        self.assertEqual(weekly.UNIVERSE_VERSION, 3)
        now = {'corpora': {'antd': self.universe(3, 1)}}
        before = {'corpora': {'antd': self.universe(2, 0)}}
        self.assertEqual(weekly.findings(now, before), [])
        self.assertEqual(weekly.changes(now, before), [
            "antd: the oracle's universe is version 3 now and was version 2 last week, so its props, agreement and wrong "
            "types are not compared with last week's"])

    def test_the_summary_says_the_universe_changed(self):
        now = {'corpora': {'antd': self.universe(2, 0)}}
        before = {'corpora': {'antd': self.universe(None, 0)}}
        self.assertEqual(weekly.changes(now, before), [
            "antd: the oracle's universe is version 2 now and was version 1 last week, so its props, agreement and wrong "
            "types are not compared with last week's"])
        self.assertEqual(weekly.changes(now, now), [])

    def test_a_corpus_without_an_oracle_has_no_universe_to_differ(self):
        self.assertTrue(weekly.same_universe({'caliper': {}}, {'oracle': {'tools': {}}}))
        self.assertTrue(weekly.same_universe({'oracle': {'error': 'x'}}, {'oracle': {'tools': {}}}))

    def test_the_table_says_when_recall_was_not_compared(self):
        now = {'caliper': 'x', 'runs': 1, 'reference_version': 2, 'corpora': {}}
        self.assertIn('version 1 of the rules, this one by version 2', weekly.markdown(now, [], 1))
        self.assertNotIn('not compared', weekly.markdown(now, []))

    def test_the_recall_section_says_recall_is_not_correctness(self):
        now = {'caliper': 'x', 'runs': 1, 'reference_version': 2, 'corpora': {}}
        self.assertIn('It is not a measure of correctness', weekly.markdown(now, []))


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
