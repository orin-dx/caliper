#!/usr/bin/env python3
"""Measure the experimental TypeScript 7.1 checker modes against the structural default, on the corpora scripts/corpora.py
sets up (or any directory laid out like it).

For each corpus it runs
  structural: `extract` with the defaults and dependency loading (the stock configuration),
  batch:      the same with `--experimental-semantic-types` (the CLI's whole-project checker pass),
  session:    one lazy session driven the way the Storybook addon drives it (scripts/checker_modes_session.mjs), with the
              checker off (the structural first-load numbers) and `semanticTypes: 'on-demand'` at two waits,
and reports wall time and peak RSS (median and spread of several runs, with the machine's load average), the props whose
type, requiredness, default or presence differ from the structural result, how often the on-demand checker answered in
time, and, as information only, the checker oracle's verdict on each mode. The oracle IS the checker, so a checker mode
agrees with it almost by construction: do not read its agree figure as accuracy (docs/measurement.md, "Checker modes").

The differing props go to a JSON file, `[{mode, corpus, file, component, prop, kinds, cosmetic, structural, checker}]`, the
input for judging them by methods that do not use the checker oracle.

Usage: scripts/checker_modes.py --bin target/release/caliper-docgen [--corpora DIR] [--corpus NAME]... [--runs 3]
       [--count 50] [--diff-out FILE] [--out FILE] [--tsc PATH] [--skip-oracle] [--skip-batch] [--skip-session]
       [--fallbacks]
"""
import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile

import corpora
from corpora import CORPORA, ROOT, run_once

DEFAULT_TSC = os.path.join(ROOT, 'apps', 'oracle', 'node_modules', 'typescript-beta', 'bin', 'tsc')
SESSION_SCRIPT = os.path.join(ROOT, 'scripts', 'checker_modes_session.mjs')
ORACLE = os.path.join(ROOT, 'apps', 'oracle', 'src', 'oracle.ts')
WAITS = (150, 600)
COMMON = ['--html-attributes', 'full', '--load-dependencies', '--quiet']


# --- comparing props -----------------------------------------------------------------------------------------------


def split_top(text, separator):
    """`text` split on `separator` where it is not inside brackets, braces, parentheses, angle brackets or quotes."""
    parts, depth, quote, start = [], 0, None, 0
    for at, ch in enumerate(text):
        if quote:
            if ch == quote and text[at - 1] != '\\':
                quote = None
        elif ch in '"\'`':
            quote = ch
        elif ch in '([{<':
            depth += 1
        elif ch in ')]}':
            depth -= 1
        elif ch == '>' and text[at - 1] != '=':
            depth -= 1
        elif ch == separator and depth == 0:
            parts.append(text[start:at])
            start = at + 1
    parts.append(text[start:])
    return parts


def top_level_arrow(text):
    """The index of the first `=>` outside brackets, braces, parentheses, angle brackets and quotes, or -1."""
    depth, quote = 0, None
    for at, ch in enumerate(text):
        if quote:
            if ch == quote and text[at - 1] != '\\':
                quote = None
        elif ch in '"\'`':
            quote = ch
        elif ch in '([{<':
            depth += 1
        elif ch in ')]}':
            depth -= 1
        elif ch == '>' and text[at - 1] != '=':
            depth -= 1
        elif ch == '=' and text[at + 1:at + 2] == '>' and depth == 0:
            return at
    return -1


def loose_type(text):
    """A type's text with the spellings that do not change its meaning removed: spacing, `| undefined`, the order of a
    union's members (after a function's `=>` too) and an `Array<T>` / `T[]` choice are not differences. A rest or optional
    parameter marker is a difference and stays."""
    if text is None:
        return None
    text = re.sub(r'\s+', ' ', text).strip()
    text = re.sub(r'\s*\|\s*undefined\b', '', text)
    text = re.sub(r'\bundefined\s*\|\s*', '', text)
    arrow = top_level_arrow(text)
    if arrow >= 0:
        return loose_type(text[:arrow]) + ' => ' + loose_type(text[arrow + 2:])
    members = [re.sub(r'^Array<(.*)>$', r'\1[]', m.strip()) for m in split_top(text, '|')]
    return ' | '.join(sorted(m for m in members if m))


