#!/usr/bin/env python3
"""The weekly record: Caliper, react-docgen-typescript (RDT) and react-docgen on the five pinned corpora.

For each corpus it records, per tool, the wall time and peak RSS of one whole run (process start included), and for
Caliper the canonical JSON's size. Recall is measured against RDT: of the components RDT reports with props, how many
the tool also reports (same file, same name), and of their props, how many it lists, split into the props `@types/react`
declares and the rest. react-docgen reads each file alone, so it lists none of React's. With --oracle it also records
how often TypeScript 7.1's checker contradicts Caliper's prop types. That is information, never a gate.

--previous FILE compares with an earlier record (the last green run's artifact) and, when Caliper is more than 10%
slower or uses more than 10% more memory, or its recall falls by more than 2 points, writes --issue FILE for the
workflow to open or update one issue with. Nothing here fails a pull request.

Usage: scripts/weekly.py --bin BIN [--corpus NAME]... [--corpora DIR] [--work DIR] [--runs N] [--oracle]
                         [--previous FILE] [--out FILE] [--markdown FILE] [--issue FILE]
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
from corpora import ROOT, run_once, selected

SLOWER = 1.10
RECALL_DROP = 0.02


def norm_name(name):
    """A component name as the tools may spell it: no `(file)` suffix, namespace or punctuation."""
    name = re.sub(r' \(.*\)$', '', name).split('/')[-1].split('.')[-1]
    return re.sub(r'[^a-z0-9]', '', name.lower())


def relative(path, root):
    """`path` relative to `root`, both through symlinks: Caliper reports canonical paths, RDT the ones it was given."""
    return os.path.relpath(os.path.realpath(path), os.path.realpath(root))


def program_inputs(name, src, out_dir):
    """The component files (every `.tsx` under the source directory) and a tsconfig rooted there."""
    files = []
    for current, dirs, names in os.walk(src):
        dirs[:] = [d for d in dirs if d != 'node_modules']
        files += [os.path.join(current, n) for n in names if n.endswith('.tsx')]
    list_file = os.path.join(out_dir, f'{name}.files.txt')
    with open(list_file, 'w') as f:
        f.write('\n'.join(sorted(files)) + '\n')
    tsconfig = os.path.join(out_dir, f'{name}.tsconfig.json')
    with open(tsconfig, 'w') as f:
        json.dump({
            'include': [f'{src}/**/*.ts', f'{src}/**/*.tsx'],
            'compilerOptions': {'target': 'ES2022', 'module': 'ESNext', 'moduleResolution': 'bundler', 'jsx': 'react-jsx',
                                'strict': True, 'skipLibCheck': True, 'noEmit': True},
        }, f, indent=2)
    return list_file, tsconfig


def tool_run(command, out):
    """One whole run of a Node tool that writes its components to `out`: wall ms, peak RSS MiB and the components."""
    seconds, rss = run_once(command)
    with open(out) as f:
        return {'ms': round(seconds * 1000), 'rss_mib': round(rss)}, json.load(f)['components']


def caliper_run(binary, src, runs):
    """Median wall ms and peak RSS MiB of `extract --json` with dependency loading on, and the JSON's size."""
    command = [binary, 'extract', '--src', src, '--json', '--load-dependencies']
    with tempfile.TemporaryFile() as out:
        run_once(command, stdout=out)
        size = out.tell()
    samples = [run_once(command) for _ in range(runs)]
    return {'ms': round(statistics.median(s for s, _ in samples) * 1000), 'rss_mib': round(max(r for _, r in samples)),
            'json_mb': round(size / 2**20, 2)}


def caliper_components(binary, src, root):
    """Caliper's components in RDT's format, as (file, name) -> prop names."""
    command = [binary, 'extract', '--src', src, '--format', 'rdt', '--html-attributes', 'full', '--include-unexported',
               '--quiet', '--load-dependencies']
    out = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    return {(relative(c['filePath'], root), norm_name(c['displayName'])): set(c['props'])
            for c in json.loads(out[out.index('{'):]).values()}


def caliper_layer_members(binary, src, root):
    """The props Caliper's collapsed inheritance layers hold, as (file, name) -> prop names: the output names them in a
    layer instead of listing them as props (ADR 0012), and the canonical JSON is the one that carries the layers."""
    command = [binary, 'extract', '--src', src, '--json', '--html-attributes', 'full', '--include-unexported', '--quiet',
               '--load-dependencies']
    out = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    document = json.loads(out[out.index('{'):])
    lists = document.get('nameLists', {})
    members = {}
    for c in document['components'].values():
        names = set()
        for layer in c.get('inheritance', []):
            if layer.get('members'):
                names |= set(lists[layer['members']])
        members[(relative(c['filePath'], root), norm_name(c['displayName']))] = names
    return members


