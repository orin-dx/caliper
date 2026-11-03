"""Tests for scripts/collapse_relevance.py: the parts that read outputs and time output, not the runs themselves."""
import json
import os
import tempfile
import unittest

import collapse_relevance as c


def write(data):
    handle = tempfile.NamedTemporaryFile('w', suffix='.json', delete=False)
    json.dump(data, handle)
    handle.close()
    return handle.name


class CollapseRelevanceTests(unittest.TestCase):
    def test_time_output_is_read_for_wall_seconds_and_peak_rss(self):
        stderr = '        0.28 real         0.20 user         0.05 sys\n  219152384  maximum resident set size\n'
        self.assertEqual(c.REAL.search(stderr).group(1), '0.28')
        self.assertEqual(int(c.RSS.search(stderr).group(1)) / 2**20, 209.0)

    def test_spread_is_the_median_and_the_range(self):
        self.assertEqual(c.spread([3.0, 1.0, 2.0]), (2.0, (1.0, 3.0)))

    def test_the_digest_ignores_timing_fields_and_nothing_else(self):
        one = write({'components': {'A': {'props': ['p']}}, 'stats': {'durationMs': 12}})
        two = write({'components': {'A': {'props': ['p']}}, 'stats': {'durationMs': 99}})
        other = write({'components': {'A': {'props': ['q']}}, 'stats': {'durationMs': 12}})
        try:
            self.assertEqual(c.canonical_digest(one), c.canonical_digest(two))
            self.assertNotEqual(c.canonical_digest(one), c.canonical_digest(other))
        finally:
            for path in (one, two, other):
                os.unlink(path)

    def test_a_bench_that_ignored_the_collapse_flag_is_not_counted_as_having_run_the_mode(self):
        self.assertTrue(c.ran_mode({'collapse': 'none'}, 'none'))
        self.assertTrue(c.ran_mode({}, None))
        self.assertFalse(c.ran_mode({}, 'none'))
        self.assertFalse(c.ran_mode({'collapse': 'dependencies'}, 'none'))

    def test_only_non_html_inheritance_layers_are_collapsed_layers(self):
        path = write({'components': {
            'A': {'inheritance': [{'typeName': 'Big'}, {'typeName': 'HTMLAttributes', 'htmlElement': 'div'}]},
            'B': {'inheritance': [{'typeName': 'Other'}]},
            'C': {},
        }})
        try:
            self.assertEqual(c.collapsed_layers(path), 2)
        finally:
            os.unlink(path)

    def test_a_spread_is_printed_with_its_range(self):
        self.assertEqual(c.fmt((1.5, (1.0, 2.0))), '1.5 (1 to 2)')


if __name__ == '__main__':
    unittest.main()