def prop_facts(prop):
    """One prop of RDT-shaped output as the facts compared: type text, required and default."""
    type_ = prop.get('type') or {}
    default = prop.get('defaultValue')
    parent = (prop.get('parent') or {}).get('fileName') or ''
    return {
        'type': type_.get('raw') or type_.get('name'),
        'required': bool(prop.get('required')),
        'default': default.get('value') if isinstance(default, dict) else None,
        'react_declared': '/node_modules/@types/react/' in parent,
    }


def component_key(component, root):
    """A component's identity across runs: its file relative to the source root, and its name."""
    path = component.get('filePath') or component.get('file') or ''
    rel = os.path.relpath(os.path.realpath(path), os.path.realpath(root)) if path else ''
    return rel, component.get('displayName') or ''


def by_component(rdt, root):
    """RDT-shaped output (`{key: {displayName, filePath, props}}`) as {(file, name): {prop: facts}}."""
    out = {}
    for component in rdt.values():
        out[component_key(component, root)] = {name: prop_facts(p) for name, p in component.get('props', {}).items()}
    return out


def differing(structural, checker):
    """The differences of one component's props: [(prop, kinds, cosmetic, structural facts, checker facts)]. `kinds` names
    what differs (`type`, `required`, `default`, `only_structural`, `only_checker`); `cosmetic` is true when the only
    difference is a type spelled differently (`loose_type` agrees)."""
    rows = []
    for name in sorted(set(structural) | set(checker)):
        a, b = structural.get(name), checker.get(name)
        if a is None:
            rows.append((name, ['only_checker'], False, None, b))
        elif b is None:
            rows.append((name, ['only_structural'], False, a, None))
        else:
            kinds = [k for k in ('type', 'required', 'default') if a[k] != b[k]]
            if kinds:
                cosmetic = kinds == ['type'] and loose_type(a['type']) == loose_type(b['type'])
                rows.append((name, kinds, cosmetic, a, b))
    return rows


def marked_parameters(text):
    """Whether a function type's own parameter list has a rest (`...args`) or optional (`name?:`) parameter. Only the
    outermost parameter list counts: an object type's optional members (`{ base?: string }`) are not parameters."""
    arrow = top_level_arrow(text)
    if arrow < 0:
        return False
    own, depth, previous = [], 0, ''
    for ch in text[:arrow]:
        if ch in '([{<':
            depth += 1
        elif ch in ')]}' or (ch == '>' and previous != '='):
            depth -= 1
        elif depth == 1:
            own.append(ch)
        previous = ch
    own = ''.join(own)
    return '...' in own or re.search(r'\w\?:', own) is not None


def categorise(kinds, a, b):
    """A coarse name for a difference, to group the many alike: what was added or dropped, a changed requiredness or
    default, and for a type the kind of change (`parameter_marker_lost`: a rest or optional parameter the checker's text
    does not carry; `alias_expanded` / `alias_collapsed`: one side names a type the other writes out, judged by length)."""
    if kinds == ['only_checker'] or kinds == ['only_structural']:
        return kinds[0]
    if kinds != ['type']:
        return '+'.join(kinds)
    s, c = a['type'] or '', b['type'] or ''
    if marked_parameters(s) and not marked_parameters(c):
        return 'parameter_marker_lost'
    if len(c) - len(s) > 20 and len(c) > 1.5 * len(s):
        return 'alias_expanded'
    if len(s) - len(c) > 20 and len(s) > 1.5 * len(c):
        return 'alias_collapsed'
    return 'other_type'


