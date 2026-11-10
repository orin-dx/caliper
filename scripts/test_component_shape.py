"""Tests that the weekly's `exclusion` and the oracle's `nonComponentReason` (apps/oracle/src/component-shape.ts) are one
rule: both read apps/oracle/src/component-shape.json, and both run the cases of
apps/oracle/tests/fixtures/component-shape-cases.json (apps/oracle/tests/component-shape.test.ts is the other half)."""
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import weekly

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORACLE = os.path.join(ROOT, 'apps', 'oracle')

with open(os.path.join(ORACLE, 'tests', 'fixtures', 'component-shape-cases.json')) as f:
    CASES = json.load(f)


class SharedRules(unittest.TestCase):
    def test_every_case_gets_the_reason_the_oracle_gets(self):
        for case in CASES:
            with self.subTest(case['why']):
                comp = {'name': case['name'], 'file': case['file'], 'props': case['props']}
                self.assertEqual(weekly.exclusion(comp), case['expected'])

    def test_the_cases_cover_every_reason(self):
        self.assertEqual({c['expected'] for c in CASES} - {None},
                         {'lib_members', 'hook', 'context_value', 'not_jsx', 'promise_props'})

    def test_the_oracle_reads_the_same_patterns_file(self):
        with open(os.path.join(ORACLE, 'src', 'component-shape.ts')) as f:
            source = f.read()
        self.assertIn("new URL('component-shape.json', import.meta.url)", source)
        self.assertEqual(weekly.SHAPE['libFile'], weekly.LIB_FILE.pattern)
        self.assertEqual(weekly.SHAPE['hook'], weekly.HOOK.pattern)


if __name__ == '__main__':
    unittest.main()
