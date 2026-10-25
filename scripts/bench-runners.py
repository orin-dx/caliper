#!/usr/bin/env python3
"""Times caliper on the parity corpora on a CI runner, and measures what a byte-scan prefilter could save there.

For each corpus the sources are fetched by `parity.py setup --no-install`; this prints, per corpus:

- the skippable-parse probe (`crates/core/examples/skippable_probe.rs`): files, files that give resolution nothing, and
  the share of parse thread time they take;
- hyperfine timings of `extract`, best of N, with the OS page cache dropped before each run (cold) and kept (warm);
- with `--dependencies`, the same with `--load-dependencies`, cold also emptying the parse cache (those corpora need
  their dependencies installed by `parity.py setup --corpus NAME`).

  python3 scripts/bench-runners.py --bin target/release/caliper-docgen --probe target/release/examples/skippable_probe \
      [--dependencies shadcn mantine] --out bench.json --markdown bench.md
"""
import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402

RUNS = 5


def machine():
    cpu = ram = ''
    if platform.system() == 'Darwin':
        cpu = subprocess.run(['sysctl', '-n', 'machdep.cpu.brand_string'], capture_output=True, text=True).stdout.strip()
        ram = int(subprocess.run(['sysctl', '-n', 'hw.memsize'], capture_output=True, text=True).stdout) // 2**20
    else:
        for line in open('/proc/cpuinfo'):
            if line.startswith('model name'):
                cpu = line.split(':', 1)[1].strip()
                break
        ram = next(int(line.split()[1]) // 1024 for line in open('/proc/meminfo') if line.startswith('MemTotal'))
    return {'os': platform.platform(), 'cpu': cpu, 'cores': os.cpu_count(), 'ramMb': ram}


def drop_caches():
    if platform.system() == 'Darwin':
        return 'sudo purge'
    return "sync && echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null"


def hyperfine(command, prepare=None, warmup=0):
    out = tempfile.mktemp(suffix='.json')
    args = ['hyperfine', '--runs', str(RUNS), '--warmup', str(warmup), '--export-json', out, '--style', 'basic']
    if prepare:
        args += ['--prepare', prepare]
    subprocess.run([*args, command], check=True, stdout=subprocess.DEVNULL)
    result = json.load(open(out))['results'][0]
    os.remove(out)
    return {key: round(result[key] * 1000, 1) for key in ('min', 'median', 'user', 'system')}


def probe(binary, src):
    """The run with the lowest extract time of three: the first pays for the OS's cold file cache."""
    runs = [json.loads(subprocess.run([binary, '--src', src], check=True, capture_output=True, text=True).stdout)
            for _ in range(3)]
    return min(runs, key=lambda run: run['extractWallMs'])


def measure(args):
    corpora = os.path.abspath(args.corpora)
    results = {'machine': machine(), 'corpora': {}}
    for corpus in parity.CORPORA:
        src = os.path.join(corpora, corpus['clone'], corpus['src'])
        quoted = f"'{args.bin}' extract --src '{src}' --quiet"
        row = {'probe': probe(args.probe, src)}
        row['cold'] = hyperfine(quoted, prepare=drop_caches())
        row['warm'] = hyperfine(quoted, warmup=1)
        if corpus['name'] in args.dependencies:
            cache = tempfile.mkdtemp()
            deps = f"{quoted} --load-dependencies --cache-dir '{cache}'"
            row['depsCold'] = hyperfine(deps, prepare=f"{drop_caches()} && rm -rf '{cache}'/*")
            row['depsWarm'] = hyperfine(deps, warmup=1)
            shutil.rmtree(cache, ignore_errors=True)
        results['corpora'][corpus['name']] = row
        print(f"{corpus['name']}: {row}", flush=True)
    return results


def markdown(results):
    m = results['machine']
    lines = [f"### {m['os']}, {m['cpu']}, {m['cores']} cores, {m['ramMb']} MB", '',
             'Wall time is hyperfine\'s best of 5 for `extract` (min / median ms); cold drops the OS page cache first. '
             'Skippable parse is the thread time of files that give resolution nothing; "of wall" divides it by the '
             'thread count (saved if perfectly parallel) and by 1 (saved if serial).', '',
             '| corpus | files | skippable files | parse thread ms | skippable ms | share of parse | cold wall | warm wall '
             '| skippable of cold wall (÷threads / serial) |',
             '| --- | --: | --: | --: | --: | --: | --: | --: | --: |']
    for name, row in results['corpora'].items():
        p = row['probe']
        share = 100 * p['skippableParseThreadMs'] / p['parseThreadMs'] if p['parseThreadMs'] else 0
        parallel = 100 * p['skippableParseThreadMs'] / p['threads'] / row['cold']['min']
        serial = 100 * p['skippableParseThreadMs'] / row['cold']['min']
        lines.append(
            f"| {name} | {p['files']} | {p['skippableFiles']} | {p['parseThreadMs']:.0f} | "
            f"{p['skippableParseThreadMs']:.1f} | {share:.1f}% | {row['cold']['min']:.0f} / {row['cold']['median']:.0f} | "
            f"{row['warm']['min']:.0f} / {row['warm']['median']:.0f} | {parallel:.1f}% / {serial:.1f}% |")
    deps = {name: row for name, row in results['corpora'].items() if 'depsCold' in row}
    if deps:
        lines += ['', '`--load-dependencies` (cold also empties the parse cache):', '',
                  '| corpus | cold wall (min / median) | warm wall (min / median) | warm user + system |',
                  '| --- | --: | --: | --: |']
        for name, row in deps.items():
            c, w = row['depsCold'], row['depsWarm']
            lines.append(f"| {name} | {c['min']:.0f} / {c['median']:.0f} | {w['min']:.0f} / {w['median']:.0f} | "
                         f"{w['user'] + w['system']:.0f} |")
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True)
    parser.add_argument('--probe', required=True)
    parser.add_argument('--corpora', default=os.path.join(parity.ROOT, '.parity', 'corpora'))
    parser.add_argument('--dependencies', nargs='*', default=[], help='corpora to also time with --load-dependencies')
    parser.add_argument('--out', default='bench.json')
    parser.add_argument('--markdown', default='bench.md')
    args = parser.parse_args()
    results = measure(args)
    json.dump(results, open(args.out, 'w'), indent=2)
    text = markdown(results)
    open(args.markdown, 'w').write(text)
    print(text)


if __name__ == '__main__':
    main()
