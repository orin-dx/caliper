#!/usr/bin/env python3
"""Caliper against react-docgen-typescript (RDT) on five public repos with their dependencies installed.

`setup` fetches each repo at its pinned commit, installs its dependencies with its own package manager and lockfile
(lifecycle scripts off, caches under the corpora directory), and prunes tests, stories and demos from the source
directory. `run` dumps RDT once per corpus (scripts/parity-rdt.mjs, cached by commit), runs Caliper in RDT's format
with and without --load-dependencies, and reports per corpus:

  - components RDT reports (with props) that Caliper also reports, by file and name;
  - RDT props not declared by @types/react that Caliper reports;
  - shared props whose type text is identical, identical after cosmetic normalization, or gives the same Storybook
    control;
  - shared props per loss pattern: Caliper's type carrying less than RDT's (a member read left as `X["k"]`, a lost
    rest parameter, ...), and two patterns where RDT carries less;
  - best-of-N wall time and peak RSS of `extract --json`, without and with --load-dependencies, next to RDT's.

--check against --baseline fails when coverage, agreement or same-control rates drop, a loss pattern grows, or BIN is
more than 1.1x slower than --base-bin (timed interleaved on the same machine). docs/parity.md explains the measures.

Usage: scripts/parity.py setup [--corpora DIR] [--corpus NAME]...
       scripts/parity.py run --bin BIN [--base-bin BIN] [--corpora DIR] [--rdt-dir DIR] [--corpus NAME]...
                             [--runs N] [--no-timing] [--out FILE] [--markdown FILE] [--baseline FILE] [--check]
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

CORPORA = [
    {
        'name': 'mantine',
        'repo': 'mantinedev/mantine',
        'commit': 'e47e45af06427b3b2497100c2b849935c23af30d',
        'clone': 'mantine',
        'src': 'packages/@mantine/core/src',
        'manager': 'yarn',
    },
    {
        'name': 'chakra',
        'repo': 'chakra-ui/chakra-ui',
        'commit': 'fa9696f328ea5a0df8685c6d8460edbd779b63b9',
        'clone': 'chakra-ui',
        'src': 'packages/react/src',
        'manager': 'pnpm',
    },
    {
        'name': 'shadcn',
        'repo': 'shadcn-ui/ui',
        'commit': 'ba1503c64472b82256d9de95e2e62c5901a38e8b',
        'clone': 'shadcn-ui',
        'src': 'apps/v4',
        'manager': 'pnpm',
    },
    {
        'name': 'antd',
        'repo': 'ant-design/ant-design',
        'commit': 'ba7ccbaa7b4b58ac1ff8e6fe2e78dfa10bfb5538',
        'clone': 'ant-design',
        'src': 'components',
        # ant-design commits no lockfile and sets package-lock=false, so there is nothing to freeze against.
        'manager': 'npm',
    },
    {
        'name': 'spectrum',
        'repo': 'adobe/react-spectrum',
        'commit': '797762ed3526ea12f089e321bd3891066affad70',
        'clone': 'react-spectrum',
        'src': 'packages',
        'manager': 'yarn',
    },
]

REGRESSION_LIMIT = 1.1
# Rates may wobble by a prop or two when a corpus' component set shifts; anything larger is a regression.
RATE_TOLERANCE = 0.0005
COREPACK_VERSION = '0.34.0'
PRUNED_DIRS = {
    '__tests__', '__snapshots__', 'test', 'tests', 'stories', '__stories__', 'demo', '__demo__', 'docs', 'e2e',
    'dist', 'build', '.storybook', 'examples',
}
PRUNED_FILE = re.compile(r'\.(test|spec|stories|story)\.|\.d\.ts$')


# ─── Setup ────────────────────────────────────────────────────────────────────


def package_manager_env(base):
    caches = os.path.join(base, 'caches')
    env = dict(os.environ)
    env.update({
        'COREPACK_HOME': os.path.join(caches, 'corepack'),
        'COREPACK_ENABLE_DOWNLOAD_PROMPT': '0',
        'COREPACK_ENABLE_AUTO_PIN': '0',
        'YARN_CACHE_FOLDER': os.path.join(caches, 'yarn'),
        'YARN_GLOBAL_FOLDER': os.path.join(caches, 'yarn-global'),
        'YARN_ENABLE_GLOBAL_CACHE': '0',
        'YARN_ENABLE_SCRIPTS': '0',
        'YARN_NODE_LINKER': 'node-modules',
        'YARN_ENABLE_TELEMETRY': '0',
        'npm_config_cache': os.path.join(caches, 'npm'),
        'PATH': os.path.join(base, 'tools', 'bin') + os.pathsep + os.environ.get('PATH', ''),
    })
    return env


def ensure_package_managers(base, env, log):
    """pnpm and yarn shims from corepack, at the versions each repo's `packageManager` pins."""
    bin_dir = os.path.join(base, 'tools', 'bin')
    if os.path.exists(os.path.join(bin_dir, 'pnpm')):
        return
    os.makedirs(bin_dir, exist_ok=True)
    corepack = shutil.which('corepack')
    if corepack is None:
        # Node 25 and later no longer bundle corepack.
        tools = os.path.join(base, 'tools')
        subprocess.run(['npm', 'install', '--prefix', tools, '--no-audit', '--no-fund', f'corepack@{COREPACK_VERSION}'],
                       env=env, stdout=log, stderr=log, check=True)
        corepack = os.path.join(tools, 'node_modules', '.bin', 'corepack')
    subprocess.run([corepack, 'enable', '--install-directory', bin_dir, 'pnpm', 'yarn'], env=env, check=True)


