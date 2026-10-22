#!/usr/bin/env python3
"""Draw a reproducible per-rule sample of detected components for precision labelling.

Runs `caliper-docgen extract --json` on every subdirectory of CORPORA, counts components by `detectedBy`, and writes
up to N per rule (seeded) with a source snippet to OUT/sample.json. Label each item `component`, `not_component` or
`unsure`, then report precision per rule as docs/precision.md does.

Usage: scripts/precision-sample.py BIN CORPORA OUT [N]
"""
import collections
import json
import os
import random
import re
import subprocess
import sys

SEED = 20261021


def snippet(path, name):
    try:
        with open(path, encoding='utf-8') as f:
            lines = f.read().splitlines()
    except OSError:
        return '', 0
    bare = re.split(r'[./]', name)[-1]
    word = re.compile(r'\b' + re.escape(bare) + r'\b')
    declaration = re.compile(r'\b(const|function|class|let|var|export)\b')
    idx = next((i for i, l in enumerate(lines) if word.search(l) and declaration.search(l)), None)
    if idx is None:
        idx = next((i for i, l in enumerate(lines) if word.search(l)), 0)
    lo, hi = max(0, idx - 3), min(len(lines), idx + 40)
    return '\n'.join(f'{i + 1:5} {lines[i]}' for i in range(lo, hi)), idx + 1


def main():
    binary, corpora, out = sys.argv[1:4]
    per_rule = int(sys.argv[4]) if len(sys.argv) > 4 else 25
    rng = random.Random(SEED)
    names = sorted(d for d in os.listdir(corpora) if os.path.isdir(os.path.join(corpora, d)))
    by_rule = collections.defaultdict(list)
    counts = collections.Counter()
    for corpus in names:
        run = subprocess.run([binary, 'extract', '--src', os.path.join(corpora, corpus), '--json'],
                             capture_output=True, text=True, check=False)
        for comp in json.loads(run.stdout)['components'].values():
            rule = comp.get('detectedBy', '?')
            counts[(corpus, rule)] += 1
            by_rule[rule].append((corpus, comp['displayName'], comp['filePath']))
    print('rule'.ljust(26) + ''.join(c[:9].rjust(10) for c in names) + 'total'.rjust(8))
    for rule in sorted(by_rule):
        row = [counts[(c, rule)] for c in names]
        print(rule.ljust(26) + ''.join(str(x).rjust(10) for x in row) + str(sum(row)).rjust(8))
    sample = []
    for rule, items in sorted(by_rule.items()):
        for corpus, name, path in items if len(items) <= per_rule else rng.sample(items, per_rule):
            text, line = snippet(path, name)
            sample.append({'id': len(sample), 'corpus': corpus, 'rule': rule, 'name': name, 'file': path,
                           'line': line, 'snippet': text})
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, 'sample.json'), 'w', encoding='utf-8') as f:
        json.dump(sample, f, indent=1)
    print(f'sampled {len(sample)} to {os.path.join(out, "sample.json")}')


if __name__ == '__main__':
    main()
