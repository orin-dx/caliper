#!/usr/bin/env python3
"""A/B speed and memory of this checkout (or a branch) against origin/main, for when a change touches the resolver.

Builds both in release mode, then times `extract --json` in the Storybook addon's configuration (dependencies loaded,
cache on after a warm-up run, inherited props not collapsed) over Mantine and Chakra with their dependencies installed
(`scripts/corpora.py setup --corpus mantine --corpus chakra`; without them nothing would be loaded, so the run stops)
with the two builds interleaved and alternating which goes first, and prints the median wall time and peak RSS of each
side. The 1.10x time and 1.20x memory thresholds come from a calibration made with dependency loading and the cache
off (within 1.04 for time and 1.12 for memory with nothing changed); it has not been repeated in this configuration, so
treat them as a starting point. Nothing enforces them.

Usage: scripts/bench.py [BRANCH] [--runs N] [--corpus NAME]...   (BRANCH defaults to this checkout, uncommitted work included)
"""
import argparse
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

import corpora
from corpora import ROOT, run_once


def git(*args):
    return subprocess.run(['git', *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def build(manifest_dir, target_dir, destination):
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'caliper-docgen', '--manifest-path',
                    os.path.join(manifest_dir, 'Cargo.toml'), '--target-dir', target_dir], check=True)
    shutil.copy(os.path.join(target_dir, 'release', 'caliper-docgen'), destination)


def extract_command(binary, src, cache_dir):
    """The addon's configuration (packages/storybook/src/provider.ts): dependencies loaded, the cache on, and
    inherited props never collapsed. Each side keeps its own cache directory so one build never reads the other's."""
    return [binary, 'extract', '--src', src, '--json', '--load-dependencies', '--collapse-inherited-props', 'none',
            '--cache-dir', cache_dir]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('branch', nargs='?')
    parser.add_argument('--runs', type=int, default=9)
    parser.add_argument('--corpus', action='append')
    args = parser.parse_args()
    args.corpus = args.corpus or ['mantine', 'chakra']
    subprocess.run(['git', 'fetch', '-q', 'origin', 'main'], cwd=ROOT)
    target = os.path.join(ROOT, 'target')
    trees = []
    with tempfile.TemporaryDirectory() as tmp:
        try:
            base_tree = os.path.join(tmp, 'base')
            git('worktree', 'add', '--detach', base_tree, git('rev-parse', '--verify', 'origin/main^{commit}'))
            trees.append(base_tree)
            build(base_tree, target, os.path.join(tmp, 'base-bin'))
            head_tree = ROOT
            if args.branch:
                head_tree = os.path.join(tmp, 'head')
                git('worktree', 'add', '--detach', head_tree, git('rev-parse', '--verify', f'{args.branch}^{{commit}}'))
                trees.append(head_tree)
            build(head_tree, target, os.path.join(tmp, 'head-bin'))
            builds = {'base': os.path.join(tmp, 'base-bin'), 'head': os.path.join(tmp, 'head-bin')}
            corpora_dir = os.path.abspath(corpora.DEFAULT_DIR)
            print(f"{'corpus':<9} {'base ms':>8} {'head ms':>8} {'ratio':>6}   {'base MiB':>8} {'head MiB':>8} {'ratio':>6}")
            for corpus in corpora.selected(args.corpus):
                src = corpora.ready(corpus, corpora_dir, sparse=False)
                if not src:
                    sys.exit(f"{corpus['name']}: dependencies not installed, so none would be loaded; "
                             f"run `scripts/corpora.py setup --corpus {corpus['name']}`")
                command = lambda binary: extract_command(binary, src, os.path.join(tmp, 'cache-' + os.path.basename(binary)))
                samples = {'base': [], 'head': []}
                for label, binary in builds.items():
                    run_once(command(binary))
                for i in range(args.runs):
                    for label in (('base', 'head') if i % 2 == 0 else ('head', 'base')):
                        samples[label].append(run_once(command(builds[label])))
                median = {label: (statistics.median(s for s, _ in runs) * 1000, statistics.median(r for _, r in runs))
                          for label, runs in samples.items()}
                (bt, br), (ht, hr) = median['base'], median['head']
                print(f"{corpus['name']:<9} {bt:8.0f} {ht:8.0f} {ht / bt:6.2f}   {br:8.0f} {hr:8.0f} {hr / br:6.2f}")
        finally:
            for tree in trees:
                subprocess.run(['git', 'worktree', 'remove', '--force', tree], cwd=ROOT, capture_output=True)


if __name__ == '__main__':
    main()