def git(*args, cwd):
    subprocess.run(['git', *args], cwd=cwd, check=True, stdout=subprocess.DEVNULL)


def fetch(corpus, checkout):
    shutil.rmtree(checkout, ignore_errors=True)
    os.makedirs(checkout)
    git('init', '-q', cwd=checkout)
    git('remote', 'add', 'origin', f"https://github.com/{corpus['repo']}.git", cwd=checkout)
    # Network steps retry: one dropped connection shouldn't fail a scheduled run.
    for attempt in range(3):
        try:
            git('fetch', '-q', '--depth', '1', 'origin', corpus['commit'], cwd=checkout)
            git('checkout', '-q', 'FETCH_HEAD', cwd=checkout)
            return
        except subprocess.CalledProcessError:
            if attempt == 2:
                raise
            time.sleep(5)


def install(corpus, checkout, base, env, log):
    caches = os.path.join(base, 'caches')
    pnpm = ['--store-dir', os.path.join(caches, 'pnpm-store'), f"--config.cache-dir={os.path.join(caches, 'pnpm-cache')}"]
    commands = {
        'pnpm': [['pnpm', 'install', '--frozen-lockfile', '--ignore-scripts', *pnpm],
                 ['pnpm', 'install', '--no-frozen-lockfile', '--ignore-scripts', *pnpm]],
        'yarn': [['yarn', 'install', '--immutable', '--mode=skip-build'],
                 ['yarn', 'install', '--no-immutable', '--mode=skip-build']],
        'npm': [['npm', 'install', '--ignore-scripts', '--no-audit', '--no-fund']],
    }[corpus['manager']]
    # The lockfile first; a lockfile the pinned package manager rejects falls back to resolving again.
    for command in commands:
        if subprocess.run(command, cwd=checkout, env=env, stdout=log, stderr=log).returncode == 0:
            return
    raise RuntimeError(f"{corpus['name']}: dependency install failed, see {log.name}")


def prune(src):
    """What neither tool should document, inside the source directory only. node_modules is never entered."""
    for current, dirs, files in os.walk(src, topdown=True):
        keep = []
        for d in dirs:
            if d == 'node_modules':
                continue
            if d in PRUNED_DIRS:
                shutil.rmtree(os.path.join(current, d))
            else:
                keep.append(d)
        dirs[:] = keep
        for f in files:
            if PRUNED_FILE.search(f):
                os.remove(os.path.join(current, f))