def distinct_pairs(rows):
    """The differing props that matter (not `@types/react`, not spelling only) grouped by what differs: one entry per
    distinct (mode, structural facts, checker facts) with how many props show it, in which corpora, its category and one
    example. A property that Chakra's style system repeats across hundreds of components is one entry."""
    groups = {}
    for r in rows:
        if r['react_declared'] or r['cosmetic']:
            continue
        key = (r['mode'], json.dumps(r['structural'], sort_keys=True), json.dumps(r['checker'], sort_keys=True))
        g = groups.setdefault(key, {'mode': r['mode'], 'category': r['category'], 'kinds': r['kinds'], 'count': 0,
                                    'corpora': {}, 'structural': r['structural'], 'checker': r['checker'],
                                    'example': {k: r[k] for k in ('corpus', 'file', 'component', 'prop')}})
        g['count'] += 1
        g['corpora'][r['corpus']] = g['corpora'].get(r['corpus'], 0) + 1
    return sorted(groups.values(), key=lambda g: -g['count'])


def diff_runs(mode, corpus, structural, checker):
    """Every difference between two {(file, name): props} maps, as the rows of the diff file, and a summary."""
    rows, summary = [], {'components_compared': 0, 'components_differing': 0, 'props_compared': 0, 'props_differing': 0,
                         'cosmetic': 0, 'real': 0, 'react_declared': 0, 'only_structural': 0, 'only_checker': 0, 'type': 0, 'required': 0,
                         'default': 0, 'components_only_structural': 0, 'components_only_checker': 0, 'categories': {}}
    summary['components_only_structural'] = len(set(structural) - set(checker))
    summary['components_only_checker'] = len(set(checker) - set(structural))
    for key in sorted(set(structural) & set(checker)):
        summary['components_compared'] += 1
        summary['props_compared'] += len(set(structural[key]) | set(checker[key]))
        found = differing(structural[key], checker[key])
        summary['components_differing'] += bool(found)
        for name, kinds, cosmetic, a, b in found:
            react = bool((a or {}).get('react_declared') or (b or {}).get('react_declared'))
            summary['props_differing'] += 1
            summary['react_declared' if react else 'cosmetic' if cosmetic else 'real'] += 1
            for kind in kinds:
                if not react:
                    summary[kind] += 1
            category = categorise(kinds, a, b)
            if not react and not cosmetic:
                summary['categories'][category] = summary['categories'].get(category, 0) + 1
            rows.append({'mode': mode, 'corpus': corpus, 'file': key[0], 'component': key[1], 'prop': name,
                         'kinds': kinds, 'category': category, 'cosmetic': cosmetic, 'react_declared': react,
                         'structural': a, 'checker': b})
    return rows, summary


# --- running -------------------------------------------------------------------------------------------------------


def spread(values):
    return {'median': round(statistics.median(values)), 'min': round(min(values)), 'max': round(max(values))}


def timed(command, runs):
    """Wall ms and peak RSS MiB of `command` over `runs` runs (output discarded): median, min and max."""
    samples = [run_once(command) for _ in range(runs)]
    return {'ms': spread([s * 1000 for s, _ in samples]), 'rss_mib': spread([r for _, r in samples])}


def run_json(command, **kwargs):
    """The JSON a command prints to stdout (anything before the first brace, such as a banner, is skipped)."""
    out = subprocess.run(command, capture_output=True, text=True, **kwargs)
    return out.returncode, out.stdout[out.stdout.find('{'):] if '{' in out.stdout else '', out.stderr


def extract_command(binary, src, fmt, extra=()):
    return [binary, 'extract', '--src', src, '--format', fmt, *COMMON, *extra] if fmt != 'json' else \
        [binary, 'extract', '--src', src, '--json', *COMMON, *extra]


def checker_args(tsc):
    return ['--experimental-semantic-types', '--tsc-path', tsc]


def batch_backend(canonical):
    """From the canonical JSON of a checker run: how many components the checker resolved and how many kept the structural
    result, by the reason the diagnostics give (`SEMANTIC_BACKEND`)."""
    resolved, kept, reasons, warnings = 0, 0, {}, []
    for d in canonical.get('diagnostics', []):
        if d.get('code') != 'SEMANTIC_BACKEND':
            continue
        message = d.get('message', '')
        found = re.match(r'(\d+) component\(s\) resolved by the TypeScript checker', message)
        if found:
            resolved += int(found.group(1))
            continue
        found = re.match(r'(\d+) component\(s\) kept their structural result: (.*)', message)
        if found:
            kept += int(found.group(1))
            reasons[found.group(2)] = reasons.get(found.group(2), 0) + int(found.group(1))
            continue
        warnings.append(message[:200])
    return {'resolved': resolved, 'kept_structural': kept, 'reasons': reasons, 'other': warnings}


