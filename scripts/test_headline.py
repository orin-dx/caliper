#!/usr/bin/env python3
"""Tests for the weekly headline: its arithmetic, the conditions that open the issue, and the top of the summary.
Run: python3 scripts/test_headline.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import headline
import oracle_data
import weekly


def scored(universe=1000, identical=700, spelled=100, less=100, wrong=2, no_type=8, prop=30, collapsed=60, components=40,
           not_judged=0):
    """The oracle's score of Caliper over the components it reports (`caliper_reported`)."""
    return {'universe': universe, 'components': components,
            'verdicts': {'identical': identical, 'spelled': spelled, 'less precise': less, 'wrong': wrong,
                         'checker any/error': no_type, 'not judged': not_judged},
            'missing_by_reason': {'component': 0, 'failed_file': 0, 'prop': prop, 'collapsed': collapsed}}


def oracle(**parts):
    return {'shared_props': 1, 'agree': 1.0, 'wrong': 0.0, 'tools': {'caliper_reported': scored(**parts)}}


HEADLINE = {'components': 40, 'props': 1000, 'correct': 800, 'less_precise': 100, 'wrong': 2, 'no_checker_type': 8,
            'not_judged': 0, 'missing': 30, 'named_only': 60}


def entry(h=None, ms=1000, rss_mib=400, **extra):
    out = {'caliper': {'ms': ms, 'rss_mib': rss_mib}, **extra}
    if h is not None:
        out['headline'] = h
    return out


class Compute(unittest.TestCase):
    def test_counts_every_prop_of_a_reported_component_once(self):
        self.assertEqual(headline.compute(oracle()), HEADLINE)

    def test_correct_is_identical_and_spelled(self):
        h = headline.compute(oracle(identical=1, spelled=2, less=0, wrong=0, no_type=0, prop=0, collapsed=0, universe=3))
        self.assertEqual((h['correct'], h['props']), (3, 3))

    def test_a_prop_the_oracle_did_not_judge_is_counted_apart_from_agreement_and_from_wrong(self):
        h = headline.compute(oracle(identical=690, not_judged=10))
        self.assertEqual((h['correct'], h['not_judged'], h['wrong'], h['less_precise']), (790, 10, 2, 100))
        self.assertNotIn('other', h)
        self.assertEqual(headline.judged(h), 892)

    def test_every_verdict_of_the_oracle_has_a_headline_kind(self):
        kinds = {headline.kind_of(v) for v in oracle_data.VERDICTS}
        self.assertEqual(kinds, {'correct', 'less_precise', 'wrong', 'no_checker_type', 'not_judged'})
        self.assertTrue(kinds <= set(headline.KINDS))
        with self.assertRaises(KeyError):
            headline.kind_of('a verdict the vocabulary does not have')

    def test_parts_that_do_not_add_up_are_counted_as_other(self):
        self.assertEqual(headline.compute(oracle(universe=1003))['other'], 3)
        self.assertNotIn('other', headline.compute(oracle()))

    def test_no_headline_without_an_oracle_or_before_it_existed(self):
        self.assertIsNone(headline.compute(None))
        self.assertIsNone(headline.compute({'error': 'tsx failed'}))
        self.assertIsNone(headline.compute({'tools': {'caliper': scored()}}))

    def test_wrong_is_of_the_props_that_can_be_judged(self):
        self.assertEqual(headline.judged(HEADLINE), 902)
        self.assertEqual(headline.wrong_rate(HEADLINE), 2 / 902)
        self.assertEqual(headline.share(HEADLINE, 'named_only'), 0.06)
        self.assertEqual(headline.wrong_rate({**HEADLINE, 'correct': 0, 'less_precise': 0, 'wrong': 0}), 0.0)
        self.assertEqual(headline.share({**HEADLINE, 'props': 0}, 'correct'), 0.0)


