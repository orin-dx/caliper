#!/usr/bin/env python3
"""Tests for scripts/checker_modes.py's parsing and diff logic, on small fixtures. Run: python3 scripts/test_checker_modes.py"""
import os
import re
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import checker_modes as cm

REACT = '/r/node_modules/@types/react/index.d.ts'


def read(path):
    with open(path) as f:
        return f.read()


def facts(type='string', required=False, default=None, react=False):
    return {'type': type, 'required': required, 'default': default, 'react_declared': react}


class SplitTop(unittest.TestCase):
    def test_splits_only_at_the_top_level(self):
        self.assertEqual(cm.split_top('a | Array<b | c> | { d: e | f } | "x|y"', '|'),
                         ['a ', ' Array<b | c> ', ' { d: e | f } ', ' "x|y"'])

    def test_an_arrow_is_not_a_closing_angle_bracket(self):
        self.assertEqual(cm.split_top('(a: string) => b | c', '|'), ['(a: string) => b ', ' c'])


class LooseType(unittest.TestCase):
    def test_spelling_that_does_not_change_the_meaning_is_removed(self):
        self.assertEqual(cm.loose_type('boolean | "true" | "false"'), cm.loose_type('"false" | "true" | boolean'))
        self.assertEqual(cm.loose_type('string | undefined'), cm.loose_type('string'))
        self.assertEqual(cm.loose_type('Array<string>'), cm.loose_type('string[]'))
        self.assertEqual(cm.loose_type('{ a?: string | undefined; }'), cm.loose_type('{ a?: string; }'))

    def test_a_union_after_an_arrow_is_ordered_too(self):
        self.assertEqual(cm.loose_type('() => Window | HTMLElement | null'),
                         cm.loose_type('() => null | HTMLElement | Window'))

    def test_a_rest_or_optional_parameter_marker_is_a_difference(self):
        self.assertNotEqual(cm.loose_type('(...args: any[]) => void'), cm.loose_type('(args: any[]) => void'))
        self.assertNotEqual(cm.loose_type('(affixed?: boolean) => void'),
                            cm.loose_type('(affixed: undefined | boolean) => void'))

    def test_a_different_type_is_a_difference(self):
        self.assertNotEqual(cm.loose_type('string'), cm.loose_type('number'))

    def test_none_stays_none(self):
        self.assertIsNone(cm.loose_type(None))


class PropFacts(unittest.TestCase):
    def test_reads_raw_over_name_and_the_default_value(self):
        p = {'type': {'name': 'enum', 'raw': '"a" | "b"'}, 'required': True, 'defaultValue': {'value': "'a'"}}
        self.assertEqual(cm.prop_facts(p), {'type': '"a" | "b"', 'required': True, 'default': "'a'",
                                            'react_declared': False})

    def test_a_null_default_and_a_react_parent(self):
        p = {'type': {'name': 'string'}, 'required': False, 'defaultValue': None, 'parent': {'fileName': REACT}}
        self.assertEqual(cm.prop_facts(p), {'type': 'string', 'required': False, 'default': None,
                                            'react_declared': True})


class Differing(unittest.TestCase):
    def test_reports_each_kind_of_difference(self):
        structural = {'a': facts('string'), 'b': facts('number', required=True), 'gone': facts(), 'same': facts()}
        checker = {'a': facts('number'), 'b': facts('number', required=False), 'new': facts(), 'same': facts()}
        rows = {name: (kinds, cosmetic) for name, kinds, cosmetic, _, _ in cm.differing(structural, checker)}
        self.assertEqual(rows, {'a': (['type'], False), 'b': (['required'], False), 'gone': (['only_structural'], False),
                                'new': (['only_checker'], False)})

    def test_a_spelling_only_difference_is_cosmetic(self):
        rows = cm.differing({'a': facts('string | undefined')}, {'a': facts('string')})
        self.assertEqual([(kinds, cosmetic) for _, kinds, cosmetic, _, _ in rows], [(['type'], True)])

    def test_identical_props_have_no_difference(self):
        self.assertEqual(cm.differing({'a': facts()}, {'a': facts()}), [])


