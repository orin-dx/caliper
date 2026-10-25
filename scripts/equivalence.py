#!/usr/bin/env python3
"""Does this build publish the same documentation as a base build?

Resolver internals (memos, interning, scheduling) should change speed, not output. This runs `extract --json` from a
base build and this working tree on the parity corpora (scripts/parity.py setup fetches them) in four configurations,
canonicalizes both outputs (the interned `props`, `propSets` and `nameLists` tables expanded in place; timings and
cache counters dropped), and reports per corpus and configuration either `identical` or a classified diff:
components added or removed, props added, removed or changed (type, or required/default/description/tags), other
component fields changed, diagnostics added or removed, enums and stats that differ.

Any diff fails the run unless scripts/equivalence-allow.json lists it. The file holds `allow`, a list of fnmatch
patterns matched against `<corpus>/<config> <kind> <item>` as printed in the report; it is empty unless a change is
meant to alter output, and then the patterns say exactly which differences are meant.

Two optional stages:
  --lazy {off,fixtures,all}  lazy-vs-batch parity (crates/core/tests/lazy_parity.rs): the fixtures, and with `all`
                             also the selected corpora's sources. Default: all.
  --oracle                   TypeScript 7.1's checker against this build (scripts/parity.py oracle), failing when
                             agreement falls or contradicted props rise past scripts/oracle-baseline.json.

Every stage has a time limit (--timeout-build, --timeout-extract, --timeout-lazy, --timeout-oracle, in seconds). A
stage that runs over is killed with its whole process group and reported by name with the tail of its output, and the
run fails: a hung build, child process or deadlocked test can never stall CI silently.

Usage: scripts/equivalence.py [BASE] [--bin BIN] [--corpora DIR] [--corpus NAME]... [--config NAME]... [--lazy MODE]
                              [--oracle] [--examples N] [--allow FILE] [--work DIR] [--out FILE] [--markdown FILE]
                              [--timeout-build S] [--timeout-extract S] [--timeout-lazy S] [--timeout-oracle S]
  BASE   a git ref (default origin/main; built once per commit in a scratch worktree) or an executable.
  --bin  this build (default: `cargo build --release -p caliper-docgen` in this checkout).
"""
import argparse
import collections
import fnmatch
import json
import os
import shutil
import signal
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import parity  # noqa: E402  (corpus list and setup, the oracle and its gate)

ROOT = parity.ROOT
CONFIGS = {
    'default': [],
    'deps': ['--load-dependencies'],
    'html-full': ['--html-attributes', 'full'],
    'deps+html-full': ['--load-dependencies', '--html-attributes', 'full'],
}
# Unexported components stay in: more components, more coverage.
COMMON = ['--json', '--quiet', '--include-unexported']
# Counters that depend on the parse cache or the clock, not on what was resolved.
VOLATILE_STATS = {'durationMs', 'dtsFilesParsed', 'dtsCacheHits'}
DEFAULT_ALLOW = os.path.join(ROOT, 'scripts', 'equivalence-allow.json')


# ─── Builds ───────────────────────────────────────────────────────────────────


class StageTimeout(Exception):
    """A stage ran past its limit and was killed."""


def run_bounded(command, *, stage, timeout, cwd=ROOT, env=None, check=True):
    """`command` with a hard time limit, no stdin and its own process group, so a timeout kills its children too.

    Returns the CompletedProcess (stdout and stderr captured as text). Raises StageTimeout naming `stage` and showing the
    tail of what the command printed, or CalledProcessError when `check` and the command fails.
    """
    process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        out, err = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        out, err = process.communicate()
        tail = '\n'.join(((out or '') + (err or '')).strip().splitlines()[-15:])
        raise StageTimeout(f'{stage} did not finish within {timeout}s and was killed. Last output:\n{tail}') from None
    if check and process.returncode != 0:
        raise subprocess.CalledProcessError(process.returncode, command, out, err)
    return subprocess.CompletedProcess(command, process.returncode, out, err)


def run(*args, cwd=ROOT, timeout=None, **kwargs):
    """A git or cargo step; `timeout` (seconds) bounds it the way `run_bounded` does."""
    if timeout is None:
        return subprocess.run(args, cwd=cwd, check=True, **kwargs)
    return run_bounded(list(args), stage=' '.join(args[:3]), timeout=timeout, cwd=cwd)


def build_head(timeout):
    run('cargo', 'build', '--release', '--locked', '-p', 'caliper-docgen', timeout=timeout)
    return os.path.join(ROOT, 'target', 'release', 'caliper-docgen')