def setup(args):
    corpora_dir = os.path.abspath(args.corpora)
    base = os.path.dirname(corpora_dir)
    os.makedirs(os.path.join(base, 'logs'), exist_ok=True)
    env = package_manager_env(base)
    with open(os.path.join(base, 'logs', 'corepack.log'), 'w') as log:
        ensure_package_managers(base, env, log)
    for corpus in selected(args.corpus):
        checkout = os.path.join(corpora_dir, corpus['clone'])
        marker = os.path.join(checkout, '.parity-complete')
        if os.path.exists(marker) and open(marker).read() == corpus['commit']:
            print(f"{corpus['name']}: ready at {corpus['commit'][:12]}")
            continue
        print(f"{corpus['name']}: fetching {corpus['commit'][:12]}", flush=True)
        fetch(corpus, checkout)
        print(f"{corpus['name']}: installing with {corpus['manager']}", flush=True)
        with open(os.path.join(base, 'logs', f"install-{corpus['name']}.log"), 'w') as log:
            install(corpus, checkout, base, env, log)
        prune(os.path.join(checkout, corpus['src']))
        with open(marker, 'w') as f:
            f.write(corpus['commit'])
        print(f"{corpus['name']}: ready at {corpus['commit'][:12]}")


# ─── Measures ─────────────────────────────────────────────────────────────────


def norm_name(name):
    """A component name as both tools may spell it: no `(file)` suffix, namespace or punctuation."""
    name = re.sub(r' \(.*\)$', '', name).split('/')[-1].split('.')[-1]
    return re.sub(r'[^a-z0-9]', '', name.lower())


def base_text(t):
    """RDT's own normalization: `| undefined` dropped, whitespace and `React.` ignored."""
    return re.sub(r'\s+', '', t.replace(' | undefined', '')).replace('React.', '')


def members(t):
    """A type's top-level union members."""
    out, depth, cur = [], 0, ''
    for ch in t:
        if ch in '<([{':
            depth += 1
        elif ch in '>)]}':
            depth -= 1
        if ch == '|' and depth == 0:
            out.append(cur)
            cur = ''
        else:
            cur += ch
    out.append(cur)
    return [m for m in out if m]


def cosmetic(t):
    """The type text with what doesn't change the type removed: quotes, `| undefined`, namespace prefixes
    (`Property.X`), `Array<T>` spelling, a trailing `;` in an object, outer parentheses and union order."""
    t = t.replace("'", '"').replace('React.', '')
    t = re.sub(r'\|\s*undefined\b', '', t)
    t = re.sub(r'\b(?:[A-Z]\w*\.)+(?=[A-Z])', '', t)
    t = re.sub(r'ReadonlyArray<([^<>]+)>', r'readonly \1[]', t)
    t = re.sub(r'Array<([^<>]+)>', r'\1[]', t)
    t = re.sub(r'\s+', '', t).replace(';}', '}').replace('&{}', '')
    while t.startswith('(') and t.endswith(')'):
        t = t[1:-1]
    return '|'.join(sorted(m.strip('()') for m in members(t)))


def control(t):
    """The Storybook control a type's text implies."""
    t = base_text(t)
    if t in ('boolean', 'false|true', 'true|false'):
        return 'boolean'
    if t == 'number':
        return 'number'
    if t == 'string':
        return 'text'
    if t == 'enum':
        return 'select'
    if '=>' in t:
        return 'function'
    if t in ('ReactNode', 'ReactElement', 'Element', 'ReactElement<any,string|JSXElementConstructor<any>>'):
        return 'node'
    return 'object'


def type_params(t):
    return set(re.findall(r'\b(?:[A-Z]|T[A-Z][a-zA-Z]*|Target|Props)\b', t))


