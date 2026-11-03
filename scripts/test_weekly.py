#!/usr/bin/env python3
"""Tests for the weekly record's helpers, on small fixtures. Run: python3 scripts/test_weekly.py"""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import corpora
import weekly

LIB = '/r/node_modules/typescript/lib/lib.es5.d.ts'
REACT = '/r/node_modules/@types/react/index.d.ts'


def prop(parent='', type=''):
    return {'parent': parent, 'type': type}


def comp(name, props, file='/r/src/button/Button.tsx'):
    return {'name': name, 'file': file, 'props': props}


class NormName(unittest.TestCase):
    def test_spellings(self):
        self.assertEqual(weekly.norm_name('Button'), 'button')
        self.assertEqual(weekly.norm_name('Button (src/button.tsx)'), 'button')
        self.assertEqual(weekly.norm_name('Select.Option'), 'option')
        self.assertEqual(weekly.norm_name('ns/Date-Picker'), 'datepicker')


class Exclusion(unittest.TestCase):
    def test_lib_members(self):
        self.assertEqual(weekly.exclusion(comp('GET', {k: prop(LIB) for k in 'abc'})), 'lib_members')
        # A minority of lib props is a component with a few odd props.
        self.assertIsNone(weekly.exclusion(comp('Button', {'a': prop(LIB), 'b': prop(), 'c': prop()})))

    def test_hook(self):
        self.assertEqual(weekly.exclusion(comp('useThing', {'a': prop()})), 'hook')
        self.assertIsNone(weekly.exclusion(comp('Userland', {'a': prop()})))

    def test_context_value(self):
        self.assertEqual(weekly.exclusion(comp('FormContext', {'value': prop(REACT)})), 'context_value')
        self.assertIsNone(weekly.exclusion(comp('Box', {'value': prop()})))
        self.assertIsNone(weekly.exclusion(comp('Box', {'value': prop(REACT), 'size': prop(REACT)})))

    def test_lowercase_names(self):
        self.assertEqual(weekly.exclusion(comp('withWarn', {'open': prop()})), 'not_jsx')
        # The file's or directory's name is RDT's name for a default export.
        self.assertIsNone(weekly.exclusion(comp('col', {'span': prop()}, '/r/src/col/index.tsx')))
        self.assertIsNone(weekly.exclusion(comp('datepicker', {'span': prop()}, '/r/src/date-picker/index.tsx')))
        # A member component takes a tag's attributes.
        self.assertIsNone(weekly.exclusion(comp('logo', {'className': prop(REACT)}, '/r/src/icons.tsx')))

    def test_promise_props(self):
        self.assertEqual(weekly.exclusion(comp('page', {'params': prop(type='Promise<{ a: string; }>')},
                                               '/r/app/page.tsx')), 'promise_props')
        mixed = {'params': prop(type='Promise<{}>'), 'title': prop(type='string')}
        self.assertIsNone(weekly.exclusion(comp('page', mixed, '/r/app/page.tsx')))


FILES = {
    'src/grid/index.tsx': "import Col from './col';\nimport Row from './row';\nexport { Col, Row };\nexport default { x };\n",
    'src/grid/col.tsx': "const Col = () => null;\nexport default Col;\n",
    'src/grid/row.tsx': "export default function Row() {}\n",
    'src/col/index.tsx': "// import directly from the old path.\nimport { Col } from '../grid';\n\nexport default Col;\n",
    'src/qr/index.tsx': "export { default } from './qr-code';\n",
    'src/qr/qr-code.tsx': "export default function QRCode() {}\n",
    'src/other/index.tsx': "import X from 'some-package';\nexport default X;\n",
}


