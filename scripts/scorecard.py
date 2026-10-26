#!/usr/bin/env python3
"""Reproducible performance scorecard on five public corpora, with the gates docs/performance.md describes.

Fetches each corpus at its pinned commit (shallow, sparse: the source directory plus the files above it, no
dependencies installed), then for each one records lines of code, components, props, unresolved-type diagnostics, and
the median wall time and peak RSS of `caliper-docgen extract --json` with a warm parse cache. Every figure is with
dependency loading off (`--no-load-dependencies`, whatever the build's default), since nothing is installed; a build
that has the flag is also timed in its default mode, as the "With dependencies loading" column.

Optional comparisons:
  --base-bin BIN    a second build (main) timed interleaved with BIN on the same machine
  --rdt             time react-docgen-typescript on the same files (scripts/scorecard-rdt.mjs)
  --rdt-baseline F  read RDT times from a committed scorecard JSON instead

--check exits 1 when a gate fails: BIN slower than 1.1x BASE on any corpus, BIN less than 10x faster than RDT on any
corpus, BIN with dependency loading more than 1.7x its time without it, or two copies of Mantine taking more than 2.2x
as long as one (the scaling test).

Usage: scripts/scorecard.py --bin BIN [--base-bin BIN] [--rdt | --rdt-baseline FILE] [--runs N] [--corpora DIR]
                            [--out FILE] [--check]
"""
import argparse
import functools
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

CORPORA = [
    {
        'name': 'mantine',
        'repo': 'mantinedev/mantine',
        'commit': '194d652452ee060aad3796c121f9b6a9f74cb453',
        'src': 'packages/@mantine/core/src',
    },
    {
        'name': 'chakra',
        'repo': 'chakra-ui/chakra-ui',
        'commit': '960ae222ace1df8de835dc3c62d698f1b58b6786',
        'src': 'packages/react/src',
    },
    {
        'name': 'shadcn',
        'repo': 'shadcn-ui/ui',
        'commit': '3d98900e9b73c45f035f8109e9d41ba6a51685e7',
        'src': 'apps/v4',
    },
    {
        'name': 'antd',
        'repo': 'ant-design/ant-design',
        'commit': 'f8f5595ab5e891fff6a2a06c5b3b98af643d008e',
        'src': 'components',
    },
    {
        'name': 'spectrum',
        'repo': 'adobe/react-spectrum',
        'commit': 'd5fb723a1c192e3a1a25c8cfdd4e26b900995307',
        'src': 'packages',
    },
]

# Diagnostics that mean a type was left unresolved, counted when they are warnings or errors.
UNRESOLVED_CODES = {
    'UNRESOLVABLE_IMPORT',
    'OPAQUE_TYPE',
    'INDEXED_ACCESS_OPAQUE',
    'TEMPLATE_LITERAL_OPAQUE',
    'GENERIC_ARGUMENT_MISMATCH',
    'UNKNOWN',
}

# Extraction ms = FIXED_MS + PER_KLOC_MS * kLOC + PER_KPROP_MS * kprops: a sizing guide, reported, not a gate
# (docs/performance.md).
FIXED_MS, PER_KLOC_MS, PER_KPROP_MS = 6.4, 0.13, 3.8
REGRESSION_LIMIT = 1.1
# Dependency loading with nothing installed: every import fails after a directory walk and the index keeps only answers
# found inside a package, so each run repeats them. Measured 1.15x to 1.2x on a laptop and up to about 1.5x on a CI
# runner; the limit catches a real jump, not that cost.
DEPS_LIMIT = 1.7
RDT_MIN_SPEEDUP = 10.0
SCALING_CORPUS = 'mantine'
SCALING_LIMIT = 2.2

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def git(*args, cwd):
    subprocess.run(['git', *args], cwd=cwd, check=True, stdout=subprocess.DEVNULL)