UTILITY = re.compile(r'\b(Omit|Pick|Exclude|Extract|NonNullable|GetProp|Parameters|ReturnType)<')

# Where Caliper's type text carries less than RDT's (ratcheted), and two where RDT's carries less (reported only).
LOSS_PATTERNS = {
    'member read left as X["k"]': lambda o, r: '["' in o and '["' not in r,
    'readonly lost': lambda o, r: 'readonly ' in r and 'readonly ' not in o,
    'rest parameter lost': lambda o, r: '...' in r and '...' not in o,
    'unbound type parameter': lambda o, r: bool(type_params(o) - type_params(r)) and not type_params(r),
    'utility type left as written': lambda o, r: bool(UTILITY.search(o)) and not UTILITY.search(r),
    'conditional left as source': lambda o, r: ' extends ' in o and '?' in o and ' extends ' not in r,
    'template literal left': lambda o, r: '`' in o and '`' not in r,
    'RefObject shown as Ref': lambda o, r: 'RefObject<' in r and 'Ref<' in o and 'RefObject<' not in o,
    'typeof left in output': lambda o, r: 'typeof ' in o and 'typeof ' not in r,
    'unreduced & string': lambda o, r: o.endswith('& string') and r == 'string',
    'index signature dropped': lambda o, r: '[' in r and ']:' in r and ']:' not in o,
    'optional parameter lost': lambda o, r: bool(re.search(r'\(\w+\?:', r)) and not re.search(r'\(\w+\?:', o),
    'any/unknown where RDT is concrete': lambda o, r: o in ('any', 'unknown', '{}', 'object')
    and r not in ('any', 'unknown', '{}', 'object'),
}
RDT_LOSS_PATTERNS = {
    'enum RDT lists as string': lambda o, r: o == 'enum' and r == 'string',
    'RDT any/unknown where Caliper is concrete': lambda o, r: r in ('any', 'unknown') and o not in ('any', 'unknown'),
}


def load_json(text):
    """Caliper's RDT output, past anything printed before the JSON."""
    return json.loads(text[text.index('{'):])


def caliper_rdt(binary, src, deps):
    args = [binary, 'extract', '--src', src, '--format', 'rdt', '--html-attributes', 'full', '--include-unexported',
            '--quiet']
    if deps:
        args.append('--load-dependencies')
    out = subprocess.run(args, capture_output=True, text=True, check=True).stdout
    return load_json(out)


def measure(rdt, mine, clone_root):
    """The coverage, agreement and loss-pattern counts of one Caliper run against RDT's dump."""
    by_file = {}
    for c in mine.values():
        rel = os.path.relpath(c['filePath'], clone_root)
        by_file.setdefault(rel, []).append((norm_name(c['displayName']), c))
    counts = {
        'rdt_components': 0, 'components_matched': 0, 'rdt_props': 0, 'props_reported': 0,
        'shared': 0, 'same_text': 0, 'same_or_cosmetic': 0, 'same_control': 0,
    }
    loss = {label: 0 for label in LOSS_PATTERNS}
    rdt_loss = {label: 0 for label in RDT_LOSS_PATTERNS}
    for comp in rdt:
        if not comp['props']:
            continue
        counts['rdt_components'] += 1
        rel = os.path.relpath(comp['file'], clone_root) if os.path.isabs(comp['file']) else comp['file']
        m = next((c for n, c in by_file.get(rel, []) if n == norm_name(comp['name'])), None)
        if m is None:
            continue
        counts['components_matched'] += 1
        for prop, info in comp['props'].items():
            theirs = info['type'] if isinstance(info['type'], str) else json.dumps(info['type'])
            react = 'node_modules/@types/react/' in (info.get('parent') or '')
            if not react:
                counts['rdt_props'] += 1
            if prop not in m['props']:
                continue
            ours = m['props'][prop]['type']['name']
            if not react:
                counts['props_reported'] += 1
                counts['shared'] += 1
                same = base_text(ours) == base_text(theirs)
                counts['same_text'] += same
                counts['same_or_cosmetic'] += same or cosmetic(ours) == cosmetic(theirs)
                counts['same_control'] += control(ours) == control(theirs)
            o, r = ours.replace('React.', ''), theirs.replace('React.', '')
            for label, test in LOSS_PATTERNS.items():
                loss[label] += bool(test(o, r))
            for label, test in RDT_LOSS_PATTERNS.items():
                rdt_loss[label] += bool(test(o, r))
    return {**counts, 'loss': loss, 'rdt_loss': rdt_loss}