class Pairing(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self.dir.name)
        for rel, text in FILES.items():
            path = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, 'w') as f:
                f.write(text)
        self.exists = lambda rel: os.path.isfile(os.path.join(self.root, rel))
        self.read = lambda rel: weekly.read_text(os.path.join(self.root, rel))

    def tearDown(self):
        self.dir.cleanup()

    def entry(self, name, rel, props=None):
        return comp(name, props or {'span': prop(), 'order': prop(REACT)}, os.path.join(self.root, rel))

    def test_export_target_follows_default_and_named_exports(self):
        target = lambda rel, name='default': weekly.export_target(rel, self.read, self.exists, name)
        self.assertEqual(target('src/col/index.tsx'), ('src/grid/col.tsx', 'Col'))
        self.assertEqual(target('src/grid/index.tsx', 'Row'), ('src/grid/row.tsx', 'Row'))
        self.assertEqual(target('src/qr/index.tsx'), ('src/qr/qr-code.tsx', 'QRCode'))
        self.assertIsNone(target('src/other/index.tsx'))
        self.assertIsNone(target('src/missing.tsx'))

    def test_reference_counts_a_reexported_component_once(self):
        found = {('src/grid/col.tsx', 'col'): {'span'}, ('src/grid/row.tsx', 'row'): {'span', 'order'}}
        rdt = [self.entry('Col', 'src/grid/col.tsx'), self.entry('col', 'src/col/index.tsx'),
               self.entry('Col', 'src/grid/index.tsx'), self.entry('Row', 'src/grid/index.tsx'),
               self.entry('Row', 'src/grid/row.tsx'), self.entry('Lost', 'src/other/index.tsx')]
        items, stats = weekly.reference(rdt, found, self.root)
        self.assertEqual(stats, {'rdt_with_props': 6, 'excluded': {}, 'exact': 2, 'paired': {'default_export': 1, 'named_export': 2},
                                 'merged': 3, 'reference': 3, 'unpaired': 1, 'excluded_total': 0,
                                 'unmatched': {'by_cause': {'file_without_components': 1},
                                               'listed': {'file_without_components': ['src/other/index.tsx: Lost']}}})
        by_first = {item['keys'][0]: item for item in items}
        self.assertEqual(set(by_first[('src/grid/col.tsx', 'col')]['keys']),
                         {('src/grid/col.tsx', 'col'), ('src/col/index.tsx', 'col'), ('src/grid/index.tsx', 'col')})
        self.assertEqual(by_first[('src/grid/col.tsx', 'col')]['props'], {'span': 'own', 'order': 'react'})
        count = weekly.recall(items, found)
        self.assertEqual(count['components'], [2, 3])
        self.assertEqual((count['own'], count['react']), ([2, 2], [1, 2]))

    def test_reference_excludes_and_counts(self):
        rdt = [self.entry('useThing', 'src/grid/col.tsx'), self.entry('Col', 'src/grid/col.tsx'),
               comp('GET', {'a': prop(LIB)}, os.path.join(self.root, 'src/grid/row.tsx')), comp('Empty', {})]
        items, stats = weekly.reference(rdt, {('src/grid/col.tsx', 'col'): set()}, self.root)
        self.assertEqual((stats['rdt_with_props'], stats['excluded'], stats['reference']),
                         (3, {'hook': 1, 'lib_members': 1}, 1))
        self.assertEqual([i['keys'] for i in items], [[('src/grid/col.tsx', 'col')]])

    def test_declaring_file_pairs_when_no_export_chain_does(self):
        found = {('src/grid/col.tsx', 'dropdown'): {'span'}}
        entry = self.entry('Dropdown', 'src/other/index.tsx', {'span': prop(os.path.join(self.root, 'src/grid/col.tsx'))})
        items, stats = weekly.reference([entry], found, self.root)
        self.assertEqual(stats['paired'], {'declaration': 1})
        self.assertEqual(items[0]['keys'], [('src/other/index.tsx', 'dropdown'), ('src/grid/col.tsx', 'dropdown')])