def program_inputs(name, src, out_dir):
    """The component files (every `.tsx` under the source directory) and a tsconfig rooted there, as the oracle wants."""
    files = []
    for current, dirs, names in os.walk(src):
        dirs[:] = [d for d in dirs if d != 'node_modules']
        files += [os.path.join(current, n) for n in names if n.endswith('.tsx')]
    list_file = os.path.join(out_dir, f'{name}.files.txt')
    with open(list_file, 'w') as f:
        f.write('\n'.join(sorted(files)) + '\n')
    tsconfig = os.path.join(out_dir, f'{name}.tsconfig.json')
    with open(tsconfig, 'w') as f:
        json.dump({'include': [f'{src}/**/*.ts', f'{src}/**/*.tsx'],
                   'compilerOptions': {'target': 'ES2022', 'module': 'ESNext', 'moduleResolution': 'bundler',
                                       'jsx': 'react-jsx', 'strict': True, 'skipLibCheck': True, 'noEmit': True}}, f)
    return list_file, tsconfig


def oracle_verdicts(binary, corpus, src, root, work, label, extra):
    """The checker oracle's verdicts on one mode's canonical JSON, as information: shared props, the share the oracle
    calls identical or spelled differently, and the share it calls wrong."""
    canonical = os.path.join(work, f"{corpus['name']}.{label}.json")
    with open(canonical, 'w') as f:
        subprocess.run([binary, 'extract', '--src', src, '--json', '--html-attributes', 'full', '--include-unexported',
                        '--load-dependencies', '--quiet', *extra], stdout=f, check=True)
    list_file, tsconfig = program_inputs(f"{corpus['name']}.{label}", src, work)
    verdicts = os.path.join(work, f"{corpus['name']}.{label}.oracle.json")
    subprocess.run(['node', ORACLE, '--root', root, '--src', src, '--tsconfig', tsconfig, '--caliper', canonical, '--out',
                    verdicts, '--files', list_file], check=True, timeout=3600, cwd=os.path.join(ROOT, 'apps', 'oracle'))
    with open(verdicts) as f:
        e = json.load(f)
    shared = e['props_shared'] or 1
    v = e['verdicts']
    return {'shared_props': e['props_shared'], 'agree': round((v['identical'] + v['spelled']) / shared, 4),
            'less_precise': round(v.get('less_precise', 0) / shared, 4), 'wrong': round(v['wrong'] / shared, 4)}


def session_run(node, src, mode, wait, tsc, count, work, label, runs):
    """One session mode: timings of its steps (from the first run), wall time and RSS over `runs` runs, and the parsed
    result of the first run."""
    command = [node, SESSION_SCRIPT, '--src', src, '--count', str(count), '--semantic', mode]
    if mode == 'on-demand':
        command += ['--wait', str(wait), '--tsc', tsc]
    out = os.path.join(work, f'{label}.session.json')
    samples = []
    first = None
    for i in range(runs):
        with open(out, 'w') as f:
            seconds, rss = run_once(command, stdout=f)
        samples.append((seconds, rss))
        if i == 0:
            with open(out) as f:
                first = json.load(f)
    return {'wall_ms': spread([s * 1000 for s, _ in samples]), 'rss_mib': spread([r for _, r in samples])}, first


def first_load(result):
    """The first-load figures of a session run: start, the first component, and the first 10 and 50."""
    results = result['results']

    def at(n):
        return round(results[min(n, len(results)) - 1]['cumulative_ms']) if results else None

    return {'session_start_ms': round(result['create_ms'] + result['index_ms']), 'components_listed':
            result['components_listed'], 'first_component_ms': at(1), 'first_10_ms': at(10), 'first_50_ms': at(50),
            'median_request_ms': round(statistics.median([r['ms'] for r in results]), 1) if results else None,
            'p90_request_ms': round(sorted(r['ms'] for r in results)[int(len(results) * 0.9) - 1], 1) if results else None}