def recall(reference, found, root, layered=None):
    """Against RDT: the components the tool also reports and, of those, RDT's props it lists, apart from and with
    `@types/react`'s, so a missing component and a missing prop are counted once each. `own` and `react` count the props
    listed as props; `own_layers` and `react_layers` also count those held in a collapsed layer (`layered`), which the
    output names without listing them."""
    layered = layered or {}
    count = {'components': [0, 0], 'own': [0, 0], 'react': [0, 0], 'own_layers': [0, 0], 'react_layers': [0, 0]}
    for comp in reference:
        if not comp['props']:
            continue
        count['components'][1] += 1
        key = (relative(comp['file'], root), norm_name(comp['name']))
        props = found.get(key)
        if props is None:
            continue
        in_layers = layered.get(key, set())
        count['components'][0] += 1
        for prop, info in comp['props'].items():
            kind = 'react' if 'node_modules/@types/react/' in (info.get('parent') or '') else 'own'
            count[kind][1] += 1
            count[kind][0] += prop in props
            count[f'{kind}_layers'][1] += 1
            count[f'{kind}_layers'][0] += prop in props or prop in in_layers
    return count


def rate(pair):
    return pair[0] / pair[1] if pair[1] else 0.0


def oracle(binary, name, src, root, work):
    """How often TypeScript 7.1's checker contradicts Caliper's prop types (apps/oracle/src/oracle.ts)."""
    caliper_json = os.path.join(work, f'{name}.caliper.json')
    with open(caliper_json, 'w') as f:
        subprocess.run([binary, 'extract', '--src', src, '--json', '--html-attributes', 'full', '--include-unexported',
                        '--load-dependencies', '--quiet'], stdout=f, check=True)
    list_file, tsconfig = program_inputs(name, src, work)
    verdicts = os.path.join(work, f'{name}.oracle.json')
    subprocess.run(['pnpm', '--dir', os.path.join(ROOT, 'apps', 'oracle'), 'exec', 'tsx', 'src/oracle.ts', '--root', root,
                    '--src', src, '--tsconfig', tsconfig, '--caliper', caliper_json, '--out', verdicts, '--files',
                    list_file], check=True, timeout=3600)
    with open(verdicts) as f:
        e = json.load(f)
    shared = e['props_shared']
    return {'shared_props': shared, 'agree': rate((e['verdicts']['identical'] + e['verdicts']['spelled'], shared)),
            'wrong': rate((e['verdicts']['wrong'], shared))}


def measure(args, corpus, work):
    root = os.path.join(os.path.abspath(args.corpora), corpus['clone'])
    src = corpora.ready(corpus, os.path.abspath(args.corpora), sparse=False)
    if not src:
        sys.exit(f"{corpus['name']}: not installed; run `scripts/corpora.py setup` first")
    list_file, tsconfig = program_inputs(corpus['name'], src, work)
    print(f"{corpus['name']}: RDT", file=sys.stderr, flush=True)
    rdt_out = os.path.join(work, f"{corpus['name']}.rdt.json")
    rdt, rdt_components = tool_run(['node', '--max-old-space-size=8192', os.path.join(ROOT, 'scripts', 'rdt.mjs'),
                                    list_file, tsconfig, rdt_out], rdt_out)
    print(f"{corpus['name']}: react-docgen", file=sys.stderr, flush=True)
    rdg_out = os.path.join(work, f"{corpus['name']}.rdg.json")
    rdg, rdg_components = tool_run(['node', os.path.join(ROOT, 'scripts', 'rdg.mjs'), list_file, rdg_out], rdg_out)
    print(f"{corpus['name']}: Caliper", file=sys.stderr, flush=True)
    found = {(relative(c['file'], root), norm_name(c['name'])): set(c['props']) for c in rdg_components}
    entry = {
        'commit': corpus['commit'],
        'caliper': caliper_run(args.bin, src, args.runs),
        'rdt': rdt,
        'react_docgen': rdg,
        'recall': {'caliper': recall(rdt_components, caliper_components(args.bin, src, root), root,
                                     caliper_layer_members(args.bin, src, root)),
                   'react_docgen': recall(rdt_components, found, root)},
    }
    if args.oracle:
        try:
            entry['oracle'] = oracle(args.bin, corpus['name'], src, root, work)
        except (subprocess.SubprocessError, OSError, KeyError) as error:
            entry['oracle'] = {'error': str(error)}
    return entry