def run_once(args):
    """Wall seconds and peak RSS in MiB of one process."""
    start = time.time()
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    _, status, usage = os.wait4(proc.pid, 0)
    elapsed = time.time() - start
    proc.returncode = os.waitstatus_to_exitcode(status)
    if proc.returncode != 0:
        raise RuntimeError(f'{args} exited with {proc.returncode}')
    # ru_maxrss is bytes on macOS and kilobytes on Linux.
    rss = usage.ru_maxrss / 2**20 if sys.platform == 'darwin' else usage.ru_maxrss / 1024
    return elapsed, rss


def timings(binaries, src, runs):
    """Best-of-N wall ms and peak RSS for each binary, with and without --load-dependencies, runs interleaved."""
    result = {}
    for deps in (False, True):
        mode = 'on' if deps else 'off'
        best = {label: (float('inf'), 0.0) for label in binaries}
        for label, binary in binaries.items():
            subprocess.run([binary, 'extract', '--src', src, '--json'] + (['--load-dependencies'] if deps else []),
                           capture_output=True, check=True)
        for _ in range(runs):
            for label, binary in binaries.items():
                args = [binary, 'extract', '--src', src, '--json'] + (['--load-dependencies'] if deps else [])
                elapsed, rss = run_once(args)
                ms, peak = best[label]
                best[label] = (min(ms, elapsed * 1000), max(peak, rss))
        for label, (ms, rss) in best.items():
            result.setdefault(label, {})[mode] = {'ms': round(ms, 1), 'rss_mib': round(rss, 1)}
    return result


def rdt_dump(corpus, src, clone_root, rdt_dir):
    """RDT's dump for this corpus, made once per commit."""
    os.makedirs(rdt_dir, exist_ok=True)
    out = os.path.join(rdt_dir, f"{corpus['name']}-{corpus['commit'][:12]}.json")
    if not os.path.exists(out):
        files = []
        for current, dirs, names in os.walk(src):
            dirs[:] = [d for d in dirs if d != 'node_modules']
            files += [os.path.join(current, n) for n in names if n.endswith('.tsx')]
        list_file = os.path.join(rdt_dir, f"{corpus['name']}.files.txt")
        with open(list_file, 'w') as f:
            f.write('\n'.join(sorted(files)) + '\n')
        # The source directory as its own program: the same component files RDT's dumps here have always used.
        tsconfig = os.path.join(rdt_dir, f"{corpus['name']}.tsconfig.json")
        with open(tsconfig, 'w') as f:
            json.dump({
                'include': [f'{src}/**/*.ts', f'{src}/**/*.tsx'],
                'compilerOptions': {
                    'target': 'ES2022', 'module': 'ESNext', 'moduleResolution': 'bundler', 'jsx': 'react-jsx',
                    'strict': True, 'skipLibCheck': True, 'noEmit': True,
                },
            }, f, indent=2)
        print(f"{corpus['name']}: dumping RDT", file=sys.stderr, flush=True)
        subprocess.run(['node', '--max-old-space-size=8192', os.path.join(ROOT, 'scripts', 'parity-rdt.mjs'),
                        list_file, tsconfig, out], check=True)
    with open(out) as f:
        dump = json.load(f)
    # Paths relative to the clone, so a dump made in another checkout location still matches.
    for comp in dump['components']:
        comp['file'] = os.path.relpath(comp['file'], clone_root)
    return dump