def on_demand_summary(result):
    """How the checker did in an on-demand run: the share of first answers that fell back to the structural result, and
    the share still structural when the same components were asked again after the late answers had arrived."""
    def share(rows):
        counts = {}
        for r in rows:
            counts[r['backend']['outcome']] = counts.get(r['backend']['outcome'], 0) + 1
        total = sum(counts.values()) or 1
        return {'counts': counts, 'checker_share': round(counts.get('checker', 0) / total, 3),
                'structural_share': round(1 - counts.get('checker', 0) / total, 3),
                'timed_out': counts.get('timed_out', 0), 'timed_out_share': round(counts.get('timed_out', 0) / total, 3)}

    first = [r for r in result['results'] if r['present']]
    second = [r for r in result['second_pass'] if r['props'] is not None]
    return {'first_pass': share(first), 'second_pass': share(second), 'tsc_rss_mib': result['tsc_rss_mib'],
            'first_answer_ms': first[0]['ms'] if first else None}


def session_props(result, key='results'):
    return {(r['file'], r['display_name']): r['props'] for r in result[key] if r['props'] is not None and r['file']}


def relative_session(props, root):
    return {(os.path.relpath(os.path.realpath(f), os.path.realpath(root)) if f else '', n): v
            for (f, n), v in props.items()}


# --- the fallbacks -------------------------------------------------------------------------------------------------


def prove_fallbacks(binary, node, corpus, root, src, work):
    """Run the checker modes against a tsc that is missing and one that exits at once, on a small corpus, and show that
    each gives the structural result plus a warning and never an error."""
    fake = os.path.join(work, 'fake-typescript')
    os.makedirs(os.path.join(fake, 'bin'), exist_ok=True)
    with open(os.path.join(fake, 'package.json'), 'w') as f:
        json.dump({'name': 'typescript', 'version': '7.1.0-beta'}, f)
    crash = os.path.join(fake, 'bin', 'tsc')
    with open(crash, 'w') as f:
        f.write('#!/bin/sh\nexit 1\n')
    os.chmod(crash, 0o755)
    missing = os.path.join(work, 'nonexistent', 'bin', 'tsc')
    structural_code, structural_rdt, _ = run_json(extract_command(binary, src, 'rdt'))
    proofs = {}
    for label, tsc in (('missing_tsc', missing), ('crashing_tsc', crash)):
        code, rdt, _ = run_json(extract_command(binary, src, 'rdt', checker_args(tsc)))
        code_json, canonical, _ = run_json(extract_command(binary, src, 'json', checker_args(tsc)))
        warnings = []
        if canonical:
            warnings = [d.get('message', '')[:160] for d in json.loads(canonical).get('diagnostics', [])
                        if d.get('code') == 'SEMANTIC_BACKEND']
        proofs[label] = {'cli_exit_code': code, 'cli_output_equals_structural': rdt == structural_rdt,
                         'cli_warnings': warnings}
        _, result = session_run(node, src, 'on-demand', 150, tsc, 8, work, f'fallback-{label}', 1)
        _, structural_result = session_run(node, src, 'off', None, None, 8, work, f'fallback-{label}-off', 1)
        summary = on_demand_summary(result)
        proofs[label].update({
            'session_outcomes': summary['first_pass']['counts'],
            'session_props_equal_structural': relative_session(session_props(result), root) ==
            relative_session(session_props(structural_result), root)})
    return proofs


# --- the report ----------------------------------------------------------------------------------------------------