class Findings(unittest.TestCase):
    def test_wrong_above_the_gate_opens_an_issue(self):
        self.assertEqual(headline.findings('antd', entry(HEADLINE), None),
                         ['antd: 2 of the 902 props whose type can be judged are wrong, 0.22%; the gate is 0.1%'])

    def test_above_the_gate_and_rising_says_where_it_was(self):
        old = entry({**HEADLINE, 'wrong': 1})
        self.assertEqual(headline.findings('antd', entry(HEADLINE), old),
                         ['antd: 2 of the 902 props whose type can be judged are wrong, 0.22%, up from 1 of 901, 0.11% last week; '
                          'the gate is 0.1%'])

    def test_inside_the_gate_a_rise_still_opens_an_issue(self):
        h = {**HEADLINE, 'correct': 899, 'less_precise': 100, 'wrong': 1}
        self.assertEqual(headline.findings('antd', entry(h), entry({**h, 'wrong': 0})),
                         ['antd: wrong types rose to 1 of 1,000, 0.10%, up from 0 of 999, 0.00% last week'])

    def test_inside_the_gate_and_not_rising_is_quiet(self):
        h = {**HEADLINE, 'correct': 899, 'less_precise': 100, 'wrong': 1}
        self.assertEqual(headline.findings('antd', entry(h), None), [])
        self.assertEqual(headline.findings('antd', entry(h), entry(h)), [])
        self.assertEqual(headline.findings('antd', entry(h), entry({**h, 'wrong': 2})), [])

    def test_a_whole_run_over_a_budget_opens_an_issue(self):
        self.assertEqual(headline.findings('chakra', entry(ms=2500, rss_mib=700), None),
                         ['chakra: a whole run\'s time is 2,500 ms, over the 2,000 ms budget',
                          "chakra: a whole run's peak memory is 700 MiB, over the 600 MiB budget"])

    def test_a_run_inside_the_budget_is_quiet_however_much_it_drifted(self):
        self.assertEqual(headline.findings('chakra', entry(ms=2000, rss_mib=600), entry(ms=500, rss_mib=100)), [])

    def test_a_record_without_a_headline_or_without_numbers_is_quiet(self):
        self.assertEqual(headline.findings('antd', {}, None), [])
        self.assertEqual(headline.findings('antd', {'caliper': {}}, entry(HEADLINE)), [])


class Summary(unittest.TestCase):
    def record(self, **corpora):
        return {'caliper': 'x', 'runs': 1, 'reference_version': weekly.REFERENCE_VERSION, 'corpora': corpora}

    def test_it_explains_the_numbers_even_when_the_oracle_did_not_run(self):
        self.assertEqual(headline.markdown(self.record(antd=entry())), [headline.HOW_TO_READ, ''])

    def test_the_table_gives_each_kind_as_a_share_of_the_props(self):
        lines = headline.markdown(self.record(antd=entry(HEADLINE)))
        self.assertEqual(lines[2:4], ['### Headline: are the types Storybook shows right?', ''])
        self.assertEqual(lines[-4:-1], ['| Corpus | Props | Correct | Less precise | Wrong | No checker type | Not judged | Missing '
                                       '| Named only |',
                                       '| --- | --: | --: | --: | --: | --: | --: | --: | --: |',
                                       '| antd | 1,000 | 80.0% | 10.0% | 0.20% | 0.8% | 0.0% | 3.0% | 6.0% |'])

    def test_an_oracle_that_failed_is_named(self):
        lines = headline.markdown(self.record(antd=entry(oracle={'error': 'tsx failed'})))
        self.assertIn('| antd | unavailable: tsx failed | | | | | | | |', lines)

    def test_the_budgets_and_the_gate_in_the_text_are_the_ones_used(self):
        self.assertIn('more than 0.1% wrong', headline.HOW_TO_READ)
        self.assertIn('over 2 s or 600 MiB', headline.HOW_TO_READ)


class WeeklyUsesIt(unittest.TestCase):
    def record(self, ms, h=HEADLINE):
        return {'caliper': 'x', 'runs': 1, 'reference_version': weekly.REFERENCE_VERSION,
                'corpora': {'antd': {'caliper': {'ms': ms, 'rss_mib': 100, 'json_mb': 1}, 'headline': h,
                                     'recall': {'caliper': {'components': [9, 10]}}}}}

    def test_a_slower_week_is_information_not_a_finding(self):
        now, before = self.record(1500, {**HEADLINE, 'wrong': 0}), self.record(1000, {**HEADLINE, 'wrong': 0})
        self.assertEqual(weekly.findings(now, before), [])
        self.assertEqual(weekly.changes(now, before), ['antd: time 1000 -> 1500'])

    def test_findings_are_the_headline_ones(self):
        self.assertEqual(weekly.findings(self.record(1000), {}),
                         ['antd: 2 of the 902 props whose type can be judged are wrong, 0.22%; the gate is 0.1%'])

    def test_the_summary_leads_with_the_headline_and_keeps_findings_apart_from_changes(self):
        text = weekly.markdown({**self.record(1000), 'corpora': {}}, ['a finding'], None, ['a change'])
        self.assertLess(text.index('How to read these numbers'), text.index('### Time and memory'))
        self.assertIn('### Needs attention (opens the issue)\n\n- a finding', text)
        self.assertIn('### Changes since last week (information)\n\n- a change', text)
        self.assertNotIn('Worse than last week', text)


if __name__ == '__main__':
    unittest.main()
