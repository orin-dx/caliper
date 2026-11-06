#!/usr/bin/env python3
"""Tests for the reference sample's wrapper, on a fake binary. Run: python3 scripts/test_reference_sample.py"""
import argparse
import json
import os
import stat
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import reference_sample


def source_dir(tmp):
    src = os.path.join(tmp, 'src')
    os.makedirs(os.path.join(src, 'button'))
    for name in ('button/Button.tsx', 'button/util.ts'):
        with open(os.path.join(src, name), 'w') as f:
            f.write('')
    return src


def fake_binary(tmp):
    """A binary that records its arguments and prints an empty JSON document."""
    path = os.path.join(tmp, 'fake-caliper')
    with open(path, 'w') as f:
        f.write('#!/bin/sh\necho "$@" > "$(dirname "$0")/args"\necho "{}"\n')
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


class CorpusConfig(unittest.TestCase):
    def test_without_a_binary_it_prepares_only_the_checkers_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            entry = reference_sample.corpus_config('antd', '/r', source_dir(tmp), tmp, None)
            self.assertEqual(sorted(entry), ['files', 'name', 'root', 'tsconfig'])
            self.assertEqual(entry['name'], 'antd')
            self.assertEqual(entry['root'], '/r')
            with open(entry['files']) as f:
                self.assertEqual(f.read().split(), [os.path.join(tmp, 'src', 'button', 'Button.tsx')])

    def test_with_a_binary_it_runs_extract_with_the_comparison_flags_and_keeps_its_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = fake_binary(tmp)
            entry = reference_sample.corpus_config('antd', '/r', source_dir(tmp), tmp, binary)
            self.assertEqual(entry['caliper'], os.path.join(tmp, 'antd.caliper.json'))
            with open(entry['caliper']) as f:
                self.assertEqual(json.load(f), {})
            with open(os.path.join(tmp, 'args')) as f:
                self.assertEqual(f.read().split(), [
                    'extract', '--src', os.path.join(tmp, 'src'), '--json', '--html-attributes', 'full',
                    '--include-unexported', '--load-dependencies', '--quiet'])


class BuildConfig(unittest.TestCase):
    def args(self, **over):
        values = {'per_corpus': 100, 'seed': 'caliper', 'list': None}
        values.update(over)
        return argparse.Namespace(**values)

    def test_the_outputs_go_to_the_output_directory(self):
        config = reference_sample.build_config([{'name': 'antd'}], self.args(), '/out')
        self.assertEqual(config, {'corpora': [{'name': 'antd'}], 'perCorpus': 100, 'seed': 'caliper',
                                  'out': '/out/reference.json', 'review': '/out/review.md'})

    def test_a_list_file_is_passed_by_absolute_path(self):
        config = reference_sample.build_config([], self.args(list='diff.json', per_corpus=300, seed='s'), '/out')
        self.assertEqual(config['list'], os.path.abspath('diff.json'))
        self.assertEqual((config['perCorpus'], config['seed']), (300, 's'))


if __name__ == '__main__':
    unittest.main()