def selected(names):
    if not names:
        return CORPORA
    unknown = set(names) - {c['name'] for c in CORPORA}
    if unknown:
        sys.exit(f"unknown corpus: {', '.join(sorted(unknown))}")
    return [c for c in CORPORA if c['name'] in names]


def run(args):
    corpora_dir = os.path.abspath(args.corpora)
    version = subprocess.run([args.bin, '--version'], capture_output=True, text=True).stdout.strip()
    report = {'caliper': version, 'runs': args.runs, 'corpora': {}}
    for corpus in selected(args.corpus):
        clone_root = os.path.join(corpora_dir, corpus['clone'])
        src = os.path.join(clone_root, corpus['src'])
        if not os.path.isdir(src):
            sys.exit(f"{corpus['name']}: {src} is missing; run `scripts/parity.py setup` first")
        dump = rdt_dump(corpus, src, clone_root, os.path.abspath(args.rdt_dir))
        entry = {
            'commit': corpus['commit'],
            'rdt': {'ms': round(dump['wallMs'], 1), 'rss_mib': round(dump['peakRssMiB'], 1)},
            'modes': {},
        }
        for deps in (False, True):
            print(f"{corpus['name']}: measuring, dependencies {'on' if deps else 'off'}", file=sys.stderr, flush=True)
            entry['modes']['on' if deps else 'off'] = measure(dump['components'], caliper_rdt(args.bin, src, deps),
                                                              clone_root)
        if not args.no_timing:
            binaries = {'head': args.bin, **({'base': args.base_bin} if args.base_bin else {})}
            print(f"{corpus['name']}: timing", file=sys.stderr, flush=True)
            entry['timing'] = timings(binaries, src, args.runs)
        report['corpora'][corpus['name']] = entry
    return report


# ─── Report and gates ─────────────────────────────────────────────────────────


def rate(a, b):
    return a / b if b else 0.0


def pct(a, b):
    return f'{100 * rate(a, b):.1f}%'


def markdown(report):
    lines = [f"Caliper {report['caliper']}, best of {report['runs']} runs.", '']
    for mode, title in (('on', 'with --load-dependencies'), ('off', 'without dependencies')):
        lines += [f'### Against RDT, {title}', '',
                  '| Corpus | Components matched | Non-React props reported | Same text | Same or cosmetic | '
                  'Same control |', '| --- | --: | --: | --: | --: | --: |']
        for name, entry in report['corpora'].items():
            m = entry['modes'][mode]
            lines.append(
                f"| {name} | {m['components_matched']}/{m['rdt_components']} | "
                f"{m['props_reported']:,}/{m['rdt_props']:,} ({pct(m['props_reported'], m['rdt_props'])}) | "
                f"{pct(m['same_text'], m['shared'])} | {pct(m['same_or_cosmetic'], m['shared'])} | "
                f"{pct(m['same_control'], m['shared'])} |")
        lines.append('')
    names = list(report['corpora'])
    lines += ['### Loss patterns, with --load-dependencies (shared props)', '',
              '| Pattern | ' + ' | '.join(names) + ' |', '| --- |' + ' --: |' * len(names)]
    first = report['corpora'][names[0]]['modes']['on']
    for group in ('loss', 'rdt_loss'):
        for label in first[group]:
            suffix = ' (RDT carries less)' if group == 'rdt_loss' else ''
            cells = [f"{report['corpora'][n]['modes']['on'][group][label]:,}" for n in names]
            lines.append(f'| {label}{suffix} | ' + ' | '.join(cells) + ' |')
    lines.append('')
    if all('timing' in e for e in report['corpora'].values()):
        lines += ['### Time and memory', '',
                  '| Corpus | Caliper default | Caliper --load-dependencies | Base default | Base --load-dependencies | '
                  'RDT (dependencies resolvable) |', '| --- | --: | --: | --: | --: | --: |']
        for name, entry in report['corpora'].items():
            t = entry['timing']
            cell = lambda r: f"{r['ms']:.0f} ms, {r['rss_mib']:.0f} MiB"
            base = t.get('base')
            lines.append(f"| {name} | {cell(t['head']['off'])} | {cell(t['head']['on'])} | "
                         f"{cell(base['off']) if base else '-'} | {cell(base['on']) if base else '-'} | "
                         f"{entry['rdt']['ms'] / 1000:.1f} s, {entry['rdt']['rss_mib']:,.0f} MiB |")
        lines.append('')
    return '\n'.join(lines)


