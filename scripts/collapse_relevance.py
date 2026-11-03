#!/usr/bin/env python3
"""Before and after the collapse rule (trail 192): what the default output and the Storybook addon's configuration cost.

Two builds of the CLI (and of `lazy_bench`, the session benchmark) are compared on the pinned corpora, in two modes:

  default    `--collapse-inherited-props dependencies`, what the CLI and the NAPI binding use unless told otherwise
  storybook  `--collapse-inherited-props none`, what the Storybook addon runs with (packages/storybook/src/provider.ts)

Per corpus, mode and build it reports the canonical JSON's bytes, the number of collapsed layers, and the median whole-run
wall time and peak RSS of N interleaved runs with their spread, the machine's load average, and whether the `none`
output is identical between builds (the rule must not touch it). With --lazy it adds a lazy session's start, time to the
first component and to the first 50, and its peak RSS. With --oracle it adds the checker oracle's verdict counts for each
build's default output (apps/oracle/src/oracle.ts), for a comparison of type correctness.

Usage: scripts/collapse_relevance.py --old BIN --new BIN [--old-bench BIN --new-bench BIN] [--corpora DIR]
           [--src NAME=DIR]... [--corpus NAME]... [--runs N] [--lazy] [--oracle] [--work DIR] [--out FILE]
"""
import argparse
import hashlib
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import corpora  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXTRACT = ['extract', '--json', '--html-attributes', 'full', '--include-unexported', '--load-dependencies', '--quiet',
           '--no-cache']
MODES = {'default': [], 'storybook': ['--collapse-inherited-props', 'none']}
LAZY_COUNTS = (1, 50)
RSS = re.compile(r'(\d+)\s+maximum resident set size')
REAL = re.compile(r'^\s*([\d.]+)\s+real', re.MULTILINE)


def timed(command, stdout):
    """Run `command` under /usr/bin/time -l: its wall seconds and peak RSS in MiB."""
    run = subprocess.run(['/usr/bin/time', '-l', *command], stdout=stdout, stderr=subprocess.PIPE, text=True, check=True)
    return float(REAL.search(run.stderr).group(1)), int(RSS.search(run.stderr).group(1)) / 2**20


def spread(values):
    """Median and (min, max) of `values`."""
    return statistics.median(values), (min(values), max(values))


def canonical_digest(path):
    """A digest of the output with the run-dependent timing fields removed, so two builds compare by content."""
    with open(path) as f:
        data = json.load(f)
    data.pop('stats', None)
    for key in ('durationMs', 'generatedAt'):
        data.pop(key, None)
    return hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()[:16]


def collapsed_layers(path):
    """How many collapsed (non-HTML) inheritance layers the output holds."""
    with open(path) as f:
        data = json.load(f)
    return sum(1 for c in data['components'].values() for layer in c.get('inheritance', []) if not layer.get('htmlElement'))


def run_extract(builds, src, work, name, runs):
    """Interleave `runs` rounds over (mode, build), alternating which build goes first so neither one is always warm."""
    cells = {(m, b): {'seconds': [], 'rss': []} for m in MODES for b in builds}
    output = {}
    for round_ in range(runs):
        order = list(builds) if round_ % 2 == 0 else list(reversed(list(builds)))
        for mode, flags in MODES.items():
            for build in order:
                path = os.path.join(work, f'{name}.{mode}.{build}.json')
                with open(path, 'w') as f:
                    seconds, rss = timed([builds[build], *EXTRACT, '--src', src, *flags], f)
                cells[(mode, build)]['seconds'].append(seconds)
                cells[(mode, build)]['rss'].append(rss)
                output[(mode, build)] = path
    result = {}
    for (mode, build), cell in cells.items():
        path = output[(mode, build)]
        result.setdefault(mode, {})[build] = {
            'bytes': os.path.getsize(path), 'layers': collapsed_layers(path), 'digest': canonical_digest(path),
            'seconds': spread(cell['seconds']), 'rss_mib': spread(cell['rss']),
        }
    return result


def run_lazy(benches, src, work, name, runs):
    """A lazy session's start, time to the first component and to the first 50, and peak RSS, per mode and build."""
    keys = os.path.join(work, f'{name}.keys')
    base = ['--src', src, '--cache', os.path.join(work, f'{name}.cache'), '--dependencies', '--full']
    first = next(iter(benches.values()))
    timed([first, *base, '--count', '0', '--write-keys', keys], subprocess.DEVNULL)
    result = {}
    for mode, flags in MODES.items():
        collapse = ['--collapse', flags[1]] if flags else []
        for build, binary in benches.items():
            cell = {count: {'ms': [], 'rss': []} for count in LAZY_COUNTS}
            for _ in range(runs):
                for count in LAZY_COUNTS:
                    with tempfile.TemporaryFile('w+') as out:
                        _, rss = timed([binary, *base, *collapse, '--keys', keys, '--count', str(count)], out)
                        out.seek(0)
                        line = json.loads(out.read().strip().splitlines()[-1])
                    cell[count]['ms'].append(line['startMs'] + line['answerMs'])
                    cell[count]['rss'].append(rss)
            result.setdefault(mode, {})[build] = {
                count: {'ms': spread(c['ms']), 'rss_mib': spread(c['rss'])} for count, c in cell.items()
            }
    return result