class DiffRuns(unittest.TestCase):
    def test_counts_react_declared_apart_and_lists_every_row(self):
        structural = {('f.tsx', 'Button'): {'onClick': facts('() => void'), 'aria-label': facts(react=True)},
                      ('f.tsx', 'Only'): {'x': facts()}}
        checker = {('f.tsx', 'Button'): {'onClick': facts('(args: any[]) => void')},
                   ('g.tsx', 'New'): {'y': facts()}}
        rows, summary = cm.diff_runs('batch', 'antd', structural, checker)
        self.assertEqual([(r['component'], r['prop'], r['react_declared']) for r in rows],
                         [('Button', 'aria-label', True), ('Button', 'onClick', False)])
        self.assertEqual((summary['components_compared'], summary['components_differing']), (1, 1))
        self.assertEqual((summary['props_differing'], summary['react_declared'], summary['real'], summary['type'],
                          summary['only_structural']), (2, 1, 1, 1, 0))
        self.assertEqual((summary['components_only_structural'], summary['components_only_checker']), (1, 1))
        self.assertEqual(rows[1]['mode'], 'batch')
        self.assertEqual(rows[1]['corpus'], 'antd')


class Categorise(unittest.TestCase):
    def test_names_the_kind_of_change(self):
        a, b = facts('(...args: any[]) => void'), facts('(args: any[]) => void')
        self.assertEqual(cm.categorise(['type'], a, b), 'parameter_marker_lost')
        self.assertEqual(cm.categorise(['type'], facts('(x?: boolean) => void'), facts('(x: undefined | boolean) => void')),
                         'parameter_marker_lost')
        self.assertEqual(cm.categorise(['type'], facts('StyleProp<Spacing>'), facts('"xs" | "sm" | "md" | "lg" | "xl" | number')),
                         'checker_longer')
        self.assertEqual(cm.categorise(['type'], facts('"xs" | "sm" | "md" | "lg" | "xl" | number'), facts('Size')),
                         'checker_shorter')
        self.assertEqual(cm.categorise(['type'], facts('string'), facts('number')), 'other_type')
        # An object type's optional member is not an optional parameter.
        self.assertEqual(cm.categorise(['type'], facts('string | { base?: string; S?: string }'),
                                       facts('string | ResponsiveProp<string>')), 'other_type')
        # A callback parameter's own arrow does not end the outer parameter list.
        self.assertEqual(cm.categorise(['type'], facts('(cb: () => void, ...rest: any[]) => void'),
                                       facts('(cb: () => void, rest: any[]) => void')), 'parameter_marker_lost')
        self.assertEqual(cm.categorise(['required', 'type'], facts(), facts()), 'required+type')
        self.assertEqual(cm.categorise(['only_checker'], None, facts()), 'only_checker')


class DistinctPairs(unittest.TestCase):
    def row(self, component, structural, checker, **extra):
        row = {'mode': 'batch', 'corpus': 'chakra', 'file': 'f.tsx', 'component': component, 'prop': 'p',
               'kinds': ['type'], 'category': 'other_type', 'cosmetic': False, 'react_declared': False,
               'structural': structural, 'checker': checker}
        row.update(extra)
        return row

    def test_groups_the_same_difference_and_skips_react_and_spelling_rows(self):
        rows = [self.row('A', facts('string'), facts('number')), self.row('B', facts('string'), facts('number')),
                self.row('C', facts('x'), facts('y')), self.row('D', facts('x'), facts('y'), react_declared=True),
                self.row('E', facts('x'), facts('y'), cosmetic=True)]
        pairs = cm.distinct_pairs(rows)
        self.assertEqual([(p['count'], p['example']['component']) for p in pairs], [(2, 'A'), (1, 'C')])
        self.assertEqual(pairs[0]['corpora'], {'chakra': 2})


