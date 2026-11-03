#!/usr/bin/env python3
"""The props that reach a reader only as a name through a collapsed layer (apps/oracle/src/layer-only.ts).

For each installed corpus it runs Caliper twice, at the default collapse and with --collapse-inherited-props none, then
sorts every prop a collapsed layer names and the component does not list into AMBIENT (what every element has, or a style
system) and API (what a dependency's component exposes), by a rule that reads only the layer's type, its file and the
prop's name (docs/measurement.md, "Layer-only props"). It reports how many of each, the API props that matter most, how
the layers fall either side of the 30-prop collapse threshold, and what listing the API props would add to the output.
This describes the default collapse of the CLI, the NAPI binding and the Vite plugin (`dependencies`). The Storybook addon
defaults to `none`, which collapses nothing, so it has no layer-only props; the expanded run is that configuration.
Measurement only: it does not change the resolver.

--runs N times each mode N times, interleaved, and reports the fastest wall time and the largest peak RSS. Output goes to
--out-dir, outside tracked files by default.

Usage: scripts/layer_only_props.py --bin BIN [--corpus NAME]... [--corpora DIR] [--runs N] [--top N] [--out-dir DIR]
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile

import corpora
from corpora import ROOT, run_once, selected

EXTRACT = ['extract', '--json', '--html-attributes', 'full', '--include-unexported', '--load-dependencies', '--quiet']
MODES = {'default': [], 'expanded': ['--collapse-inherited-props', 'none']}


def command(binary, src, mode):
    """The extract command for one collapse mode."""
    return [binary, EXTRACT[0], '--src', src, *EXTRACT[1:], *MODES[mode]]


def run_corpus(name, binary, src, out_dir, runs):
    """Both modes' JSON on disk, and each mode's fastest wall ms and largest peak RSS MiB over `runs` interleaved runs."""
    paths = {mode: os.path.join(out_dir, f'{name}.{mode}.json') for mode in MODES}
    timings = {mode: {'ms': [], 'rss_mib': []} for mode in MODES}
    for _ in range(runs):
        for mode in MODES:
            with open(paths[mode], 'w') as f:
                seconds, rss = run_once(command(binary, src, mode), stdout=f)
            timings[mode]['ms'].append(round(seconds * 1000))
            timings[mode]['rss_mib'].append(round(rss))
    summary = {mode: {'ms': min(t['ms']), 'rss_mib': max(t['rss_mib'])} for mode, t in timings.items()}
    return {'name': name, 'default': paths['default'], 'expanded': paths['expanded']}, summary


def build_config(entries, top, out_dir):
    """The report's config for the corpora in `entries`."""
    return {'corpora': entries, 'top': top, 'out': os.path.join(out_dir, 'layer-only.json')}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True)
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--top', type=int, default=20)
    parser.add_argument('--out-dir')
    args = parser.parse_args()
    out_dir = os.path.abspath(args.out_dir or tempfile.mkdtemp(prefix='layer-only-'))
    os.makedirs(out_dir, exist_ok=True)
    entries, timings = [], {}
    for corpus in selected(args.corpus):
        corpora_dir = os.path.abspath(args.corpora)
        src = corpora.ready(corpus, corpora_dir, sparse=False)
        if not src:
            sys.exit(f"{corpus['name']}: not installed; run `scripts/corpora.py setup` first")
        entry, timing = run_corpus(corpus['name'], args.bin, src, out_dir, args.runs)
        entries.append(entry)
        timings[corpus['name']] = timing
    config = build_config(entries, args.top, out_dir)
    config_path = os.path.join(out_dir, 'config.json')
    with open(config_path, 'w') as f:
        json.dump(config, f, indent=2)
    subprocess.run(['node', os.path.join(ROOT, 'apps', 'oracle', 'src', 'layer-only-report.ts'), '--config', config_path],
                   check=True, timeout=3600)
    with open(os.path.join(out_dir, 'timings.json'), 'w') as f:
        json.dump(timings, f, indent=2)
    print(f"report: {config['out']}\ntimings: {json.dumps(timings)}", file=sys.stderr)


if __name__ == '__main__':
    main()