def run_oracle(binaries, name, src, root, work):
    """The checker oracle's verdict counts for each build's default output (the Storybook mode is identical)."""
    import weekly  # noqa: E402  (imports the oracle's program inputs)
    list_file, tsconfig = weekly.program_inputs(name, src, work)
    result = {}
    for build in binaries:
        caliper = os.path.join(work, f'{name}.default.{build}.json')
        verdicts = os.path.join(work, f'{name}.oracle.{build}.json')
        subprocess.run(['pnpm', '--dir', os.path.join(ROOT, 'apps', 'oracle'), 'exec', 'tsx', 'src/oracle.ts', '--root', root,
                        '--src', src, '--tsconfig', tsconfig, '--caliper', caliper, '--out', verdicts, '--files', list_file],
                       check=True, timeout=3600)
        with open(verdicts) as f:
            report = json.load(f)
        result[build] = {'shared': report['props_shared'], **report['verdicts']}
    return result


def fmt(pair):
    median, (low, high) = pair
    return f'{median:.3g} ({low:.3g} to {high:.3g})'


def report(name, extract, lazy, oracle):
    print(f'\n== {name}')
    for mode, builds in extract.items():
        old, new = builds['old'], builds['new']
        change = 100 * (new['bytes'] - old['bytes']) / old['bytes']
        print(f"  {mode}: bytes {old['bytes']:,} -> {new['bytes']:,} ({change:+.1f}%), layers {old['layers']} -> {new['layers']}, "
              f"identical output {old['digest'] == new['digest']}")
        for build in ('old', 'new'):
            print(f"    {build}: {fmt(builds[build]['seconds'])} s, {fmt(builds[build]['rss_mib'])} MiB")
    for mode, builds in (lazy or {}).items():
        for build, counts in builds.items():
            cells = ', '.join(f"{count} components {fmt(c['ms'])} ms {fmt(c['rss_mib'])} MiB" for count, c in counts.items())
            print(f'  lazy {mode} {build}: {cells}')
    for build, verdicts in (oracle or {}).items():
        print(f'  oracle {build}: ' + ', '.join(f'{k} {v:,}' for k, v in verdicts.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--old', required=True)
    parser.add_argument('--new', required=True)
    parser.add_argument('--old-bench')
    parser.add_argument('--new-bench')
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--src', action='append', default=[], metavar='NAME=DIR')
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--lazy', action='store_true')
    parser.add_argument('--oracle', action='store_true')
    parser.add_argument('--work', default=None)
    parser.add_argument('--out')
    args = parser.parse_args()
    if args.lazy and not (args.old_bench and args.new_bench):
        parser.error('--lazy needs --old-bench and --new-bench (the lazy_bench example of each build)')
    overrides = dict(item.split('=', 1) for item in args.src)
    work = args.work or tempfile.mkdtemp(prefix='collapse-relevance-')
    os.makedirs(work, exist_ok=True)
    print(f'load average {os.getloadavg()[0]:.1f}, {args.runs} runs, work {work}', file=sys.stderr)
    results = {}
    for corpus in corpora.CORPORA:
        name = corpus['name']
        if args.corpus and name not in args.corpus:
            continue
        src = overrides.get(name) or corpora.ready(corpus, os.path.abspath(args.corpora), sparse=False)
        if not src:
            sys.exit(f'{name}: not installed; run `scripts/corpora.py setup` or pass --src {name}=DIR')
        # The repository root is the source directory without the corpus's own source path.
        root = src[: -len(corpus['src'])].rstrip('/') if src.endswith(corpus['src']) else os.path.dirname(src)
        builds = {'old': args.old, 'new': args.new}
        extract = run_extract(builds, src, work, name, args.runs)
        lazy = run_lazy({'old': args.old_bench, 'new': args.new_bench}, src, work, name, args.runs) if args.lazy else None
        oracle = run_oracle(builds, name, src, root, work) if args.oracle else None
        results[name] = {'extract': extract, 'lazy': lazy, 'oracle': oracle}
        report(name, extract, lazy, oracle)
    if args.out:
        with open(args.out, 'w') as f:
            json.dump(results, f, indent=1)


if __name__ == '__main__':
    main()
