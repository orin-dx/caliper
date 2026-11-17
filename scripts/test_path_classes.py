"""Tests that the scripts and the oracle class a declaration's path by one rule: both read
apps/oracle/src/path-classes.json, and both run the cases of apps/oracle/tests/fixtures/path-class-cases.json
(apps/oracle/tests/path-class.test.ts is the other half)."""
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import oracle_data

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORACLE = os.path.join(ROOT, 'apps', 'oracle')

with open(os.path.join(ORACLE, 'tests', 'fixtures', 'path-class-cases.json')) as f:
    CASES = json.load(f)


class SharedPathClasses(unittest.TestCase):
    def test_every_case_is_classed_as_the_oracle_classes_it(self):
        for case in CASES:
            with self.subTest(case['why']):
                self.assertEqual(
                    {'dependency': oracle_data.is_dependency(case['path']),
                     'reactTypes': oracle_data.is_react_types(case['path']),
                     'libFile': oracle_data.is_lib_file(case['path'])},
                    {key: case[key] for key in ('dependency', 'reactTypes', 'libFile')})

    def test_the_cases_hold_every_combination_the_rules_can_give(self):
        self.assertEqual({(c['dependency'], c['reactTypes'], c['libFile']) for c in CASES},
                         {(False, False, False), (True, False, False), (True, True, False), (True, False, True),
                          (False, False, True)})

    def test_the_oracle_reads_the_same_rules_file(self):
        with open(os.path.join(ORACLE, 'src', 'path-class.ts')) as f:
            source = f.read()
        self.assertIn("new URL('path-classes.json', import.meta.url)", source)
        with open(os.path.join(ORACLE, 'src', 'path-classes.json')) as f:
            self.assertEqual(json.load(f), oracle_data.PATHS)

    def test_the_weekly_reads_react_types_through_the_same_rules(self):
        import weekly
        react = '/r/node_modules/@types/react/index.d.ts'
        self.assertTrue(weekly.declared_in_react({'parent': react}))
        self.assertFalse(weekly.declared_in_react({'parent': '/r/src/Button.tsx'}))
        self.assertFalse(weekly.declared_in_react({}))


if __name__ == '__main__':
    unittest.main()