def resolve_base(base, work, timeout):
    """An executable as given; a git ref built once per commit in a scratch worktree, then cached by commit."""
    if os.path.isfile(base) and os.access(base, os.X_OK):
        return os.path.abspath(base)
    sha = run('git', 'rev-parse', '--verify', f'{base}^{{commit}}', capture_output=True, text=True).stdout.strip()
    cached = os.path.join(work, 'bin', sha)
    if os.path.exists(cached):
        return cached
    tree = os.path.join(work, 'base-tree')
    run('git', 'worktree', 'prune')
    shutil.rmtree(tree, ignore_errors=True)
    run('git', 'worktree', 'add', '--detach', tree, sha, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        # One target directory for both builds: the dependencies are shared, only this workspace's crates rebuild.
        run('cargo', 'build', '--release', '--locked', '-p', 'caliper-docgen', '--manifest-path',
            os.path.join(tree, 'Cargo.toml'), '--target-dir', os.path.join(work, 'base-target'), timeout=timeout)
    finally:
        run('git', 'worktree', 'remove', '--force', tree, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.makedirs(os.path.dirname(cached), exist_ok=True)
    shutil.copy(os.path.join(work, 'base-target', 'release', 'caliper-docgen'), cached)
    return cached


# ─── Canonical form ───────────────────────────────────────────────────────────


def extract(binary, src, flags, cache_dir, timeout):
    command = [binary, 'extract', '--src', src, '--cache-dir', cache_dir, *COMMON, *flags]
    out = run_bounded(command, stage=f'extract {os.path.basename(src)}', timeout=timeout).stdout
    return json.loads(out[out.index('{'):])


def canonical(output):
    """The output with its interned tables expanded where they are used, and nothing that varies run to run.

    A component's `props` (prop set ids) becomes one map of name to prop; an inheritance layer's `omitted` and a
    diagnostic's `affects` (name list ids) become the names. Everything else is kept as published, so a change to
    any field shows up.
    """
    props, sets, lists = output.get('props', {}), output.get('propSets', {}), output.get('nameLists', {})

    def names(value):
        return list(lists[value]) if isinstance(value, str) else value

    components = {}
    for key, c in output['components'].items():
        expanded = {}
        for set_id in c['props']:
            for name, prop_id in sets[set_id].items():
                expanded[name] = props[prop_id]
        layers = [{**layer, 'omitted': names(layer.get('omitted'))} for layer in c.get('inheritance', [])]
        notable = {name: props.get(prop_id, prop_id) for name, prop_id in c.get('notableInherited', {}).items()}
        components[key] = {**c, 'props': expanded, 'inheritance': layers, 'notableInherited': notable}
    diagnostics = [{**d, 'affects': names(d['affects'])} if 'affects' in d else d for d in output['diagnostics']]
    return {
        'version': output.get('schemaVersion'),
        'components': components,
        'enums': output.get('enums', {}),
        'diagnostics': collections.Counter(dump(d) for d in diagnostics),
        'stats': {k: v for k, v in output.get('stats', {}).items() if k not in VOLATILE_STATS},
    }


def dump(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


# ─── Diff ─────────────────────────────────────────────────────────────────────

PROP_FIELDS = ('required', 'defaultValue', 'description', 'tags', 'parent', 'declarations')


def diff(base, head):
    """(kind, item, detail) for each difference, `base` to `head`."""
    found = []
    if base['version'] != head['version']:
        found.append(('schema-version', 'schemaVersion', f"{base['version']} -> {head['version']}"))
    bc, hc = base['components'], head['components']
    for key in sorted(set(bc) - set(hc)):
        found.append(('component-removed', key, ''))
    for key in sorted(set(hc) - set(bc)):
        found.append(('component-added', key, ''))
    for key in sorted(set(bc) & set(hc)):
        b, h = bc[key], hc[key]
        if b == h:
            continue
        for name in sorted(set(b['props']) - set(h['props'])):
            found.append(('prop-removed', f'{key}.{name}', dump(b['props'][name]['type'])[:160]))
        for name in sorted(set(h['props']) - set(b['props'])):
            found.append(('prop-added', f'{key}.{name}', dump(h['props'][name]['type'])[:160]))
        for name in sorted(set(b['props']) & set(h['props'])):
            bp, hp = b['props'][name], h['props'][name]
            if bp == hp:
                continue
            if bp['type'] != hp['type']:
                found.append(('prop-type-changed', f'{key}.{name}', f"{dump(bp['type'])[:120]} -> {dump(hp['type'])[:120]}"))
            for field in PROP_FIELDS:
                if bp.get(field) != hp.get(field):
                    found.append(('prop-field-changed', f'{key}.{name}.{field}',
                                  f'{dump(bp.get(field))[:80]} -> {dump(hp.get(field))[:80]}'))
        for name in sorted(set(b['notableInherited']) - set(h['notableInherited'])):
            found.append(('notable-removed', f'{key}.{name}', ''))
        for name in sorted(set(h['notableInherited']) - set(b['notableInherited'])):
            found.append(('notable-added', f'{key}.{name}', ''))
        for name in sorted(set(b['notableInherited']) & set(h['notableInherited'])):
            if b['notableInherited'][name] != h['notableInherited'][name]:
                found.append(('notable-changed', f'{key}.{name}', ''))
        for field in sorted((set(b) | set(h)) - {'props', 'notableInherited'}):
            if b.get(field) != h.get(field):
                found.append(('component-field-changed', f'{key}.{field}',
                              f'{dump(b.get(field))[:100]} -> {dump(h.get(field))[:100]}'))
    for text, n in sorted((base['diagnostics'] - head['diagnostics']).items()):
        found.append(('diagnostic-removed', diagnostic_item(text), f'x{n}'))
    for text, n in sorted((head['diagnostics'] - base['diagnostics']).items()):
        found.append(('diagnostic-added', diagnostic_item(text), f'x{n}'))
    for key in sorted(set(base['enums']) | set(head['enums'])):
        if base['enums'].get(key) != head['enums'].get(key):
            found.append(('enum-changed', key, ''))
    for key in sorted(set(base['stats']) | set(head['stats'])):
        if base['stats'].get(key) != head['stats'].get(key):
            found.append(('stat-changed', key, f"{base['stats'].get(key)} -> {head['stats'].get(key)}"))
    return found


def diagnostic_item(text):
    """A diagnostic as the report names it: severity, code and message first, so an allow pattern can match on them
    whatever the length of its `affects` list (which sorts first in the canonical JSON), then the JSON itself."""
    try:
        d = json.loads(text)
        head = f"{d['severity']} {d['code']} {str(d['message'])[:100]}"
    except (ValueError, KeyError, TypeError):
        return text[:200]
    return f'{head} | {text[:100]}'


def allowed(label, kind, item, patterns):
    return any(fnmatch.fnmatchcase(f'{label} {kind} {item}', p) for p in patterns)


# ─── Stages ───────────────────────────────────────────────────────────────────


def compare(args, base_bin, head_bin, patterns):
    """Per corpus and configuration, the differences the allow-list does not cover, and the report text."""
    report, failures = [], 0
    configs = args.config or list(CONFIGS)
    for corpus in parity.selected(args.corpus):
        src = os.path.join(args.corpora, corpus['clone'], corpus['src'])
        if not os.path.isdir(src):
            sys.exit(f"{corpus['name']}: {src} is missing; run `scripts/parity.py setup --corpus {corpus['name']}`")
        for name in configs:
            label = f"{corpus['name']}/{name}"
            start = time.time()
            outputs = []
            for tag, binary in (('base', base_bin), ('head', head_bin)):
                cache = os.path.join(args.work, 'cache', tag)
                os.makedirs(cache, exist_ok=True)
                outputs.append(canonical(extract(binary, src, CONFIGS[name], cache, args.timeout_extract)))
            found = diff(*outputs)
            unexpected = [d for d in found if not allowed(label, d[0], d[1], patterns)]
            n_components = len(outputs[1]['components'])
            took = f'{time.time() - start:.1f}s'
            if not found:
                report.append(f'{label}: identical ({n_components} components, {took})')
                continue
            by_kind = collections.Counter(kind for kind, _, _ in unexpected)
            if not unexpected:
                report.append(f'{label}: {len(found)} differences, all allowed ({n_components} components, {took})')
                continue
            failures += 1
            summary = ', '.join(f'{n} {kind}' for kind, n in sorted(by_kind.items()))
            report.append(f'{label}: DIFFERS: {summary} ({n_components} components, {took})')
            shown = collections.Counter()
            for kind, item, detail in unexpected:
                shown[kind] += 1
                if shown[kind] <= args.examples:
                    report.append(f'    {kind}: {item}' + (f'  [{detail}]' if detail else ''))
            for kind, n in sorted(shown.items()):
                if n > args.examples:
                    report.append(f'    ... and {n - args.examples} more {kind}')
    return failures, report


def lazy_parity(args, mode):
    """The lazy session against batch extraction: the fixtures, and the selected corpora's sources."""
    command = ['cargo', 'test', '-p', 'caliper-docgen-core', '--test', 'lazy_parity', '--release', '--locked', '--']
    env = dict(os.environ)
    if mode == 'all':
        env['CALIPER_PARITY_SRC'] = os.pathsep.join(
            os.path.join(args.corpora, c['clone'], c['src']) for c in parity.selected(args.corpus))
        command.append('--include-ignored')
    result = run_bounded(command, stage=f'lazy parity ({mode})', timeout=args.timeout_lazy, env=env, check=False)
    summary = [line for line in result.stdout.splitlines() if line.startswith('test result') or ' FAILED' in line]
    detail = '\n'.join(summary) or result.stderr[-2000:]
    failures = result.stdout[result.stdout.find('failures:'):] if result.returncode != 0 and 'failures:' in result.stdout else ''
    if failures:
        # The panic message of each failed test: without it a failed gate says that it failed and not why.
        detail += '\n' + failures[:3000]
    return result.returncode == 0, f'lazy parity ({mode}): {"ok" if result.returncode == 0 else "FAILED"}\n{detail}'


def oracle_stage(args, head_bin):
    ns = argparse.Namespace(bin=head_bin, corpora=args.corpora, corpus=args.corpus,
                            work=os.path.join(args.work, 'oracle'), timeout=args.timeout_oracle)
    current = parity.oracle(ns)
    with open(args.oracle_baseline) as f:
        baseline = json.load(f)
    lines = ['TypeScript 7.1 oracle (verdicts per shared prop):']
    for name, e in current['corpora'].items():
        old = baseline.get('corpora', {}).get(name, {}).get('verdicts')
        shared = e['props_shared'] or 1
        cell = lambda v: f"{100 * e['verdicts'][v] / shared:.2f}%" + (
            f" (was {100 * old[v] / (baseline['corpora'][name]['props_shared'] or 1):.2f}%)" if old else '')
        lines.append(f"  {name}: wrong {cell('wrong')}, less precise {cell('less precise')}")
    failures = parity.oracle_check(current, baseline)
    lines += [f'  FAIL {f}' for f in failures]
    return not failures, '\n'.join(lines)


# ─── Main ─────────────────────────────────────────────────────────────────────


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('base', nargs='?', default='origin/main')
    parser.add_argument('--bin')
    parser.add_argument('--corpora', default=os.path.join(ROOT, '.parity', 'corpora'))
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--config', action='append', default=[], choices=list(CONFIGS))
    parser.add_argument('--lazy', choices=['off', 'fixtures', 'all'], default='all')
    parser.add_argument('--oracle', action='store_true')
    parser.add_argument('--oracle-baseline', default=os.path.join(ROOT, 'scripts', 'oracle-baseline.json'))
    parser.add_argument('--examples', type=int, default=5, help='differences shown per kind')
    parser.add_argument('--allow', default=DEFAULT_ALLOW)
    parser.add_argument('--work', default=os.path.join(ROOT, '.parity', 'equivalence'))
    parser.add_argument('--out')
    parser.add_argument('--markdown')
    for stage, default in (('build', 2400), ('extract', 600), ('lazy', 1800), ('oracle', 3600)):
        parser.add_argument(f'--timeout-{stage}', type=int, default=default, metavar='S',
                            help=f'seconds the {stage} stage may take (default {default})')
    args = parser.parse_args()
    args.corpora = os.path.abspath(args.corpora)
    args.work = os.path.abspath(args.work)
    os.makedirs(args.work, exist_ok=True)

    with open(args.allow) as f:
        patterns = json.load(f)['allow']
    try:
        head_bin = os.path.abspath(args.bin) if args.bin else build_head(args.timeout_build)
        base_bin = resolve_base(args.base, args.work, args.timeout_build)
    except StageTimeout as timeout:
        sys.exit(f'{timeout}')

    ok, lines = True, []
    for name, stage in (
        ('compare', lambda: (lambda f, l: (f == 0, l))(*compare(args, base_bin, head_bin, patterns))),
        ('lazy', lambda: lazy_parity(args, args.lazy) if args.lazy != 'off' else (True, [])),
        ('oracle', lambda: oracle_stage(args, head_bin) if args.oracle else (True, [])),
    ):
        # A stage that hangs fails the run with its name, and the stages after it still run and report.
        try:
            good, text = stage()
        except (StageTimeout, subprocess.TimeoutExpired) as timeout:
            good, text = False, f'{name} stage: TIMED OUT\n{timeout}'
        ok = ok and good
        lines += text if isinstance(text, list) else [text] if text else []

    verdict = 'Output is equivalent to the base.' if ok else 'Output differs from the base, or a gate failed.'
    text = '\n'.join(lines + ['', verdict])
    print(text)
    if args.markdown:
        with open(args.markdown, 'w') as f:
            f.write('```\n' + text + '\n```\n')
    if args.out:
        with open(args.out, 'w') as f:
            json.dump({'ok': ok, 'lines': lines}, f, indent=2)
            f.write('\n')
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main()
