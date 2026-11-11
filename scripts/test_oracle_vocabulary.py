"""Tests that the scripts read the oracle's verdict names from the one file the oracle writes them to
(apps/oracle/src/vocabulary.json, written from vocabulary.ts: apps/oracle/tests/vocabulary.test.ts fails while the two
differ), and that no script keeps a list of its own."""
import os
import re
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import headline
import oracle_data

SCRIPTS = os.path.dirname(os.path.abspath(__file__))


class Vocabulary(unittest.TestCase):
    def test_the_names_are_the_oracles(self):
        self.assertEqual(oracle_data.VERDICTS, ['identical', 'spelled', 'less precise', 'wrong', 'checker any/error',
                                                'not judged'])
        self.assertEqual(oracle_data.AGREE, ['identical', 'spelled'])
        self.assertTrue(set(oracle_data.AGREE) <= set(oracle_data.VERDICTS))

    def test_the_headline_counts_every_verdict_under_one_kind(self):
        self.assertEqual({headline.kind_of(v) for v in oracle_data.VERDICTS} - {'correct'},
                         set(headline.VERDICT_KINDS.values()))
        self.assertEqual(set(headline.VERDICT_KINDS), set(oracle_data.VERDICTS) - set(oracle_data.AGREE))

    def test_no_script_lists_the_verdicts_again(self):
        listed = re.compile(r"""['"]identical['"]\s*,\s*['"]spelled['"]""")
        for name in sorted(os.listdir(SCRIPTS)):
            if not name.endswith('.py') or name.startswith('test_'):
                continue
            with open(os.path.join(SCRIPTS, name)) as f:
                self.assertIsNone(listed.search(f.read()), f'{name} lists the verdicts; read scripts/oracle_data.py')


if __name__ == '__main__':
    unittest.main()
