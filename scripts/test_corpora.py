#!/usr/bin/env python3
"""Tests what corpora.py prunes and when it refetches. Run: python3 scripts/test_corpora.py"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import corpora


class Prune(unittest.TestCase):
    def test_keeps_declaration_files_and_drops_tests_stories_and_dist(self):
        with tempfile.TemporaryDirectory() as src:
            for rel in ['a.test.ts', 'b.stories.tsx', 'types/shared.d.ts', 'dist/x.d.ts', 'c.tsx']:
                path = os.path.join(src, rel)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                open(path, 'w').close()
            corpora.prune(src)
            left = sorted(os.path.relpath(os.path.join(c, f), src) for c, _, fs in os.walk(src) for f in fs)
            self.assertEqual(left, ['c.tsx', os.path.join('types', 'shared.d.ts')])


class Ready(unittest.TestCase):
    CORPUS = {'name': 'x', 'clone': 'x', 'commit': 'abc', 'src': 'src'}

    def checkout_with_marker(self, text):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        os.makedirs(os.path.join(d.name, 'x'))
        with open(os.path.join(d.name, 'x', corpora.MARKER), 'w') as f:
            f.write(text)
        return d.name

    def test_installed_corpus_pruned_under_an_older_rule_is_not_ready(self):
        self.assertIsNone(corpora.ready(self.CORPUS, self.checkout_with_marker('abc:installed'), sparse=False))

    def test_installed_corpus_at_the_current_prune_version_is_ready(self):
        marker = 'abc' + corpora.installed_marker()
        self.assertTrue(corpora.ready(self.CORPUS, self.checkout_with_marker(marker), sparse=False))


if __name__ == '__main__':
    unittest.main()