def corpus_report(args, corpus, work, diff_rows):
    base = os.path.abspath(args.corpora)
    root = os.path.join(base, corpus['clone'])
    src = os.path.join(root, corpus['src'])
    if not os.path.isdir(src):
        sys.exit(f"{corpus['name']}: not installed under {base}; run `scripts/corpora.py setup` first")
    name = corpus['name']
    report = {'corpus': name, 'load_before': os.getloadavg()[0]}
    say = lambda what: print(f'{name}: {what}', file=sys.stderr, flush=True)

    say('structural')
    structural_cmd = extract_command(args.bin, src, 'rdt')
    report['structural'] = timed(structural_cmd, args.runs)
    _, structural_text, _ = run_json(structural_cmd)
    structural_props = by_component(json.loads(structural_text), root)
    report['structural']['components'] = len(structural_props)

    if not args.skip_batch:
        say('batch checker')
        batch_cmd = extract_command(args.bin, src, 'rdt', checker_args(args.tsc))
        report['batch'] = timed(batch_cmd, args.runs)
        _, batch_text, _ = run_json(batch_cmd)
        batch_props = by_component(json.loads(batch_text), root)
        rows, summary = diff_runs('batch', name, structural_props, batch_props)
        diff_rows += rows
        report['batch']['diff'] = summary
        _, canonical, _ = run_json(extract_command(args.bin, src, 'json', checker_args(args.tsc)))
        report['batch']['backend'] = batch_backend(json.loads(canonical))

    if not args.skip_session:
        say('session, checker off')
        off_stats, off = session_run(args.node, src, 'off', None, None, args.count, work, f'{name}-off', args.runs)
        report['session_off'] = {**off_stats, **first_load(off)}
        off_props = relative_session(session_props(off), root)
        for wait in WAITS:
            say(f'session, on-demand, wait {wait} ms')
            stats, result = session_run(args.node, src, 'on-demand', wait, args.tsc, args.count, work,
                                        f'{name}-on-demand-{wait}', args.runs if wait == WAITS[0] else 1)
            entry = {**stats, **first_load(result), **on_demand_summary(result)}
            late = relative_session(session_props(result, 'second_pass'), root)
            rows, summary = diff_runs(f'on-demand-{wait}', name, off_props, late)
            diff_rows += rows
            entry['diff_after_late_answers'] = summary
            report[f'session_on_demand_{wait}'] = entry

    if not args.skip_oracle:
        say('oracle (information only: the checker modes are judged by the checker itself)')
        report['oracle'] = {'structural': oracle_verdicts(args.bin, corpus, src, root, work, 'structural', ()),
                            'batch': oracle_verdicts(args.bin, corpus, src, root, work, 'batch',
                                                     checker_args(args.tsc))}
    report['load_after'] = os.getloadavg()[0]
    return report