class OracleVerdicts(unittest.TestCase):
    # A real report: `node apps/oracle/src/oracle.ts` on antd, 2026-11-02 (its `verdicts` and `props_shared`).
    REAL = {'props_shared': 3075, 'verdicts': {'identical': 1772, 'spelled': 1096, 'less precise': 206, 'wrong': 0,
                                               'checker any/error': 1}}

    def test_reads_every_verdict_of_a_real_report(self):
        self.assertEqual(cm.oracle_summary(self.REAL), {
            'shared_props': 3075, 'agree': round((1772 + 1096) / 3075, 4), 'less_precise': round(206 / 3075, 4),
            'wrong': 0.0, 'checker_any_error': round(1 / 3075, 4)})

    def test_no_shared_props_gives_no_shares_not_zeros(self):
        summary = cm.oracle_summary({'props_shared': 0, 'verdicts': {name: 0 for name in cm.ORACLE_VERDICTS}})
        self.assertEqual((summary['shared_props'], summary['agree'], summary['less_precise'], summary['wrong']),
                         (0, None, None, None))

    def test_the_less_precise_share_is_not_zero_when_the_oracle_reports_some(self):
        self.assertGreater(cm.oracle_summary(self.REAL)['less_precise'], 0.06)

    def test_a_verdict_key_the_oracle_does_not_emit_is_an_error_not_a_zero(self):
        with self.assertRaises(KeyError):
            cm.oracle_summary({'props_shared': 1, 'verdicts': {'identical': 1, 'spelled': 0, 'less_precise': 0,
                                                               'wrong': 0, 'checker any/error': 0}})

    def test_the_verdict_names_match_the_oracles_source(self):
        source = read(os.path.join(cm.ROOT, 'apps', 'oracle', 'src', 'oracle-compare.ts'))
        listed = re.search(r'export const VERDICTS: Verdict\[\] = \[(.*?)\]', source).group(1)
        self.assertEqual(re.findall(r"'([^']+)'", listed), list(cm.ORACLE_VERDICTS))


class RunJson(unittest.TestCase):
    def test_returns_the_json_after_any_banner(self):
        self.assertEqual(cm.run_json([sys.executable, '-c', 'print("banner"); print(\'{"a": 1}\')']), '{"a": 1}\n')

    def test_a_failing_command_raises_with_the_command_and_its_stderr(self):
        with self.assertRaises(RuntimeError) as raised:
            cm.run_json([sys.executable, '-c', 'import sys; sys.stderr.write("boom happened"); sys.exit(3)'])
        message = str(raised.exception)
        self.assertIn('exited with 3', message)
        self.assertIn('boom happened', message)
        self.assertIn('-c', message)

    def test_output_with_no_json_raises_instead_of_returning_an_empty_string(self):
        with self.assertRaises(RuntimeError) as raised:
            cm.run_json([sys.executable, '-c', 'print("no braces here")'])
        self.assertIn('no JSON', str(raised.exception))


class ProcessStats(unittest.TestCase):
    def test_the_checker_off_session_reports_wall_time_and_memory(self):
        stats = cm.process_stats('off', [(1.0, 100.0), (3.0, 120.0), (2.0, 110.0)])
        self.assertEqual(stats['wall_ms']['median'], 2000)
        self.assertEqual(stats['rss_mib'], {'median': 110, 'min': 100, 'max': 120})

    def test_an_on_demand_session_reports_none_because_the_script_sleeps_and_asks_twice(self):
        self.assertEqual(cm.process_stats('on-demand', [(5.0, 100.0)]), {})