def findings(record, previous):
    """What got worse since `previous`: Caliper's time and memory by more than 10%, its recall by more than 2 points."""
    out = []
    for name, entry in record['corpora'].items():
        old = previous.get('corpora', {}).get(name)
        if not old:
            continue
        for key, label in (('ms', 'time'), ('rss_mib', 'peak memory')):
            if entry['caliper'][key] > SLOWER * old['caliper'][key]:
                out.append(f"{name}: {label} {old['caliper'][key]} -> {entry['caliper'][key]}")
        for kind in ('components', 'own', 'react', 'own_layers', 'react_layers'):
            if kind not in old['recall']['caliper']:
                continue  # a record from before layers were counted
            now, before = rate(entry['recall']['caliper'][kind]), rate(old['recall']['caliper'][kind])
            if now < before - RECALL_DROP:
                out.append(f'{name}: recall of {kind} {100 * before:.1f}% -> {100 * now:.1f}%')
    return out


def markdown(record, notes):
    lines = [f"Caliper {record['caliper']}, median of {record['runs']} runs; RDT and react-docgen one run each. Whole "
             'process, Node start included.', '', '### Time and memory', '',
             '| Corpus | Caliper | JSON | RDT | react-docgen |', '| --- | --: | --: | --: | --: |']
    for name, e in record['corpora'].items():
        cell = lambda t: f"{t['ms'] / 1000:.2f} s, {t['rss_mib']:,} MiB"
        lines.append(f"| {name} | {cell(e['caliper'])} | {e['caliper']['json_mb']} MB | {cell(e['rdt'])} | "
                     f"{cell(e['react_docgen'])} |")
    lines += ['', '### Recall against RDT', '',
              '| Corpus | Tool | Components | Own props | Own, with layers | `@types/react` props | `@types/react`, with layers |',
              '| --- | --- | --: | --: | --: | --: | --: |']
    for name, e in record['corpora'].items():
        for tool in ('caliper', 'react_docgen'):
            r = e['recall'][tool]
            # A tool that writes no layers (react-docgen) has the same count with and without them.
            kinds = ('components', 'own', 'own_layers', 'react', 'react_layers')
            lines.append(f'| {name} | {tool} | ' + ' | '.join(f'{100 * rate(r.get(k, r[k.removesuffix("_layers")])):.1f}%'
                                                              for k in kinds) + ' |')
    oracle_rows = [(n, e['oracle']) for n, e in record['corpora'].items() if 'oracle' in e]
    if oracle_rows:
        lines += ['', "### TypeScript 7.1's checker (information)", '', '| Corpus | Shared props | Agree | Wrong |',
                  '| --- | --: | --: | --: |']
        for name, o in oracle_rows:
            lines.append(f'| {name} | unavailable: {o["error"][:80]} | | |' if 'error' in o else
                         f"| {name} | {o['shared_props']:,} | {100 * o['agree']:.1f}% | {100 * o['wrong']:.2f}% |")
    if notes:
        lines += ['', '### Worse than last week', ''] + [f'- {n}' for n in notes]
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True)
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--work', default=os.path.join(ROOT, '.corpora', 'work'))
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--oracle', action='store_true')
    parser.add_argument('--previous')
    parser.add_argument('--out')
    parser.add_argument('--markdown')
    parser.add_argument('--issue')
    args = parser.parse_args()
    work = os.path.abspath(args.work)
    os.makedirs(work, exist_ok=True)
    version = subprocess.run([args.bin, '--version'], capture_output=True, text=True).stdout.strip()
    record = {'caliper': version, 'runs': args.runs,
              'corpora': {c['name']: measure(args, c, work) for c in selected(args.corpus)}}
    notes = []
    if args.previous and os.path.exists(args.previous):
        with open(args.previous) as f:
            notes = findings(record, json.load(f))
    table = markdown(record, notes)
    print(table)
    for path, content in ((args.out, json.dumps(record, indent=2) + '\n'), (args.markdown, table),
                          (args.issue, 'The weekly record got worse:\n\n' + '\n'.join(f'- {n}' for n in notes) + '\n'
                           if notes else None)):
        if path and content:
            with open(path, 'w') as f:
                f.write(content)


if __name__ == '__main__':
    main()