class Unmatched(unittest.TestCase):
    """What the file shows about an item no tool name matches."""
    FILES = {
        'src/icons.tsx': "export const Icons = {\n  logo: (props: P) => <svg {...props} />,\n  spin: memo(Spin),\n  size: number,\n};\n",
        'src/form/Form.tsx': 'const Form = () => null;\nconst List = () => null;\nForm.List = List;\nexport default Form;\n',
        'src/chart/index.tsx': "import * as Recharts from 'recharts';\nconst Tooltip = Recharts.Tooltip;\nconst Other = () => null;\n",
        'src/icon/lucide.tsx': "export const IconLucide = createIcon<Props>('lucide');\n",
        'src/calendar/index.tsx': "import config from 'pkg/dayjs';\nconst Calendar = generate<Dayjs>(config);\nexport default Calendar;\n",
        'src/direction/index.tsx': "export { Provider as DirectionProvider } from 'pkg';\n",
        'src/plain/Plain.tsx': 'const Plain = () => null;\nexport default Plain;\n',
    }

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.root = os.path.realpath(self.dir.name)
        for rel, text in self.FILES.items():
            path = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, 'w') as f:
                f.write(text)

    def cause(self, name, rel, found=()):
        found_by_file = {}
        for file, key in found:
            found_by_file.setdefault(file, {})[key] = None
        read = lambda r: weekly.read_text(os.path.join(self.root, r))
        exists = lambda r: os.path.isfile(os.path.join(self.root, r))
        return weekly.unmatched_cause(name, rel, found_by_file, read, exists)

    def test_causes(self):
        self.assertEqual(self.cause('logo', 'src/icons.tsx'), 'object_member')
        self.assertEqual(self.cause('spin', 'src/icons.tsx'), 'object_member')
        # A type annotation is not an object member.
        self.assertEqual(self.cause('size', 'src/icons.tsx'), 'file_without_components')
        self.assertEqual(self.cause('List', 'src/form/Form.tsx'), 'compound_member')
        self.assertEqual(self.cause('Tooltip', 'src/chart/index.tsx', [('src/chart/index.tsx', 'other')]), 'dependency')
        self.assertEqual(self.cause('IconLucide', 'src/icon/lucide.tsx'), 'call_result')
        self.assertEqual(self.cause('DirectionProvider', 'src/direction/index.tsx'), 'dependency')

    def test_default_export_is_named_by_the_file(self):
        # RDT calls the default export `calendar`; it is declared `Calendar`, built by a call.
        self.assertEqual(self.cause('calendar', 'src/calendar/index.tsx'), 'call_result')

    def test_declared_but_not_reported(self):
        self.assertEqual(self.cause('Plain', 'src/plain/Plain.tsx'), 'file_without_components')
        self.assertEqual(self.cause('Plain', 'src/plain/Plain.tsx', [('src/plain/Plain.tsx', 'x')]),
                         'file_with_other_components')

    def test_the_record_counts_every_cause_and_caps_the_lists(self):
        rdt = [comp(f'N{i}', {'a': prop()}, f'{self.root}/src/plain/Plain.tsx') for i in range(weekly.LIST_CAP + 3)]
        _, stats = weekly.reference(rdt, {}, self.root)
        self.assertEqual(stats['unmatched']['by_cause'], {'file_without_components': weekly.LIST_CAP + 3})
        self.assertEqual(len(stats['unmatched']['listed']['file_without_components']), weekly.LIST_CAP)


class Recall(unittest.TestCase):
    def test_layers_count_only_in_layer_columns(self):
        items = [{'keys': [('a.tsx', 'a')], 'props': {'x': 'own', 'y': 'own', 'onClick': 'react'}}]
        found = {('a.tsx', 'a'): {'x', 'onClick'}}
        count = weekly.recall(items, found, {('a.tsx', 'a'): {'y'}})
        self.assertEqual((count['own'], count['own_layers'], count['react'], count['react_layers']),
                         ([1, 2], [2, 2], [1, 1], [1, 1]))
        self.assertEqual(weekly.recall(items, {}), {'components': [0, 1], 'own': [0, 0], 'react': [0, 0],
                                                    'own_layers': [0, 0], 'react_layers': [0, 0]})


def score(listed=3, universe=10, agree=0.4, **more):
    return {'universe': universe, 'listed': listed, 'agree_universe': agree, 'agree_listed': 0.8, 'wrong_listed': 0.05,
            'missing_by_stratum': {'same_file': 4, 'other_project_file': 1, 'dependency': 2, 'unknown': 0},
            'missing_by_reason': {'component': 5, 'failed_file': 0, 'prop': 2, 'collapsed': 0}, 'examples': {}, **more}