def fetch(corpus, corpora_dir):
    """The corpus' source directory, fetched once per pinned commit and source directory."""
    checkout = os.path.join(corpora_dir, f"{corpus['name']}-{corpus['commit'][:12]}")
    src = os.path.join(checkout, corpus['src'])
    marker = os.path.join(checkout, '.scorecard-complete')
    if os.path.exists(marker):
        with open(marker) as f:
            if f.read() == corpus['src']:
                return src
    shutil.rmtree(checkout, ignore_errors=True)
    os.makedirs(checkout)
    git('init', '-q', cwd=checkout)
    git('remote', 'add', 'origin', f"https://github.com/{corpus['repo']}.git", cwd=checkout)
    git('sparse-checkout', 'set', corpus['src'], cwd=checkout)
    # Network steps retry: one dropped connection shouldn't fail a scheduled run.
    for attempt in range(3):
        try:
            git('fetch', '-q', '--depth', '1', '--filter=blob:none', 'origin', corpus['commit'], cwd=checkout)
            git('checkout', '-q', 'FETCH_HEAD', cwd=checkout)
            break
        except subprocess.CalledProcessError:
            if attempt == 2:
                raise
            time.sleep(5)
    with open(marker, 'w') as f:
        f.write(corpus['src'])
    return src


def excluded(relative):
    """The same exclusions as crates/core/src/pipeline/discover.rs."""
    name = os.path.basename(relative)
    parts = relative.split(os.sep)
    return (
        '.stories.' in name
        or '.test.' in name
        or '.spec.' in name
        or '__snapshots__' in parts
        or 'node_modules' in parts
    )


def lines_of_code(src):
    total = 0
    for dirpath, _, names in os.walk(src):
        for name in names:
            path = os.path.join(dirpath, name)
            if name.endswith(('.ts', '.tsx')) and not excluded(os.path.relpath(path, src)):
                with open(path, 'rb') as f:
                    total += f.read().count(b'\n')
    return total


def measure(cmd, stdout=subprocess.DEVNULL):
    """Wall time in ms and peak RSS in MB of one run of `cmd`."""
    start = time.perf_counter()
    proc = subprocess.Popen(cmd, stdout=stdout, stderr=subprocess.DEVNULL)
    _, status, usage = os.wait4(proc.pid, 0)
    elapsed = (time.perf_counter() - start) * 1000
    proc.returncode = os.waitstatus_to_exitcode(status)
    if proc.returncode not in (0, 1):
        sys.exit(f'{" ".join(cmd)} exited {proc.returncode}')
    # ru_maxrss is bytes on macOS, kilobytes on Linux.
    rss = usage.ru_maxrss / (1024 * 1024 if sys.platform == 'darwin' else 1024)
    return elapsed, rss


@functools.lru_cache(maxsize=None)
def has_deps_flag(binary):
    """Whether `binary` knows `--no-load-dependencies` (builds from before dependency loading became the default don't)."""
    help_text = subprocess.run([binary, 'extract', '--help'], capture_output=True, text=True).stdout
    return '--no-load-dependencies' in help_text


def extract_command(binary, src, cache, deps=False):
    """`extract --json` over `src`. The corpora are fetched without their dependencies, so the scorecard measures with
    dependency loading off, whatever the binary's default; a build from before the flag existed is already off. The HTML
    attributes are `none`, so the gate times the resolver on the work main did; the cost of listing them is in ADR 0024."""
    command = [binary, 'extract', '--json', '-s', src, '--cache-dir', cache, '--html-attributes', 'none']
    if not deps and has_deps_flag(binary):
        command.append('--no-load-dependencies')
    return command


def counts(binary, src, cache):
    with tempfile.TemporaryFile() as out:
        measure(extract_command(binary, src, cache), stdout=out)
        out.seek(0)
        output = json.load(out)
    components = output['components'].values()
    return {
        'components': len(components),
        'props': sum(len(component['props']) for component in components),
        'unresolved': sum(
            1
            for d in output['diagnostics']
            if d['code'] in UNRESOLVED_CODES and d['severity'] != 'info'
        ),
    }


def timed_run(binary, src, cache, deps=False):
    """Wall ms, peak RSS MB, and the extraction's own `durationMs` of one run."""
    with tempfile.TemporaryFile() as out:
        wall, rss = measure(extract_command(binary, src, cache, deps), stdout=out)
        out.seek(0)
        return wall, rss, json.load(out)['stats']['durationMs']


