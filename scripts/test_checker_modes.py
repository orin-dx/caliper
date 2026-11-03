#!/usr/bin/env python3
"""Tests for scripts/checker_modes.py's parsing and diff logic, on small fixtures. Run: python3 scripts/test_checker_modes.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import checker_modes as cm

REACT = '/r/node_modules/@types/react/index.d.ts'


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
                         'alias_expanded')
        self.assertEqual(cm.categorise(['type'], facts('"xs" | "sm" | "md" | "lg" | "xl" | number'), facts('Size')),
                         'alias_collapsed')
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


class ByComponent(unittest.TestCase):
    def test_keys_a_component_by_file_relative_to_the_root(self):
        out = cm.by_component({'Button': {'displayName': 'Button', 'filePath': '/r/src/Button.tsx',
                                          'props': {'a': {'type': {'name': 'string'}, 'required': False,
                                                          'defaultValue': None}}}}, '/r')
        self.assertEqual(list(out), [(os.path.join('src', 'Button.tsx'), 'Button')])
        self.assertEqual(out[(os.path.join('src', 'Button.tsx'), 'Button')]['a']['type'], 'string')


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
        load = cm.first_load(self.result(['none'] * 12))
        self.assertEqual((load['session_start_ms'], load['first_component_ms'], load['first_10_ms']), (25, 10, 100))
        self.assertEqual(load['first_50_ms'], 120)
        self.assertEqual(load['components_listed'], 99)

    def test_on_demand_summary_separates_timeouts_from_the_checkers_own_fallbacks(self):
        summary = cm.on_demand_summary(self.result(['checker', 'timed_out', 'fallback', 'checker'],
                                                   ['checker', 'checker', 'fallback', 'checker']))
        self.assertEqual(summary['first_pass']['counts'], {'checker': 2, 'timed_out': 1, 'fallback': 1})
        self.assertEqual((summary['first_pass']['checker_share'], summary['first_pass']['timed_out']), (0.5, 1))
        self.assertEqual(summary['second_pass']['checker_share'], 0.75)
        self.assertEqual(summary['tsc_rss_mib'], 200)

    def test_session_props_skip_answers_without_a_file_or_props(self):
        result = self.result(['none', 'none'])
        result['results'][1]['props'] = None
        self.assertEqual(list(cm.session_props(result)), [('f.tsx', 'C0')])


if __name__ == '__main__':
    unittest.main()