class CheckerMemory(unittest.TestCase):
    """The checker is found by what it is, not by a string in somebody's command line: the child of this run's session
    driver whose executable is the `tsc` binary. Rows are `ps -ax -o pid=,ppid=,rss=,command=`."""
    TSC = '/r/typescript-beta/bin/tsc'
    SCRIPT = '/r/scripts/checker_modes_session.mjs'
    MARKER = '/tmp/work/antd-on-demand-150.cache'
    OPTIONS = '{"semanticTypes": "on-demand", "tscPath": "' + TSC + '", "cacheDir": "' + MARKER + '"}'
    # The measuring helper and the Node driver both carry the options, and so the tsc path, in their command lines.
    HELPER = ' 100 1 40960 /usr/bin/python3 -I /r/scripts/corpora.py measure /tmp/r.json -- node ' + SCRIPT + ' --options ' + OPTIONS
    DRIVER = ' 200 100 4194304 node ' + SCRIPT + ' --options ' + OPTIONS + ' --count 50'
    # What `ps` shows for the checker: the platform binary the `bin/tsc` launcher resolves to, run in `--api` mode.
    NATIVE = '/r/node_modules/.pnpm/@typescript+typescript-darwin-arm64@7.1.0-beta/node_modules/@typescript/typescript-darwin-arm64/lib/tsc'
    CHECKER = ' 300 200 204800 ' + NATIVE + ' --api --async --cwd /p'

    def memory(self, *rows, tsc=None):
        return cm.checker_rss_mib('\n'.join(rows) + '\n', tsc or self.TSC, self.SCRIPT, self.MARKER, 'node')

    def test_the_checker_is_not_the_driver_that_names_its_path_and_uses_more_memory(self):
        self.assertEqual(self.memory(self.HELPER, self.DRIVER, self.CHECKER), 200)

    def test_the_largest_of_several_checker_processes_the_driver_spawned(self):
        second = ' 301 200 307200 ' + self.NATIVE + ' --api --async --cwd /q'
        self.assertEqual(self.memory(self.HELPER, self.DRIVER, self.CHECKER, second), 300)

    def test_no_checker_process_is_none_not_the_driver_or_the_helper(self):
        self.assertIsNone(self.memory(self.HELPER, self.DRIVER))

    def test_a_child_that_is_not_named_like_the_configured_binary_is_not_the_checker(self):
        self.assertIsNone(self.memory(self.HELPER, self.DRIVER, self.CHECKER, tsc='/nowhere/bin/tsgo'))

    def test_a_tsc_child_that_does_not_run_in_api_mode_is_not_the_checker(self):
        version = ' 302 200 999999 ' + self.NATIVE + ' --version'
        self.assertIsNone(self.memory(self.DRIVER, version))
        self.assertEqual(self.memory(self.DRIVER, version, self.CHECKER), 200)

    def test_a_grandchild_is_not_the_drivers_checker(self):
        grandchild = ' 303 300 999999 ' + self.NATIVE + ' --api --async --cwd /g'
        self.assertEqual(self.memory(self.DRIVER, self.CHECKER, grandchild), 200)

    def test_another_runs_driver_and_checker_are_not_this_ones(self):
        other_options = self.OPTIONS.replace(self.MARKER, '/tmp/work/shadcn-on-demand-150.cache')
        other = ' 400 1 4194304 node ' + self.SCRIPT + ' --options ' + other_options
        other_checker = ' 401 400 999999 ' + self.NATIVE + ' --api --async --cwd /z'
        self.assertEqual(self.memory(self.DRIVER, self.CHECKER, other, other_checker), 200)
        self.assertIsNone(self.memory(self.DRIVER, other, other_checker))

    def test_an_orphan_with_the_same_executable_and_a_process_that_only_mentions_the_path_do_not_count(self):
        orphan = ' 500 1 999999 ' + self.NATIVE + ' --api --async --cwd /old'
        viewer = ' 501 200 999999 less ' + self.TSC
        self.assertEqual(self.memory(self.DRIVER, self.CHECKER, orphan, viewer), 200)
        self.assertIsNone(self.memory(self.DRIVER, orphan, viewer))

    def test_a_listing_that_does_not_parse_is_none(self):
        self.assertIsNone(cm.checker_rss_mib('', self.TSC, self.SCRIPT, self.MARKER, 'node'))
        self.assertIsNone(cm.checker_rss_mib('not a listing\n', self.TSC, self.SCRIPT, self.MARKER, 'node'))

    def sampler(self, answers):
        calls = iter(answers)
        sampler = cm.TscSampler(self.TSC, self.SCRIPT, self.MARKER, 'node', interval=0.01,
                                ps=lambda: next(calls, (0, '')))
        sampler.start()
        while sampler.samples < len(answers):
            pass
        return sampler

    def listing(self, checker_kib):
        return '\n'.join([self.HELPER, self.DRIVER, f' 300 200 {checker_kib} {self.NATIVE} --api']) + '\n'

    def test_the_peak_is_the_largest_checker_size_seen_across_samples(self):
        sampler = self.sampler([(0, self.listing(102400)), (0, self.listing(307200)), (0, self.listing(204800))])
        self.assertEqual(sampler.finish(), 300)

    def test_a_sampler_that_never_saw_the_checker_reports_none_and_prints_na(self):
        sampler = self.sampler([(0, self.HELPER + '\n' + self.DRIVER + '\n'), (1, '')])
        self.assertIsNone(sampler.finish())
        self.assertEqual(cm.mib(None), 'n/a (no checker process seen)')
        self.assertEqual(cm.mib(300), '300 MiB')