def time_binaries(binaries, src, caches, runs, deps=()):
    """Medians and max peak RSS per binary, runs interleaved so machine drift hits each one alike. Each name in `deps`
    is timed again as `<name>-deps`, with dependency loading on."""
    variants = {name: (binary, False) for name, binary in binaries.items()}
    variants.update({f'{name}-deps': (binaries[name], True) for name in deps})
    for name in deps:
        caches[f'{name}-deps'] = caches[name] + '-deps'
    samples = {name: [] for name in variants}
    for _ in range(runs):
        for name, (binary, with_deps) in variants.items():
            samples[name].append(timed_run(binary, src, caches[name], with_deps))
    return {
        name: {
            'ms': round(statistics.median(wall for wall, _, _ in runs_), 1),
            'extract_ms': statistics.median(duration for _, _, duration in runs_),
            'rss_mb': round(max(rss for _, rss, _ in runs_), 1),
        }
        for name, runs_ in samples.items()
    }


def scaling(binary, src, runs, work_dir):
    """Extraction ms for one copy of `src` and for two, interleaved. Two copies double the files, components and props
    (every name collides once), so linear scaling takes at most twice as long."""
    one, two = os.path.join(work_dir, 'one'), os.path.join(work_dir, 'two')
    shutil.copytree(src, os.path.join(one, 'a'), symlinks=True)
    shutil.copytree(src, os.path.join(two, 'a'), symlinks=True)
    shutil.copytree(src, os.path.join(two, 'b'), symlinks=True)
    caches = {'one': os.path.join(work_dir, 'cache-one'), 'two': os.path.join(work_dir, 'cache-two')}
    for name, tree in (('one', one), ('two', two)):
        timed_run(binary, tree, caches[name])
    samples = {'one': [], 'two': []}
    for _ in range(runs):
        for name, tree in (('one', one), ('two', two)):
            samples[name].append(timed_run(binary, tree, caches[name])[2])
    return {name: statistics.median(durations) for name, durations in samples.items()}


def time_rdt(src, runs):
    script = os.path.join(ROOT, 'scripts', 'scorecard-rdt.mjs')
    results = [json.loads(subprocess.run(['node', script, src], check=True, capture_output=True).stdout) for _ in range(runs)]
    return {**results[0], 'ms': statistics.median(r['ms'] for r in results)}


def predicted_ms(row):
    return FIXED_MS + PER_KLOC_MS * row['loc'] / 1000 + PER_KPROP_MS * row['props'] / 1000


def gates(rows, scaled):
    """Every gate failure, as a message."""
    failures = []
    if scaled and scaled['two'] > SCALING_LIMIT * scaled['one']:
        failures.append(
            f"scaling: two copies of {SCALING_CORPUS} take {scaled['two']} ms, more than {SCALING_LIMIT}x one copy's "
            f"{scaled['one']} ms"
        )
    for row in rows:
        if 'base' in row and row['ms'] > REGRESSION_LIMIT * row['base']['ms']:
            failures.append(
                f"{row['name']}: {row['ms']} ms is more than {REGRESSION_LIMIT}x main's {row['base']['ms']} ms"
            )
        if 'deps' in row and row['deps']['ms'] > DEPS_LIMIT * row['ms']:
            failures.append(
                f"{row['name']}: dependency loading takes {row['deps']['ms']} ms, more than {DEPS_LIMIT}x the "
                f"{row['ms']} ms without it"
            )
        if 'rdt' in row and row['rdt']['ms'] < RDT_MIN_SPEEDUP * row['ms']:
            failures.append(
                f"{row['name']}: {row['rdt']['ms'] / row['ms']:.1f}x faster than RDT, below {RDT_MIN_SPEEDUP:.0f}x"
            )
    return failures