class OracleTools(unittest.TestCase):
    def test_the_oracle_gets_both_dumps_and_the_record_keeps_caliper_numbers_and_the_tool_scores(self):
        with tempfile.TemporaryDirectory() as work:
            calls = []

            def fake_run(command, **kwargs):
                calls.append(command)
                if '--out' in command:
                    with open(command[command.index('--out') + 1], 'w') as f:
                        f.write(json.dumps({'props_shared': 10, 'verdicts': {'identical': 6, 'spelled': 2, 'wrong': 1},
                                            'tools': {'caliper': score(), 'rdt': score(listed=9)}}))
                return subprocess.CompletedProcess(command, 0)

            with mock.patch.object(weekly.subprocess, 'run', side_effect=fake_run):
                out = weekly.oracle('bin', 'antd', os.path.join(work, 'src'), work, work, '/w/rdt.json', '/w/rdg.json')
        oracle_call = next(c for c in calls if '--out' in c)
        self.assertEqual(oracle_call[oracle_call.index('--rdt') + 1], '/w/rdt.json')
        self.assertEqual(oracle_call[oracle_call.index('--rdg') + 1], '/w/rdg.json')
        self.assertEqual((out['shared_props'], out['agree'], out['wrong']), (10, 0.8, 0.1))
        self.assertEqual(sorted(out['tools']), ['caliper', 'rdt'])
        self.assertNotIn('examples', out['tools']['rdt'])
        self.assertEqual(out['tools']['rdt']['listed'], 9)

    def test_a_tool_line_says_what_it_agrees_on_and_what_it_omits(self):
        self.assertEqual(
            weekly.tool_line('antd', 'rdt', score(universe=7199, listed=3442, agree=0.4484)),
            "- antd, RDT: agrees on 44.8% of the 7,199 props the checker finds (80.0% of the 3,442 it lists, 5.00% of those "
            "wrong); omits 7 (4 declared in the component's file, 1 in other project files, 2 in dependencies, 0 unknown)")

    def test_collapsed_layers_and_thrown_files_are_their_own_clauses(self):
        line = weekly.tool_line('shadcn', 'react_docgen', score(failed_files=265, missing_by_reason={
            'component': 0, 'failed_file': 13077, 'prop': 2, 'collapsed': 5510}))
        self.assertTrue(line.endswith('; 5,510 more sit in collapsed layers; threw on 265 files, 13,077 of the omitted props'))
        self.assertIn('- shadcn, react-docgen: ', line)

    def test_the_summary_lists_each_tool_after_the_oracle_table(self):
        corpus = {'caliper': {'ms': 1, 'rss_mib': 1, 'json_mb': 1}, 'rdt': {'ms': 1, 'rss_mib': 1},
                  'react_docgen': {'ms': 1, 'rss_mib': 1},
                  'reference': {'rdt_with_props': 1, 'excluded_total': 0, 'merged': 0, 'reference': 1, 'exact': 1,
                                'paired': {}, 'unpaired': 0},
                  'recall': {'caliper': {k: [1, 1] for k in ('components', 'own', 'react', 'own_layers', 'react_layers')},
                             'react_docgen': {k: [1, 1] for k in ('components', 'own', 'react')}},
                  'oracle': {'shared_props': 10, 'agree': 0.8, 'wrong': 0.0,
                             'tools': {'caliper': score(), 'rdt': score(), 'react_docgen': score()}}}
        text = weekly.markdown({'caliper': 'x', 'runs': 1, 'reference_version': 2, 'corpora': {'antd': corpus}}, [])
        order = [text.index(f'- antd, {label}: ') for label in ('Caliper', 'RDT', 'react-docgen')]
        self.assertEqual(order, sorted(order))
        self.assertLess(text.index("### TypeScript 7.1's checker"), order[0])