class DiagnosticWording(unittest.TestCase):
    """The outcome of an on-demand answer is read from the wording of the checker's diagnostics. If a message is reworded
    the outcome would silently read `none`, so the phrases are pinned to the Rust that emits them."""
    PHRASES = ('has not answered yet', 'kept their structural result', 'resolved by the TypeScript checker',
               'backend unavailable')

    def test_every_phrase_the_classifiers_match_is_emitted_by_the_checker_code(self):
        rust = read(os.path.join(cm.ROOT, 'crates', 'core', 'src', 'pipeline', 'semantic_hook.rs')) + \
            read(os.path.join(cm.ROOT, 'crates', 'core', 'src', 'types', 'semantic.rs'))
        for phrase in self.PHRASES:
            self.assertIn(phrase, rust)

    def test_the_session_script_and_the_batch_parser_match_those_phrases(self):
        script = read(cm.SESSION_SCRIPT)
        for phrase in self.PHRASES:
            self.assertIn(phrase, script)
        parser = read(os.path.join(cm.ROOT, 'scripts', 'checker_modes.py'))
        for phrase in ('resolved by the TypeScript checker', 'kept their structural result'):
            self.assertIn(phrase, parser)


class AddonOptions(unittest.TestCase):
    def test_a_session_is_created_with_the_options_the_addon_passes(self):
        options = cm.addon_session_options('/src', 'on-demand', 150, '/t/tsc', '/cache')
        self.assertEqual(options, {'srcDirs': ['/src'], 'loadDependencies': True, 'collapseInheritedProps': 'none',
                                   'lazy': True, 'cacheDir': '/cache', 'semanticTypes': 'on-demand',
                                   'semanticWaitMs': 150, 'tscPath': '/t/tsc'})

    def test_the_checker_off_session_names_no_checker_options(self):
        options = cm.addon_session_options('/src', 'off', None, None, '/cache')
        self.assertNotIn('semanticWaitMs', options)
        self.assertNotIn('tscPath', options)
        self.assertEqual(options['semanticTypes'], 'off')

    def test_the_defaults_match_the_addons_source(self):
        source = read(os.path.join(cm.ROOT, 'packages', 'storybook', 'src', 'provider.ts'))
        self.assertIn("collapseInheritedProps: options.collapseInheritedProps ?? 'none'", source)
        self.assertIn('loadDependencies: options.loadDependencies ?? true', source)
        self.assertNotIn('cache: options.cache ??', source)
        self.assertNotIn('htmlAttributes: options.htmlAttributes ??', source)

    def test_the_cli_runs_use_the_same_props_settings(self):
        self.assertEqual(cm.COMMON[cm.COMMON.index('--collapse-inherited-props') + 1], 'none')
        self.assertIn('--load-dependencies', cm.COMMON)


class SessionScript(unittest.TestCase):
    def test_nothing_measures_the_checker_between_two_requests(self):
        source = read(cm.SESSION_SCRIPT)
        for process_call in ('pgrep', 'execFile', 'child_process'):
            self.assertNotIn(process_call, source)

    def test_the_script_takes_its_options_from_the_parent(self):
        source = read(cm.SESSION_SCRIPT)
        self.assertIn("arg('--options')", source)
        self.assertNotIn('cache: false', source)


class ByComponent(unittest.TestCase):
    def test_keys_a_component_by_file_relative_to_the_root(self):
        out = cm.by_component({'Button': {'displayName': 'Button', 'filePath': '/r/src/Button.tsx',
                                          'props': {'a': {'type': {'name': 'string'}, 'required': False,
                                                          'defaultValue': None}}}}, '/r')
        self.assertEqual(list(out), [(os.path.join('src', 'Button.tsx'), 'Button')])
        self.assertEqual(out[(os.path.join('src', 'Button.tsx'), 'Button')]['a']['type'], 'string')


