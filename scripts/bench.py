#!/usr/bin/env python3
"""A/B speed and memory of this checkout (or a branch) against origin/main, for when a change touches the resolver.

Builds both in release mode, then times `extract --json` over Mantine and Chakra (source directory only, no
dependencies, fetched once into .corpora/) with the two builds interleaved and alternating which goes first, and prints
the median wall time and peak RSS of each side. Calibrated on CI runners and a laptop: with nothing changed, the ratio
stays within 1.04 for time and 1.12 for memory, so more than 1.10x in time or 1.20x in memory is worth a look. Nothing
enforces that.

Usage: scripts/bench.py [BRANCH] [--runs N] [--corpus NAME]...   (BRANCH defaults to this checkout, uncommitted work included)
"""
import argparse
import os
import shutil
import statistics
import subprocess
import tempfile

import corpora
from corpora import ROOT, run_once


def git(*args):
    return subprocess.run(['git', *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def build(manifest_dir, target_dir, destination):
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'caliper-docgen', '--manifest-path',
                    os.path.join(manifest_dir, 'Cargo.toml'), '--target-dir', target_dir], check=True)
    shutil.copy(os.path.join(target_dir, 'release', 'caliper-docgen'), destination)


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
                src = corpora.source_dir(corpus, corpora_dir)
                command = lambda binary: [binary, 'extract', '--src', src, '--json', '--no-cache', '--no-load-dependencies']
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