def sentences(report):
    """The report as plain lines for a terminal or a summary, no tables."""
    out = []
    for c in report['corpora']:
        s = c['structural']
        out.append(f"{c['corpus']}: structural {s['ms']['median']} ms (min {s['ms']['min']}, max {s['ms']['max']}), peak "
                   f"{s['rss_mib']['max']} MiB, {s['components']} components; load {c['load_before']:.1f} to "
                   f"{c['load_after']:.1f}")
        if 'batch' in c:
            b = c['batch']
            d = b['diff']
            out.append(f"  batch checker: {b['ms']['median']} ms, peak {b['rss_mib']['max']} MiB (largest process); "
                       f"checker resolved {b['backend']['resolved']} components, kept structural for "
                       f"{b['backend']['kept_structural']}; {d['props_differing']} of {d['props_compared']} props differ "
                       f"({d['react_declared']} are @types/react props, {d['cosmetic']} only in spelling, {d['real']} "
                       f"otherwise: {d['type']} type, {d['required']} "
                       f"required, {d['default']} default, {d['only_structural']} only structural, {d['only_checker']} "
                       f"only checker)")
        if 'session_off' in c:
            o = c['session_off']
            out.append(f"  first load, checker off: session start {o['session_start_ms']} ms, first component "
                       f"{o['first_component_ms']} ms, 10 components {o['first_10_ms']} ms, 50 components "
                       f"{o['first_50_ms']} ms, whole process peak {o['rss_mib']['max']} MiB")
        for wait in WAITS:
            e = c.get(f'session_on_demand_{wait}')
            if e:
                out.append(f"  on-demand, wait {wait} ms: first answer {e['first_answer_ms']} ms, 10 components "
                           f"{e['first_10_ms']} ms, 50 components {e['first_50_ms']} ms; checker answered "
                           f"{e['first_pass']['checker_share'] * 100:.0f}% of first requests (timed out: "
                           f"{e['first_pass']['timed_out']}; the rest kept the structural result for the checker's own "
                           f"reasons) and {e['second_pass']['checker_share'] * 100:.0f}% when asked again; tsc peak "
                           f"{e['tsc_rss_mib']} MiB; {e['diff_after_late_answers']['props_differing']} props differ from "
                           f"the structural session")
        if 'oracle' in c:
            s, b = c['oracle']['structural'], c['oracle']['batch']
            out.append(f"  oracle (the checker judging itself, information only): structural agree "
                       f"{s['agree'] * 100:.1f}% wrong {s['wrong'] * 100:.2f}%; batch checker agree {b['agree'] * 100:.1f}% "
                       f"wrong {b['wrong'] * 100:.2f}%")
    return '\n'.join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True, help='the release caliper-docgen binary')
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--corpus', action='append', help='a corpus name; repeat for several (default all)')
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--count', type=int, default=50, help='components a session asks for (0 = all)')
    parser.add_argument('--tsc', default=DEFAULT_TSC, help='the typescript@7.1 `bin/tsc` the checker runs')
    parser.add_argument('--node', default='node')
    parser.add_argument('--out', help='write the full report here as JSON')
    parser.add_argument('--diff-out', default=os.path.join(tempfile.gettempdir(), 'checker_modes_diff.json'),
                        help='the differing props, one row each (without @types/react and spelling-only rows)')
    parser.add_argument('--pairs-out', default=os.path.join(tempfile.gettempdir(), 'checker_modes_pairs.json'),
                        help='the differences grouped: one entry per distinct (structural, checker) pair, most common first')
    parser.add_argument('--diff-all', action='store_true', help='keep @types/react and spelling-only rows in --diff-out')
    parser.add_argument('--skip-oracle', action='store_true')
    parser.add_argument('--skip-batch', action='store_true')
    parser.add_argument('--skip-session', action='store_true')
    parser.add_argument('--fallbacks', action='store_true', help='also prove the missing and crashing tsc fallbacks')
    args = parser.parse_args()
    args.bin = os.path.abspath(args.bin)
    wanted = [c for c in CORPORA if not args.corpus or c['name'] in args.corpus]
    diff_rows, reports = [], []
    with tempfile.TemporaryDirectory() as work:
        for corpus in wanted:
            reports.append(corpus_report(args, corpus, work, diff_rows))
        proofs = None
        if args.fallbacks:
            small = next((c for c in wanted if c['name'] == 'antd'), wanted[0])
            root = os.path.join(os.path.abspath(args.corpora), small['clone'])
            proofs = prove_fallbacks(args.bin, args.node, small, root, os.path.join(root, small['src']), work)
    report = {'bin': args.bin, 'tsc': args.tsc, 'runs': args.runs, 'count': args.count, 'corpora': reports,
              'fallbacks': proofs}
    kept_rows = diff_rows if args.diff_all else [r for r in diff_rows if not r['react_declared'] and not r['cosmetic']]
    with open(args.diff_out, 'w') as f:
        json.dump(kept_rows, f)
    pairs = distinct_pairs(diff_rows)
    with open(args.pairs_out, 'w') as f:
        json.dump(pairs, f, indent=1)
    if args.out:
        with open(args.out, 'w') as f:
            json.dump(report, f, indent=1)
    print(sentences(report))
    if proofs:
        print('fallbacks: ' + json.dumps(proofs))
    print(f'{len(kept_rows)} of {len(diff_rows)} differing props written to {args.diff_out} (rows that are '
          f'{"all kept" if args.diff_all else "not @types/react and not spelling only"}); {len(pairs)} distinct '
          f'differences written to {args.pairs_out}')


if __name__ == '__main__':
    main()