class ComponentIdentity(unittest.TestCase):
    def test_a_component_without_a_file_path_is_an_error_not_an_empty_path(self):
        with self.assertRaises(KeyError):
            cm.by_component({'A': {'displayName': 'A', 'props': {}}, 'B': {'displayName': 'A', 'props': {}}}, '/r')


class BatchBackend(unittest.TestCase):
    def test_sums_the_checker_diagnostics_by_reason(self):
        canonical = {'diagnostics': [
            {'code': 'SEMANTIC_BACKEND', 'message': '211 component(s) resolved by the TypeScript checker'},
            {'code': 'SEMANTIC_BACKEND', 'message': '37 component(s) kept their structural result: not exported'},
            {'code': 'SEMANTIC_BACKEND', 'message': '5 component(s) kept their structural result: not exported'},
            {'code': 'SEMANTIC_BACKEND', 'message': 'the checker could not start'},
            {'code': 'OTHER', 'message': '9 component(s) resolved by the TypeScript checker'}]}
        self.assertEqual(cm.batch_backend(canonical), {'resolved': 211, 'kept_structural': 42,
                                                      'reasons': {'not exported': 42},
                                                      'other': ['the checker could not start']})


class Sessions(unittest.TestCase):
    def result(self, outcomes, second=None):
        def row(i, outcome):
            return {'key': f'C{i}', 'ms': 10.0 + i, 'cumulative_ms': 10.0 * (i + 1), 'present': True, 'file': 'f.tsx',
                    'display_name': f'C{i}', 'props': {'a': facts()}, 'backend': {'messages': [], 'outcome': outcome}}
        return {'create_ms': 5.0, 'index_ms': 20.0, 'components_listed': 99, 'tsc_rss_mib': 200,
                'results': [row(i, o) for i, o in enumerate(outcomes)],
                'second_pass': [row(i, o) for i, o in enumerate(second or outcomes)]}

    def test_first_load_reads_the_cumulative_times(self):
        load = cm.first_load(self.result(['none'] * 60))
        self.assertEqual((load['session_start_ms'], load['first_component_ms'], load['first_10_ms']), (25, 10, 100))
        self.assertEqual((load['first_50_ms'], load['asked']), (500, 60))
        self.assertEqual(load['components_listed'], 99)

    def test_first_load_does_not_report_50_components_when_fewer_were_asked(self):
        load = cm.first_load(self.result(['none'] * 12))
        self.assertEqual((load['asked'], load['first_10_ms'], load['first_50_ms']), (12, 100, None))
        self.assertEqual(cm.first_load(self.result(['none'] * 5))['first_10_ms'], None)

    def test_on_demand_summary_separates_timeouts_from_the_checkers_own_fallbacks(self):
        summary = cm.on_demand_summary(self.result(['checker', 'timed_out', 'fallback', 'checker'],
                                                   ['checker', 'checker', 'fallback', 'checker']))
        self.assertEqual(summary['first_pass']['counts'], {'checker': 2, 'timed_out': 1, 'fallback': 1})
        self.assertEqual((summary['first_pass']['checker_share'], summary['first_pass']['timed_out']), (0.5, 1))
        self.assertEqual(summary['second_pass']['checker_share'], 0.75)
        self.assertEqual(summary['tsc_rss_mib'], 200)

    def test_no_answers_is_not_a_share_of_zero(self):
        empty = self.result([])
        empty['second_pass'] = []
        summary = cm.on_demand_summary(empty)
        self.assertEqual((summary['first_pass']['checker_share'], summary['first_pass']['timed_out_share']),
                         (None, None))
        self.assertEqual(cm.percent(None), 'n/a')
        self.assertEqual(cm.percent(0.768), '77%')

    def test_session_props_skip_answers_without_a_file_or_props(self):
        result = self.result(['none', 'none'])
        result['results'][1]['props'] = None
        self.assertEqual(list(cm.session_props(result)), [('f.tsx', 'C0')])


if __name__ == '__main__':
    unittest.main()
