#!/usr/bin/env python3
"""Tests that bench.py measures the configuration the Storybook addon runs. Run: python3 scripts/test_bench.py"""
import os
import re
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench
from corpora import ROOT


class BenchConfiguration(unittest.TestCase):
    def command(self):
        return bench.extract_command('/bin/caliper-docgen', '/src', '/cache')

    def test_no_flag_turns_off_what_the_addon_leaves_on(self):
        command = self.command()
        self.assertNotIn('--no-cache', command)
        self.assertNotIn('--no-load-dependencies', command)
        self.assertIn('--load-dependencies', command)

    def test_collapse_and_dependency_defaults_are_the_addons(self):
        provider = open(os.path.join(ROOT, 'packages/storybook/src/provider.ts')).read()
        collapse = re.search(r"collapseInheritedProps: options\.collapseInheritedProps \?\? '(\w+)'", provider)
        load = re.search(r'loadDependencies: options\.loadDependencies \?\? (\w+)', provider)
        self.assertEqual((collapse.group(1), load.group(1)), ('none', 'true'))
        command = self.command()
        self.assertEqual(command[command.index('--collapse-inherited-props') + 1], collapse.group(1))


if __name__ == '__main__':
    unittest.main()