class Baseline(unittest.TestCase):
    def previous_file(self, text):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = os.path.join(tmp.name, 'weekly.json')
        with open(path, 'w') as f:
            f.write(text)
        return path

    def test_a_corrupt_previous_stops_the_run_naming_the_file(self):
        path = self.previous_file('{"corpora": ')
        with self.assertRaises(SystemExit) as raised:
            weekly.load_previous(path)
        self.assertTrue(str(raised.exception).startswith(f'--previous {path} is not a readable record: '))

    def test_a_previous_with_a_renamed_corpora_key_stops_the_run(self):
        path = self.previous_file('{"corpus": {}}')
        with self.assertRaises(SystemExit) as raised:
            weekly.load_previous(path)
        self.assertEqual(str(raised.exception), f'--previous {path} is not a weekly record: it has no "corpora" object')

    def test_a_missing_previous_is_no_baseline_and_the_summary_says_so(self):
        self.assertEqual(weekly.load_previous('/nonexistent/weekly.json'), {})
        now = {'caliper': 'x', 'runs': 1, 'reference_version': 2, 'corpora': {}}
        self.assertIn('No baseline: there was no earlier record', weekly.markdown(now, [], baseline=False))
        self.assertNotIn('No baseline', weekly.markdown(now, []))

    def test_a_failed_baseline_download_is_not_called_a_missing_baseline(self):
        now = {'caliper': 'x', 'runs': 1, 'reference_version': 2, 'corpora': {}}
        table = weekly.markdown(now, [], baseline=False, baseline_failed=True)
        self.assertIn("The baseline download failed, so this run was not compared with last week's record.", table)
        self.assertNotIn('No baseline', table)

    def test_the_header_states_the_warm_versus_cold_measurement(self):
        now = {'caliper': 'x', 'runs': 5, 'reference_version': 2, 'corpora': {}}
        header = weekly.markdown(now, []).splitlines()[0]
        self.assertEqual(header, 'Caliper x, time the median of 5 runs (after one warm-up run) and memory the largest peak '
                                 'of them; RDT and react-docgen one cold run each, so the comparison favours Caliper. '
                                 'Whole process, Node start included.')

    def test_an_oracle_error_is_an_issue_line(self):
        record = {'corpora': {'chakra': {'oracle': {'error': 'the comparison is empty'}, 'agent_eval': {'recall': 1}}}}
        self.assertEqual(weekly.errors(record), ['chakra, oracle: the comparison is empty'])


class AddonConfiguration(unittest.TestCase):
    def test_the_timed_command_is_the_addons_collapse_none_with_dependencies(self):
        commands = []

        def run_once(command, **kwargs):
            commands.append(command)
            return 1.0, 10

        with mock.patch.object(weekly, 'run_once', run_once):
            weekly.caliper_run('/bin/caliper-docgen', '/src', 1)
        command = commands[-1]
        self.assertIn('--load-dependencies', command)
        self.assertEqual(command[command.index('--collapse-inherited-props') + 1], 'none')


class PeakRss(unittest.TestCase):
    def test_units(self):
        self.assertEqual(corpora.rss_mib(5 * 2**20, 'darwin'), 5)
        self.assertEqual(corpora.rss_mib(5 * 1024, 'linux'), 5)

    def test_each_run_reports_its_own_peak(self):
        small = corpora.run_once([sys.executable, '-c', 'pass'])[1]
        big = corpora.run_once([sys.executable, '-c', 'b = bytearray(200 * 2**20); b[::4096] = bytes(len(b[::4096]))'])[1]
        again = corpora.run_once([sys.executable, '-c', 'pass'])[1]
        self.assertGreater(big, small + 150)
        self.assertLess(again, big - 150)

    def test_this_process_memory_does_not_leak_in(self):
        held = bytearray(300 * 2**20)
        held[::4096] = bytes(len(held[::4096]))
        reading = corpora.run_once([sys.executable, '-c', 'pass'])[1]
        del held
        self.assertLess(reading, 150)

    def test_failure_is_an_error(self):
        with self.assertRaises(RuntimeError):
            corpora.run_once([sys.executable, '-c', 'raise SystemExit(3)'])


if __name__ == '__main__':
    unittest.main()