def table(rows):
    header = (
        '| Corpus | kLOC | Components | Props | Unresolved | Wall ms | Peak RSS MB | Extract ms | Model ms | vs main '
        '| With dependencies loading | vs RDT |'
    )
    lines = [header, '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in rows:
        base = f"{row['ms'] / row['base']['ms']:.2f}x" if 'base' in row else ''
        deps = f"{row['deps']['ms']} ms ({row['deps']['ms'] / row['ms']:.2f}x)" if 'deps' in row else ''
        rdt = f"{row['rdt']['ms'] / row['ms']:.0f}x faster" if 'rdt' in row else ''
        lines.append(
            f"| {row['name']} | {row['loc'] / 1000:.1f} | {row['components']:,} | {row['props']:,} | "
            f"{row['unresolved']:,} | {row['ms']} | {row['rss_mb']} | {row['extract_ms']} | {predicted_ms(row):.0f} | "
            f'{base} | {deps} | {rdt} |'
        )
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True, help='the caliper-docgen binary to score')
    parser.add_argument('--base-bin', help="main's caliper-docgen binary, for the regression gate")
    rdt = parser.add_mutually_exclusive_group()
    rdt.add_argument('--rdt', action='store_true', help='time react-docgen-typescript on each corpus')
    rdt.add_argument('--rdt-baseline', help='read RDT times from this scorecard JSON')
    parser.add_argument('--runs', type=int, default=10)
    parser.add_argument('--rdt-runs', type=int, default=1)
    parser.add_argument('--corpora', default=os.path.join(ROOT, '.scorecard', 'corpora'))
    parser.add_argument('--out', help='write the scorecard as JSON here')
    parser.add_argument('--check', action='store_true', help='exit 1 when a gate fails')
    parser.add_argument('--corpus', action='append', metavar='NAME',
                        help='score only this corpus (repeatable); the scaling gate runs only with mantine')
    args = parser.parse_args()

    binaries = {'head': os.path.abspath(args.bin)}
    if args.base_bin:
        binaries['base'] = os.path.abspath(args.base_bin)
    rdt_baseline = {}
    if args.rdt_baseline:
        with open(args.rdt_baseline) as f:
            rdt_baseline = {row['name']: row['rdt'] for row in json.load(f)['corpora'] if 'rdt' in row}

    rows = []
    scaled = None
    with tempfile.TemporaryDirectory() as caches_dir:
        for corpus in [c for c in CORPORA if not args.corpus or c['name'] in args.corpus]:
            src = fetch(corpus, args.corpora)
            caches = {name: os.path.join(caches_dir, corpus['name'], name) for name in binaries}
            row = {
                'name': corpus['name'],
                'repo': corpus['repo'],
                'commit': corpus['commit'],
                'src': corpus['src'],
                'loc': lines_of_code(src),
                # The first run fills the parse cache and is the counts run.
                **counts(binaries['head'], src, caches['head']),
            }
            if 'base' in binaries:
                row['base_counts'] = counts(binaries['base'], src, caches['base'])
            deps = ('head',) if has_deps_flag(binaries['head']) else ()
            timed = time_binaries(binaries, src, caches, args.runs, deps)
            row.update(timed.pop('head'))
            if 'head-deps' in timed:
                row['deps'] = timed['head-deps']
            if 'base' in timed:
                row['base'] = timed['base']
            if args.rdt:
                row['rdt'] = time_rdt(src, args.rdt_runs)
            elif corpus['name'] in rdt_baseline:
                row['rdt'] = rdt_baseline[corpus['name']]
            rows.append(row)
            print(f"{corpus['name']}: {row['ms']} ms", file=sys.stderr)
            if corpus['name'] == SCALING_CORPUS:
                scaled = scaling(binaries['head'], src, args.runs, os.path.join(caches_dir, 'scaling'))

    report = table(rows)
    if scaled:
        report += f"\n\nScaling: {SCALING_CORPUS} extracts in {scaled['one']} ms, two copies in {scaled['two']} ms "
        report += f"({scaled['two'] / scaled['one']:.2f}x, limit {SCALING_LIMIT}x)."
    failures = gates(rows, scaled)
    summary = report + '\n\n' + ('\n'.join(f'- FAIL {f}' for f in failures) if failures else 'All gates pass.') + '\n'
    print(summary)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as f:
            f.write(summary)
    if args.out:
        scorecard = {
            'platform': sys.platform,
            'runs': args.runs,
            'model': {'fixed_ms': FIXED_MS, 'per_kloc_ms': PER_KLOC_MS, 'per_kprop_ms': PER_KPROP_MS},
            'scaling': {'corpus': SCALING_CORPUS, 'one_ms': scaled['one'], 'two_ms': scaled['two']} if scaled else None,
            'corpora': rows,
        }
        with open(args.out, 'w') as f:
            json.dump(scorecard, f, indent=2)
            f.write('\n')
    if args.check and failures:
        sys.exit(1)


if __name__ == '__main__':
    main()
