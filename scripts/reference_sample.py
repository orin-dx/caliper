#!/usr/bin/env python3
"""The independent type reference on the pinned corpora (apps/oracle/src/reference.ts).

For each installed corpus it builds the checker's inputs, runs Caliper once, and judges a seeded, stratified control sample
of props (plus any props a --list file names) by two methods that fail differently: the type the TypeScript checker reports
(A) and the type each declaration writes (B). Each prop gets a reference from them, and Caliper's type is scored against
that reference as correct, less precise or wrong:

- Both methods agree: the reference rests on both. A and B agree on structure for some props and only on a bare alias name
  for others, so even this is not always strong evidence.
- B abstains or says less (a computed, named, mapped or inferred type): the reference rests on the checker alone. That is a
  minority of props, but a real one, and it is exactly where a checker-backed mode would agree with the checker by
  construction. The report counts scores by which method stood behind them (`correct_by_basis`, `wrong_by_basis`), so read
  a score together with its basis.
- The checker has no type and B does: the reference rests on the declaration alone, and a mismatch is never called wrong
  where the declaration only names a type.
- A and B contradict each other: the prop is contested, listed in review.md for a person and counted for no tool.

--list FILE is a JSON array of {corpus, file, component, prop} to judge besides the control sample, for example the props
two Caliper modes differ on; the report says how many listed props the checker found and names the rest. --no-tool judges
the reference only, without scoring Caliper. Output goes to --out-dir, outside tracked files by default.

Usage: scripts/reference_sample.py --bin BIN [--corpus NAME]... [--corpora DIR] [--per-corpus N] [--seed S]
                                   [--list FILE] [--out-dir DIR] [--no-tool]
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile

import corpora
import weekly
from corpora import ROOT, selected

EXTRACT = ['extract', '--json', '--html-attributes', 'full', '--include-unexported', '--load-dependencies', '--quiet']


def corpus_config(name, root, src, work, binary):
    """One corpus's entry in the reference's config: the checker's inputs, and Caliper's JSON when `binary` is given."""
    files, tsconfig = weekly.program_inputs(name, src, work)
    entry = {'name': name, 'root': root, 'tsconfig': tsconfig, 'files': files}
    if binary:
        caliper = os.path.join(work, f'{name}.caliper.json')
        with open(caliper, 'w') as f:
            subprocess.run([binary, EXTRACT[0], '--src', src, *EXTRACT[1:]], stdout=f, check=True)
        entry['caliper'] = caliper
    return entry


def build_config(entries, args, out_dir):
    """The reference's config for the corpora in `entries`."""
    config = {'corpora': entries, 'perCorpus': args.per_corpus, 'seed': args.seed,
              'out': os.path.join(out_dir, 'reference.json'), 'review': os.path.join(out_dir, 'review.md')}
    if args.list:
        config['list'] = os.path.abspath(args.list)
    return config


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', help='the caliper-docgen binary (required unless --no-tool)')
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--per-corpus', type=int, default=100)
    parser.add_argument('--seed', default='caliper')
    parser.add_argument('--list')
    parser.add_argument('--out-dir')
    parser.add_argument('--no-tool', action='store_true')
    args = parser.parse_args()
    if not args.no_tool and not args.bin:
        parser.error('--bin is required unless --no-tool')
    out_dir = os.path.abspath(args.out_dir or tempfile.mkdtemp(prefix='reference-'))
    os.makedirs(out_dir, exist_ok=True)
    entries = []
    for corpus in selected(args.corpus):
        corpora_dir = os.path.abspath(args.corpora)
        src = corpora.ready(corpus, corpora_dir, sparse=False)
        if not src:
            sys.exit(f"{corpus['name']}: not installed; run `scripts/corpora.py setup` first")
        root = os.path.join(corpora_dir, corpus['clone'])
        entries.append(corpus_config(corpus['name'], root, src, out_dir, None if args.no_tool else args.bin))
    config = build_config(entries, args, out_dir)
    config_path = os.path.join(out_dir, 'config.json')
    with open(config_path, 'w') as f:
        json.dump(config, f, indent=2)
    subprocess.run(['node', os.path.join(ROOT, 'apps', 'oracle', 'src', 'reference.ts'), '--config', config_path],
                   check=True, timeout=3600)
    print(f"report: {config['out']}\nreview: {config['review']}", file=sys.stderr)


if __name__ == '__main__':
    main()
