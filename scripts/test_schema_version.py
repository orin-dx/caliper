"""Tests that the scripts refuse caliper JSON of a schema version they do not read."""
import json
import os
import re
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import agent_eval
import schema_version
import weekly

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def completed(stdout):
    return subprocess.CompletedProcess([], 0, stdout=stdout, stderr='')


class SupportedVersion(unittest.TestCase):
    def test_matches_the_version_rust_writes(self):
        with open(os.path.join(ROOT, 'crates', 'core', 'src', 'types', 'output.rs')) as f:
            rust = int(re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);', f.read()).group(1))
        self.assertEqual(schema_version.SUPPORTED_SCHEMA_VERSION, rust)

    def test_a_different_version_stops_with_a_message_naming_both(self):
        with self.assertRaises(SystemExit) as raised:
            schema_version.check({'schemaVersion': 99}, 'agent_eval')
        self.assertEqual(str(raised.exception),
                         f'agent_eval: the caliper output has schema version 99, and this script reads version '
                         f'{schema_version.SUPPORTED_SCHEMA_VERSION}. Run it with a caliper-docgen binary of the same release.')

    def test_version_6_without_exports_is_not_read_and_version_7_is(self):
        self.assertEqual(schema_version.SUPPORTED_SCHEMA_VERSION, 7)
        with self.assertRaises(SystemExit) as raised:
            schema_version.check({'schemaVersion': 6, 'components': {}}, 'weekly')
        self.assertIn('has schema version 6, and this script reads version 7.', str(raised.exception))

    def test_output_without_a_version_is_version_1(self):
        with self.assertRaises(SystemExit) as raised:
            schema_version.check({'components': {}}, 'weekly')
        self.assertIn('has schema version 1,', str(raised.exception))

    def test_the_supported_version_passes_through(self):
        document = {'schemaVersion': schema_version.SUPPORTED_SCHEMA_VERSION, 'components': {}}
        self.assertIs(schema_version.check(document, 'weekly'), document)


class Readers(unittest.TestCase):
    wrong = json.dumps({'schemaVersion': 99, 'components': {}, 'diagnostics': []})

    def test_agent_eval_refuses_a_mismatching_document(self):
        with mock.patch('agent_eval.subprocess.run', return_value=completed('')) as run:
            run.side_effect = [completed(''), completed(self.wrong)]
            with mock.patch('agent_eval.parse_toon', return_value={}):
                with self.assertRaises(SystemExit) as raised:
                    agent_eval.caliper_views('caliper', '/src', '/src')
        self.assertIn('has schema version 99,', str(raised.exception))

    def test_weekly_refuses_a_mismatching_document(self):
        with mock.patch('weekly.subprocess.run', return_value=completed(self.wrong)):
            with self.assertRaises(SystemExit) as raised:
                weekly.caliper_layer_members('caliper', '/src', '/src')
        self.assertIn('has schema version 99,', str(raised.exception))


if __name__ == '__main__':
    unittest.main()