def check(report, baseline):
    """Each gate that fails, as a message."""
    failures = []
    for name, entry in report['corpora'].items():
        old = baseline.get('corpora', {}).get(name)
        for mode, m in entry['modes'].items():
            if old is None or mode not in old['modes']:
                continue
            o = old['modes'][mode]
            for label, (num, den) in {
                'components matched': ('components_matched', 'rdt_components'),
                'props reported': ('props_reported', 'rdt_props'),
                'same text': ('same_text', 'shared'),
                'same or cosmetic': ('same_or_cosmetic', 'shared'),
                'same control': ('same_control', 'shared'),
            }.items():
                now, before = rate(m[num], m[den]), rate(o[num], o[den])
                if now < before - RATE_TOLERANCE:
                    failures.append(f'{name} ({mode}): {label} fell from {100 * before:.2f}% to {100 * now:.2f}%')
            for label, count in m['loss'].items():
                before = o['loss'].get(label)
                if before is not None and count > before:
                    failures.append(f'{name} ({mode}): "{label}" grew from {before:,} to {count:,} props')
        timing = entry.get('timing', {})
        if 'base' in timing:
            for mode in ('off', 'on'):
                head, base = timing['head'][mode]['ms'], timing['base'][mode]['ms']
                if head > REGRESSION_LIMIT * base:
                    failures.append(f'{name} ({mode}): {head:.0f} ms is more than {REGRESSION_LIMIT}x the base '
                                    f'build ({base:.0f} ms)')
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    default_corpora = os.path.join(ROOT, '.parity', 'corpora')
    p_setup = sub.add_parser('setup', help='fetch, install and prune the corpora')
    p_setup.add_argument('--corpora', default=default_corpora)
    p_setup.add_argument('--corpus', action='append', default=[])
    p_run = sub.add_parser('run', help='measure a build against RDT')
    p_run.add_argument('--bin', required=True)
    p_run.add_argument('--base-bin')
    p_run.add_argument('--corpora', default=default_corpora)
    p_run.add_argument('--rdt-dir', default=os.path.join(ROOT, '.parity', 'rdt'))
    p_run.add_argument('--corpus', action='append', default=[])
    p_run.add_argument('--runs', type=int, default=7)
    p_run.add_argument('--no-timing', action='store_true')
    p_run.add_argument('--out')
    p_run.add_argument('--markdown')
    p_run.add_argument('--baseline')
    p_run.add_argument('--check', action='store_true')
    args = parser.parse_args()

    if args.command == 'setup':
        setup(args)
        return
    report = run(args)
    table = markdown(report)
    print(table, flush=True)
    if args.out:
        with open(args.out, 'w') as f:
            json.dump(report, f, indent=2)
            f.write('\n')
    if args.markdown:
        with open(args.markdown, 'w') as f:
            f.write(table + '\n')
    if args.check:
        if not args.baseline:
            sys.exit('--check needs --baseline')
        with open(args.baseline) as f:
            failures = check(report, json.load(f))
        if failures:
            print('\nGates failed:\n' + '\n'.join(f'- {f}' for f in failures), file=sys.stderr)
            sys.exit(1)
        print('\nAll gates passed.', file=sys.stderr)


if __name__ == '__main__':
    main()
