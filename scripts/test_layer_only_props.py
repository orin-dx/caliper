#!/usr/bin/env python3
"""Tests for the layer-only wrapper, on a fake binary. Run: python3 scripts/test_layer_only_props.py"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import layer_only_props


class Command(unittest.TestCase):
    def test_the_modes_differ_only_in_the_collapse_flag(self):
        default = layer_only_props.command('bin', '/src', 'default')
        expanded = layer_only_props.command('bin', '/src', 'expanded')
        self.assertEqual(default, ['bin', 'extract', '--src', '/src', '--json', '--html-attributes', 'full',
                                   '--include-unexported', '--load-dependencies', '--quiet'])
        self.assertEqual(expanded, default + ['--collapse-inherited-props', 'none'])


class BuildConfig(unittest.TestCase):
    def test_the_report_goes_to_the_output_directory(self):
        config = layer_only_props.build_config([{'name': 'antd'}], 20, '/out')
        self.assertEqual(config, {'corpora': [{'name': 'antd'}], 'top': 20, 'out': '/out/layer-only.json'})


class RunCorpus(unittest.TestCase):
    def test_both_modes_are_run_each_time_and_the_fastest_and_largest_are_kept(self):
        calls = []
        samples = iter([(0.30, 100), (0.20, 90), (0.10, 120), (0.25, 95)])

        def fake_run_once(args, stdout=None):
            calls.append('none' if '--collapse-inherited-props' in args else 'default')
            stdout.write('{}')
            return next(samples)

        real = layer_only_props.run_once
        layer_only_props.run_once = fake_run_once
        try:
            with tempfile.TemporaryDirectory() as out:
                entry, summary = layer_only_props.run_corpus('antd', 'bin', '/src', out, 2)
                self.assertEqual(entry['default'], os.path.join(out, 'antd.default.json'))
        finally:
            layer_only_props.run_once = real
        self.assertEqual(calls, ['default', 'none', 'default', 'none'])
        self.assertEqual(summary, {'default': {'ms': 100, 'rss_mib': 120}, 'expanded': {'ms': 200, 'rss_mib': 95}})


if __name__ == '__main__':
    unittest.main()
