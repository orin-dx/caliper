#!/usr/bin/env python3
"""The weekly record: Caliper, react-docgen-typescript (RDT) and react-docgen on the five pinned corpora.

For each corpus it records, per tool, the wall time and peak RSS of one whole run (process start included), and for
Caliper the canonical JSON's size. Recall is measured against RDT: of the components RDT reports with props, how many
the tool also reports (same file, same name), and of their props, how many it lists, split into the props `@types/react`
declares and the rest. react-docgen follows imports to the file that declares a type but infers nothing, and lists none
of React's props. With --oracle it also records how often TypeScript 7.1's checker contradicts each tool's prop types,
over every prop the checker finds, and the headline of scripts/headline.py: how many of the props of the components Caliper
reports are right. With --agent-eval it also asks a fixed list of questions about real components and scores what a
reader of Caliper's output answers against the checker (scripts/agent_eval.py).

--previous FILE compares with an earlier record (the last green run's artifact) and writes --issue FILE for the workflow
to open or update one issue with, when a wrong type is above the gate or rises, a reader's question is answered
silent-wrong, or a whole run is over a budget (scripts/headline.py). Drift in time, memory and recall against RDT is
listed in the summary, not alarmed on. Nothing here fails a pull request.

Usage: scripts/weekly.py --bin BIN [--corpus NAME]... [--corpora DIR] [--work DIR] [--runs N] [--oracle] [--agent-eval]
                         [--previous FILE] [--baseline-failed] [--out FILE] [--markdown FILE] [--issue FILE]
"""
import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile

import agent_eval
import corpora
import headline
import oracle_data
import schema_version
from corpora import ROOT, run_once, selected

BASELINE_FAILED = "The baseline download failed, so this run was not compared with last week's record."
SLOWER = 1.10
RECALL_DROP = 0.02
# The rules that decide what the reference is: the exclusions and the pairing. Raise it when a change to them moves recall,
# so that a record is never compared on recall with one made under other rules (a record without it is version 1).
REFERENCE_VERSION = 2
# The universe of the oracle's scores (`oracle.tools`): version 2 counts candidate components only, naming a default export
# by its declaration; a record without it is version 1, which counted every function the checker takes for a component
# and named a default export `default`. Version 3 pairs a checker component with Caliper's by file and export name
# (ADR 0027) where version 2 paired by a normalised display name. Version 4 scores a prop whose checker type the oracle did
# not evaluate as `not judged` where version 3 scored it identical, and classes a declaration's path by one rule
# (apps/oracle/src/path-classes.json). Raise it when a change moves what the oracle counts.
UNIVERSE_VERSION = 4


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
    """Median wall ms and largest peak RSS MiB of `extract --json` in the Storybook addon's configuration (dependencies
    loaded, inherited props not collapsed), and the JSON's size. Every run here is the structural resolver's
    (`--no-checker`): the record and its budgets predate the checker default (docs/measurement.md)."""
    command = [binary, 'extract', '--src', src, '--json', '--load-dependencies', '--collapse-inherited-props', 'none',
               '--no-checker']
    with tempfile.TemporaryFile() as out:
        run_once(command, stdout=out)
        size = out.tell()
    samples = [run_once(command) for _ in range(runs)]
    return {'ms': round(statistics.median(s for s, _ in samples) * 1000), 'rss_mib': round(max(r for _, r in samples)),
            'json_mb': round(size / 2**20, 2)}


def caliper_components(binary, src, root):
    """Caliper's components in RDT's format, as (file, name) -> prop names."""
    command = [binary, 'extract', '--src', src, '--format', 'rdt', '--html-attributes', 'full', '--include-unexported',
               '--quiet', '--load-dependencies', '--collapse-inherited-props', 'none', '--no-checker']
    out = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    return {(relative(c['filePath'], root), norm_name(c['displayName'])): set(c['props'])
            for c in json.loads(out[out.index('{'):]).values()}


def caliper_layer_members(binary, src, root):
    """The props Caliper's collapsed inheritance layers hold, as (file, name) -> prop names: the output names them in a
    layer instead of listing them as props, and the canonical JSON is the one that carries the layers."""
    command = [binary, 'extract', '--src', src, '--json', '--html-attributes', 'full', '--include-unexported', '--quiet',
               '--load-dependencies', '--collapse-inherited-props', 'none', '--no-checker']
    out = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    document = schema_version.check(json.loads(out[out.index('{'):]), 'weekly')
    lists = document.get('nameLists', {})
    members = {}
    for c in document['components'].values():
        names = set()
        for layer in c.get('inheritance', []):
            if layer.get('members'):
                names |= set(lists[layer['members']])
        members[(relative(c['filePath'], root), norm_name(c['displayName']))] = names
    return members


# The shape rules of `exclusion`, shared with the oracle's universe (apps/oracle/src/component-shape.ts): the patterns are in
# one file both read (scripts/oracle_data.py), and both run the cases of apps/oracle/tests/fixtures/component-shape-cases.json.
HOOK = re.compile(oracle_data.SHAPE['hook'])
PROMISE_TYPE = re.compile(oracle_data.SHAPE['promiseType'])
EXTENSIONS = ('', '.tsx', '.ts', '/index.tsx', '/index.ts')
# An identifier inside any number of calls (`memo(forwardRef(Foo))`, `React.memo(Foo)`, `Object.assign(Foo, {...})`),
# followed by what ends an argument or an expression: `, ) ; !`, a type assertion or the end of the line.
WRAPPED = r'(?:[\w.]+\s*(?:<[^>]*>)?\s*\(\s*)*(\w+)(?=\s*[,);!]|\s+as\b|\s+satisfies\b|\s*$)'


def declared_in_react(info):
    return oracle_data.is_react_types(info.get('parent') or '')


def exclusion(comp):
    """Why RDT's entry is not a component, from what RDT reports about it, or None. RDT lists every function it can
    give a parameter to, so hooks, route handlers, helpers and context objects come with it. The evidence is the shape:
    props that are the members of a TypeScript lib type (`String`, `Function`, `Request`) mean the parameter is not an
    object of props; a hook is named `use` and a capital; a `value` declared by `@types/react` and nothing else is a
    context's Provider; a lowercase name cannot be a JSX tag unless it takes the attributes of one (a `@types/react`
    prop) or is the file's or directory's own name, which RDT gives a default export; and props that are all promises
    are handed over by a router, not by a parent. The oracle's universe applies the same rules to the checker's functions
    (apps/oracle/src/component-shape.ts)."""
    props = comp['props']
    in_lib = sum(oracle_data.is_lib_file(i.get('parent') or '') for i in props.values())
    if in_lib * 2 > len(props):
        return 'lib_members'
    name = comp['name']
    if HOOK.search(name):
        return 'hook'
    if 'value' in props and set(props) <= set(oracle_data.SHAPE['contextValueProps']) and all(map(declared_in_react, props.values())):
        return 'context_value'
    if name[:1].islower() and norm_name(name) not in own_names(comp['file']) and not any(map(declared_in_react, props.values())):
        return 'not_jsx'
    if all(PROMISE_TYPE.search(i.get('type', '')) for i in props.values()):
        return 'promise_props'
    return None


def read_text(path):
    """A source file's text, or None if there is no such file."""
    if not os.path.isfile(path):
        return None
    with open(path, errors='replace') as f:
        return f.read()


def import_binding(text, name):
    """Where `name` was imported from in `text`: (module, imported name) or None."""
    for match in re.finditer(r"^import\s+(?:type\s+)?([^;'\"]*?)\s+from\s+['\"]([^'\"]+)['\"]", text, re.M):
        clause, module = match.groups()
        if not clause.startswith('{'):
            default, _, clause = clause.partition(',')
            if default.strip() == name:
                return module, 'default'
        for original, alias in re.findall(r'(\w+)(?:\s+as\s+(\w+))?', clause.partition('{')[2].partition('}')[0]):
            if (alias or original) == name:
                return module, original
    return None


def export_source(text, name):
    """How `text` exports `name` (`default` or a name): ('local', local name), ('from', module, name) or None."""
    if name == 'default':
        # The second pattern unwraps calls whose first argument is an identifier (`memo(Foo)`, `observer(forwardRef(Foo))`,
        # `Object.assign(Foo, {...})`), and a trailing assertion (`Foo as T`, `Foo satisfies T`, `Foo!`): the wrapper is
        # whatever function the file calls, so the name that matters is the innermost identifier.
        for pattern in (r'^export\s+default\s+(?:async\s+)?(?:function\*?|class)\s+(\w+)',
                        rf'^export\s+default\s+{WRAPPED}'):
            match = re.search(pattern, text, re.M)
            if match:
                return 'local', match.group(1)
    else:
        declaration = rf'^export\s+(?:declare\s+)?(?:const|let|var|function\*?|class|async\s+function)\s+{re.escape(name)}\b'
        if re.search(declaration, text, re.M):
            return 'local', name
    for match in re.finditer(r"^export\s*\{([^}]*)\}\s*(?:from\s*['\"]([^'\"]+)['\"])?", text, re.M):
        for original, alias in re.findall(r'(\w+)(?:\s+as\s+(\w+))?', match.group(1)):
            if (alias or original) == name:
                return ('from', match.group(2), original) if match.group(2) else ('local', original)
    return None


def resolve_module(file, module, exists):
    """The file `module` names from `file` (relative paths only), as a path relative to the root."""
    if not module.startswith('.'):
        return None
    base = os.path.normpath(os.path.join(os.path.dirname(file), module))
    return next((base + ext for ext in EXTENSIONS if exists(base + ext)), None)


def export_target(file, read, exists, name='default', hops=6):
    """The file and local name where what `file` exports as `name` is defined: follows `export default X`,
    `import X from`, `export { X } from` and `export { default as X }` through re-exports. None where it ends elsewhere."""
    text = read(file)
    exported = export_source(text, name) if text and hops else None
    if not exported:
        return None
    if exported[0] == 'from':
        target = resolve_module(file, exported[1], exists)
        return export_target(target, read, exists, exported[2], hops - 1) if target else None
    chain = local_chain(text, exported[1])
    for local in chain:
        imported = import_binding(text, local)
        if imported:
            target = resolve_module(file, imported[0], exists)
            return export_target(target, read, exists, imported[1], hops - 1) if target else None
    return file, exported[1]


def local_chain(text, name, hops=4):
    """`name` and what a chain of `const name = Other` / `const name = memo(Other)` in `text` leads to, nearest first:
    the component is declared under the last, and Caliper may report it under any of them."""
    names = [name]
    while text and len(names) <= hops:
        match = re.search(rf'^(?:export\s+)?(?:const|let|var)\s+{re.escape(names[-1])}\b(?:\s*:[^=\n]+)?\s*=\s*{WRAPPED}',
                          text, re.M)
        if not match or match.group(1) in names:
            break
        names.append(match.group(1))
    return names


def display_assignments(text):
    """The `Foo.displayName = 'Bar'` assignments in `text`, as (local name, shown name) pairs."""
    return re.findall(r"^\s*(\w+)\.displayName\s*=\s*['\"]([^'\"]+)['\"]", text or '', re.M)


def own_names(file):
    """The names RDT gives a default export of `file`: the file's own, and its directory's."""
    return {norm_name(os.path.splitext(os.path.basename(file))[0]), norm_name(os.path.basename(os.path.dirname(file)))}


def counterpart(file, name, found_by_file, read=None, claimed=()):
    """Caliper's key in `file` for a component called `name`, or None: the names must agree, since a file's only
    component says nothing about whether RDT's entry is that component. With `read`, the names a local alias chain
    leads to count too, and the `displayName` each is given; a key in `claimed` (one an RDT entry already matches
    exactly) is never the answer."""
    text = read(file) if read else None
    names = local_chain(text, name) if read else [name]
    # Caliper names a component by its `displayName` where there is one, RDT by the declaration.
    shown = dict(display_assignments(text))
    for candidate in [n for local in names for n in (local, shown.get(local)) if n]:
        key = (file, norm_name(candidate))
        if norm_name(candidate) in found_by_file.get(file, {}) and key not in claimed:
            return key
    return None


def pair(comp, root, found_by_file, read, exists, claimed=()):
    """Caliper's key and the evidence for RDT's entry that Caliper does not report under its own key. In another file:
    where the default export of RDT's file is defined (RDT names a default export after the file or directory) or, for a
    named export, where that is. In the same file: the component that default export is, when the RDT name is the file's
    or directory's own and no other entry already matches it, or the one whose `displayName` RDT reports. Else the file
    that declares its props, when Caliper reports a component of that name there. A local alias chain
    (`const A = memo(B); export default A`) leads to the declaration. None if no evidence leads to a component."""
    file = relative(comp['file'], root)
    for export in ('default', comp['name']):
        target = export_target(file, read, exists, export)
        if not target:
            continue
        if target[0] != file:
            found = counterpart(target[0], target[1], found_by_file, read)
            if found:
                return found, 'default_export' if export == 'default' else 'named_export'
        elif export == 'default' and norm_name(comp['name']) in own_names(file):
            found = counterpart(file, target[1], found_by_file, read, claimed)
            if found:
                return found, 'default_alias'
    local = next((l for l, shown in display_assignments(read(file)) if norm_name(shown) == norm_name(comp['name'])), None)
    found = counterpart(file, local, found_by_file, read, claimed) if local else None
    if found:
        return found, 'display_name'
    name = norm_name(comp['name'])
    for info in comp['props'].values():
        parent = info.get('parent') or ''
        declared = relative(parent, root) if parent and not oracle_data.is_react_types(parent) else None
        if declared and declared != file and name in found_by_file.get(declared, {}):
            return (declared, name), 'declaration'
    return None


LIST_CAP = 15


def from_dependency(text, name):
    """Whether `text` imports `name` from a package (any form of import), not from a relative path."""
    for match in re.finditer(r"^import\s+(?:type\s+)?([^;'\"]*?)\s+from\s+['\"]([^'\"]+)['\"]", text, re.M):
        if not match.group(2).startswith('.') and re.search(rf'\b{re.escape(name)}\b', match.group(1)):
            return True
    return False


def reaches_dependency(file, read, exists, name, hops=6):
    """Whether what `file` exports as `name` ends in a package: `export { X } from 'pkg'`, or an import from one."""
    text = read(file)
    exported = export_source(text, name) if text and hops else None
    if not exported:
        return False
    if exported[0] == 'from':
        target = resolve_module(file, exported[1], exists)
        return reaches_dependency(target, read, exists, exported[2], hops - 1) if target else not exported[1].startswith('.')
    for local in local_chain(text, exported[1]):
        imported = import_binding(text, local)
        if imported:
            target = resolve_module(file, imported[0], exists)
            return reaches_dependency(target, read, exists, imported[1], hops - 1) if target else not imported[0].startswith('.')
    return False


def unmatched_cause(name, file, found_by_file, read, exists):
    """Why Caliper reports none of an item's names, from the item's file: a member of another component
    (`Form.List = List`); a member of an object literal (`logo: () => ...`); an alias of something a package exports
    (`const Tooltip = Recharts.Tooltip`); a value a call builds (`createIcon(...)`, `generateCalendar(...)`); a name the
    file exports from a package; else a component the file declares that Caliper does not report, in a file where it
    reports others or none. RDT names a default export after the file, so for that name the declaration is the one the
    default export leads to. The causes are what the file shows, so a cause says where to look, not what is wrong."""
    text = read(file) or ''
    names = [name.split('.')[-1]]
    default = norm_name(name) in own_names(file)
    exported = export_source(text, 'default') if default else None
    if exported and exported[0] == 'local':
        names += local_chain(text, exported[1])
    pattern = '(?:' + '|'.join(map(re.escape, dict.fromkeys(names))) + ')'
    declaration = rf'^(?:export\s+)?(?:const|let|var)\s+{pattern}\b(?:\s*:[^=\n]+)?\s*=\s*'
    if re.search(rf'\b\w+\.{pattern}\s*=[^=]', text):
        return 'compound_member'
    if re.search(rf'^\s+{pattern}\s*:\s*(?:\(|<|function\b|async\b|[\w$]+\s*=>|(?:React\.)?(?:memo|forwardRef)\b)', text, re.M):
        return 'object_member'
    alias = re.search(declaration + r'(\w+)(?:\.\w+)*\s*;?\s*$', text, re.M)
    if alias and from_dependency(text, alias.group(1)):
        return 'dependency'
    if re.search(declaration + r'[\w.]+\s*(?:<[^>]*>)?\(', text, re.M):
        return 'call_result'
    if reaches_dependency(file, read, exists, name) or (default and reaches_dependency(file, read, exists, 'default')):
        return 'dependency'
    return 'file_with_other_components' if found_by_file.get(file) else 'file_without_components'


def reference(rdt_components, found, root):
    """The components recall is measured on, from RDT's output: entries with props, minus those that are not components
    (`exclusion`), with every RDT entry for one component counted once. An entry that Caliper reports under another file
    pairs with it by evidence (`pair`) and joins the entry in that file; RDT's repeats of one key join too. Each item is
    {'keys': every (file, name) a tool may report it under, 'props': prop -> 'own' | 'react'}, so one list serves every
    tool. Also the counts, so a change in the reference shows in weekly.json, and the items Caliper reports nowhere, by
    cause (`unmatched_cause`): a count per cause and the first `LIST_CAP` of each, as `file: name`."""
    found_by_file = {}
    for file, name in found:
        found_by_file.setdefault(file, {})[name] = None
    exists = lambda rel: os.path.isfile(os.path.join(root, rel))
    read = lambda rel: read_text(os.path.join(root, rel))
    stats = {'rdt_with_props': 0, 'excluded': {}, 'exact': 0, 'paired': {}, 'merged': 0}
    entries = []
    for comp in rdt_components:
        if not comp['props']:
            continue
        stats['rdt_with_props'] += 1
        why = exclusion(comp)
        if why:
            stats['excluded'][why] = stats['excluded'].get(why, 0) + 1
            continue
        entries.append((comp, (relative(comp['file'], root), norm_name(comp['name']))))
    # Caliper's keys that an RDT entry matches exactly: another entry in that file is never paired with them.
    claimed = {key for _, key in entries if key in found}
    groups = {}
    for comp, key in entries:
        target = key if key in found else None
        if target:
            stats['exact'] += 1
        else:
            paired = pair(comp, root, found_by_file, read, exists, claimed)
            if paired:
                target = paired[0]
                stats['paired'][paired[1]] = stats['paired'].get(paired[1], 0) + 1
        keys = [key] + ([target] if target else [])
        group = groups.get(target or key)
        if group is None:
            group = groups[target or key] = {'keys': [], 'exact': {}, 'other': {}, 'file': key[0], 'name': comp['name']}
        else:
            stats['merged'] += 1
        group['keys'] += keys
        # The props of the entries in the component's own file when there are any, else of all the repeats united, so the
        # reference does not depend on the order RDT lists them in. A prop any repeat takes from `@types/react` is one.
        into = group['exact' if key == target else 'other']
        for prop, info in comp['props'].items():
            into[prop] = 'react' if into.get(prop) == 'react' or declared_in_react(info) else 'own'
    items = [{'keys': list(dict.fromkeys(g['keys'])), 'props': g['exact'] or g['other']} for g in groups.values()]
    stats['reference'] = len(items)
    missed = [g for g, item in zip(groups.values(), items) if not any(k in found for k in item['keys'])]
    stats['unpaired'] = len(missed)
    stats['unmatched'] = {'by_cause': {}, 'listed': {}}
    for g in sorted(missed, key=lambda g: (g['file'], g['name'])):
        cause = unmatched_cause(g['name'], g['file'], found_by_file, read, exists)
        stats['unmatched']['by_cause'][cause] = stats['unmatched']['by_cause'].get(cause, 0) + 1
        listed = stats['unmatched']['listed'].setdefault(cause, [])
        if len(listed) < LIST_CAP:
            listed.append(f"{g['file']}: {g['name']}")
    stats['excluded_total'] = sum(stats['excluded'].values())
    return items, stats


def recall(items, found, layered=None):
    """Against the reference: the components the tool also reports and, of those, RDT's props it lists, apart from and
    with `@types/react`'s, so a missing component and a missing prop are counted once each. `own` and `react` count the
    props listed as props; `own_layers` and `react_layers` also count those held in a collapsed layer (`layered`), which
    the output names without listing them."""
    layered = layered or {}
    count = {'components': [0, 0], 'own': [0, 0], 'react': [0, 0], 'own_layers': [0, 0], 'react_layers': [0, 0]}
    for item in items:
        count['components'][1] += 1
        key = next((k for k in item['keys'] if k in found), None)
        if key is None:
            continue
        in_layers = layered.get(key, set())
        count['components'][0] += 1
        for prop, kind in item['props'].items():
            count[kind][1] += 1
            count[kind][0] += prop in found[key]
            count[f'{kind}_layers'][1] += 1
            count[f'{kind}_layers'][0] += prop in found[key] or prop in in_layers
    return count


def rate(pair):
    return pair[0] / pair[1] if pair[1] else 0.0


def oracle_tools(tools):
    """The oracle's score for each tool, without the examples that are there to review the adapters by."""
    return {tool: {key: value for key, value in score.items() if key != 'examples'} for tool, score in tools.items()}


def oracle(binary, name, src, root, work, rdt_out, rdg_out):
    """How often TypeScript 7.1's checker contradicts Caliper's prop types (apps/oracle/src/oracle.ts), and how RDT and
    react-docgen fare against it over the same props. `shared_props`, `agree` and `wrong` are Caliper's, over the props
    both list, and keep their meaning from week to week (a prop the checker side did not evaluate is in `shared_props`
    and counts toward neither `agree` nor `wrong`, as `verdicts['not judged']`); `tools` holds all three over every prop of the candidate
    components the checker finds (the functions `exclusion`'s shape rules take out are left out: `universe_excluded`), and
    `tools_all_functions` the same scores over every function the checker takes for a component, as `tools` was before
    `universe_version` 2."""
    caliper_json = os.path.join(work, f'{name}.caliper.json')
    with open(caliper_json, 'w') as f:
        subprocess.run([binary, 'extract', '--src', src, '--json', '--html-attributes', 'full', '--include-unexported',
                        '--load-dependencies', '--collapse-inherited-props', 'none', '--quiet', '--no-checker'],
                       stdout=f, check=True)
    list_file, tsconfig = program_inputs(name, src, work)
    verdicts = os.path.join(work, f'{name}.oracle.json')
    subprocess.run(['pnpm', '--dir', os.path.join(ROOT, 'apps', 'oracle'), 'exec', 'tsx', 'src/oracle.ts', '--root', root,
                    '--src', src, '--tsconfig', tsconfig, '--caliper', caliper_json, '--out', verdicts, '--files',
                    list_file, '--rdt', rdt_out, '--rdg', rdg_out], check=True, timeout=3600)
    with open(verdicts) as f:
        e = json.load(f)
    shared = e['props_shared']
    return {'shared_props': shared, 'agree': rate((sum(e['verdicts'][v] for v in oracle_data.AGREE), shared)),
            'wrong': rate((e['verdicts']['wrong'], shared)), 'universe_version': UNIVERSE_VERSION,
            'tools': oracle_tools(e['tools']), 'tools_all_functions': oracle_tools(e['tools_all_functions']),
            'universe_excluded': e['universe_excluded']}


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
    caliper_found = caliper_components(args.bin, src, root)
    items, counts = reference(rdt_components, caliper_found, root)
    entry = {
        'commit': corpus['commit'],
        'caliper': caliper_run(args.bin, src, args.runs),
        'rdt': rdt,
        'react_docgen': rdg,
        'reference': counts,
        'recall': {'caliper': recall(items, caliper_found, caliper_layer_members(args.bin, src, root)),
                   'react_docgen': recall(items, found)},
    }
    if args.oracle:
        try:
            entry['oracle'] = oracle(args.bin, corpus['name'], src, root, work, rdt_out, rdg_out)
            if h := headline.compute(entry['oracle']):
                entry['headline'] = h
        except (subprocess.SubprocessError, OSError, KeyError) as error:
            entry['oracle'] = {'error': str(error)}
    if args.agent_eval:
        try:
            result = agent_eval.evaluate(args.bin, corpus['name'], src, root, tsconfig, work)
            if result:
                entry['agent_eval'] = result
        except (subprocess.SubprocessError, OSError, KeyError, ValueError) as error:
            entry['agent_eval'] = {'error': str(error)}
    return entry


def same_reference(record, previous):
    """Whether `record` and `previous` were measured against references built by the same rules."""
    return record.get('reference_version', 1) == previous.get('reference_version', 1)


def same_universe(entry, old):
    """Whether two corpus entries counted the same props: the oracle's universe has had its rules changed (`UNIVERSE_VERSION`)
    when their `universe_version` differs. An entry without an oracle has no universe to differ."""
    now, before = entry.get('oracle') or {}, old.get('oracle') or {}
    if 'tools' not in now or 'tools' not in before:
        return True
    return now.get('universe_version', 1) == before.get('universe_version', 1)


def findings(record, previous):
    """What opens the issue: a wrong type above the gate or higher than last week, a silent-wrong answer to a reader's
    question, a whole run over a budget (scripts/headline.py). Drift against `previous` is `changes`, not a finding. A rise
    in wrong types is not read against a record whose universe was another (`same_universe`): the props counted differ."""
    out = []
    for name, entry in record['corpora'].items():
        old = previous.get('corpora', {}).get(name)
        if old and not same_universe(entry, old):
            old = {key: value for key, value in old.items() if key != 'headline'}
        out += headline.findings(name, entry, old) + agent_eval.findings(name, entry)
    return out


def changes(record, previous):
    """What moved since `previous`, for the summary and not for an issue: Caliper's time and memory by more than 10%,
    its recall against RDT by more than 2 points, which is not compared across references built by different rules, and the
    agent correct rate by more than 5 points."""
    out = []
    for name, entry in record['corpora'].items():
        old = previous.get('corpora', {}).get(name)
        if not old:
            continue
        if not same_universe(entry, old):
            out.append(f"{name}: the oracle's universe is version {entry['oracle'].get('universe_version', 1)} now and was version "
                       f"{old['oracle'].get('universe_version', 1)} last week, so its props, agreement and wrong types are "
                       "not compared with last week's")
        for key, label in (('ms', 'time'), ('rss_mib', 'peak memory')):
            now, before = headline.own_metric(entry, key), headline.own_metric(old, key)
            if now is not None and before is not None and now > SLOWER * before:
                out.append(f"{name}: {label} {before} -> {now}")
        for kind in ('components', 'own', 'react', 'own_layers', 'react_layers') if same_reference(record, previous) else ():
            if kind not in (old.get('recall') or {}).get('caliper', {}) or kind not in (entry.get('recall') or {}).get('caliper', {}):
                continue  # a record from before layers were counted
            now, before = rate(entry['recall']['caliper'][kind]), rate(old['recall']['caliper'][kind])
            if now < before - RECALL_DROP:
                out.append(f'{name}: recall of {kind} {100 * before:.1f}% -> {100 * now:.1f}%')
        out += agent_eval.changes(name, entry, old)
    return out


TOOL_LABELS = {'caliper': 'Caliper', 'rdt': 'RDT', 'react_docgen': 'react-docgen'}


def tool_line(corpus, tool, score):
    """One tool against the checker, in words: agreement of the whole universe first, then of what the tool lists, then
    what it omits by where the checker says the prop is declared."""
    s = score['missing_by_stratum']
    parts = [f"agrees on {100 * score['agree_universe']:.1f}% of the {score['universe']:,} props the checker finds "
             f"({100 * score['agree_listed']:.1f}% of the {score['listed']:,} it lists, "
             f"{100 * score['wrong_listed']:.2f}% of those wrong)",
             f"omits {sum(s.values()):,} ({s['same_file']:,} declared in the component's file, {s['other_project_file']:,} in "
             f"other project files, {s['dependency']:,} in dependencies, {s['unknown']:,} unknown)"]
    if score['missing_by_reason']['collapsed']:
        parts.append(f"{score['missing_by_reason']['collapsed']:,} more sit in collapsed layers")
    if score.get('failed_files'):
        parts.append(f"threw on {score['failed_files']:,} files, {score['missing_by_reason']['failed_file']:,} of the omitted props")
    return f"- {corpus}, {TOOL_LABELS[tool]}: " + '; '.join(parts)


SHAPE_LABELS = {'lib_members': 'a lib type', 'hook': 'a hook', 'context_value': "a context's value", 'not_jsx': 'lowercase',
                'promise_props': 'promises'}


def excluded_line(corpus, oracle_entry):
    """What the oracle's universe left out as not components, and what each tool's whole-universe agreement reads without
    that rule (`tools_all_functions`), so the figure before `universe_version` 2 stays in sight."""
    left = oracle_entry['universe_excluded']
    why = ', '.join(f"{v['props']:,} {SHAPE_LABELS[k]}" for k, v in left['by_reason'].items())
    before = oracle_entry.get('tools_all_functions') or {}
    reads = ', '.join(f"{TOOL_LABELS[t]} {100 * before[t]['agree_universe']:.1f}%"
                      for t in ('caliper', 'rdt', 'react_docgen') if t in before)
    return (f"- {corpus}: {left['components']:,} functions with {left['props']:,} props left out"
            + (f' ({why})' if why else '') + (f'; over every function: {reads}' if reads else ''))


def markdown(record, notes, reference_changed=None, moved=(), baseline=True, baseline_failed=False):
    lines = headline.markdown(record)
    lines += [f"Caliper {record['caliper']}, time the median of {record['runs']} runs (after one warm-up run) and memory the largest peak of them; RDT "
              'and react-docgen one cold run each, so the comparison favours Caliper. Whole process, Node start included.', '', '### Time and memory', '',
              '| Corpus | Caliper | JSON | RDT | react-docgen |', '| --- | --: | --: | --: | --: |']
    for name, e in record['corpora'].items():
        cell = lambda t: f"{t['ms'] / 1000:.2f} s, {t['rss_mib']:,} MiB"
        lines.append(f"| {name} | {cell(e['caliper'])} | {e['caliper']['json_mb']} MB | {cell(e['rdt'])} | "
                     f"{cell(e['react_docgen'])} |")
    lines += ['', '### Reference (RDT, cleaned)', '',
              '| Corpus | RDT entries with props | Not components | Repeats merged | Reference | Same file | Paired | Unpaired |',
              '| --- | --: | --: | --: | --: | --: | --: | --: |']
    for name, e in record['corpora'].items():
        r = e['reference']
        lines.append(f"| {name} | {r['rdt_with_props']} | {r['excluded_total']} | {r['merged']} | {r['reference']} | "
                     f"{r['exact']} | {sum(r['paired'].values())} | {r['unpaired']} |")
    lines += ['', '### Reference items Caliper reports nowhere, by cause', '', '| Corpus | Cause | Items |', '| --- | --- | --: |']
    for name, e in record['corpora'].items():
        for cause, count in sorted(e['reference'].get('unmatched', {}).get('by_cause', {}).items(), key=lambda c: -c[1]):
            lines.append(f'| {name} | {cause} | {count} |')
    lines += ['', '### Recall against the reference', '',
              "Recall is the share of RDT's reference components and props that Caliper also reports. It is not a measure of "
              "correctness: what RDT lists beyond Caliper is not necessarily right.", '',
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
        tool_rows = [(n, o['tools']) for n, o in oracle_rows if 'tools' in o]
        if tool_rows:
            lines += ['', "Each tool against the checker, over every own prop the checker finds on an exported candidate "
                          'component (a prop a tool omits counts against it). Hooks, lowercase helpers and functions whose '
                          "parameter is a lib type, a context's value or a router's promises are no candidates and are left "
                          'out, by the rules of the reference above:', '']
            lines += [tool_line(name, tool, tools[tool]) for name, tools in tool_rows
                      for tool in ('caliper', 'rdt', 'react_docgen') if tool in tools]
            lines += ['', 'Left out of that universe as not components, with the whole-universe agreement over every function '
                          'the checker takes for a component (the universe before these rules):', '']
            lines += [excluded_line(name, o) for name, o in oracle_rows if 'tools' in o and 'universe_excluded' in o]
    lines += agent_eval.markdown(record)
    if baseline_failed:
        lines += ['', BASELINE_FAILED + ' Nothing here was checked for getting worse.']
    elif not baseline:
        lines += ['', 'No baseline: there was no earlier record to compare with, so nothing here was checked for getting worse.']
    if reference_changed is not None:
        lines += ['', f"Recall was not compared with last week's: its reference was built by version {reference_changed} of "
                      f"the rules, this one by version {record['reference_version']}."]
    if notes:
        lines += ['', '### Needs attention (opens the issue)', ''] + [f'- {n}' for n in notes]
    if moved:
        lines += ['', '### Changes since last week (information)', ''] + [f'- {n}' for n in moved]
    return '\n'.join(lines) + '\n'


def load_previous(path):
    """The earlier record at `path`, `{}` when there is none to compare with. A file that is there but unreadable stops the
    run before anything is measured: a corrupt baseline is a broken artifact, not an absent one."""
    if not path or not os.path.exists(path):
        return {}
    try:
        with open(path) as f:
            previous = json.load(f)
    except (OSError, ValueError) as error:
        raise SystemExit(f'--previous {path} is not a readable record: {error}')
    if not isinstance(previous, dict) or not isinstance(previous.get('corpora'), dict):
        raise SystemExit(f'--previous {path} is not a weekly record: it has no "corpora" object')
    return previous


def errors(record):
    """What a corpus could not measure (the oracle or the agent evaluation threw), as issue lines."""
    return [f'{name}, {part.replace("_", " ")}: {entry[part]["error"][:200]}'
            for name, entry in record['corpora'].items() for part in ('oracle', 'agent_eval')
            if isinstance(entry.get(part), dict) and 'error' in entry[part]]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True)
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--work', default=os.path.join(ROOT, '.corpora', 'work'))
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--oracle', action='store_true')
    parser.add_argument('--agent-eval', action='store_true')
    parser.add_argument('--previous')
    parser.add_argument('--baseline-failed', action='store_true', help='the download of --previous failed: say so and open the issue')
    parser.add_argument('--out')
    parser.add_argument('--markdown')
    parser.add_argument('--issue')
    args = parser.parse_args()
    previous = load_previous(args.previous)
    work = os.path.abspath(args.work)
    os.makedirs(work, exist_ok=True)
    version = subprocess.run([args.bin, '--version'], capture_output=True, text=True).stdout.strip()
    record = {'caliper': version, 'runs': args.runs, 'reference_version': REFERENCE_VERSION,
              'corpora': {c['name']: measure(args, c, work) for c in selected(args.corpus)}}
    reference_changed = None
    if previous and not same_reference(record, previous):
        reference_changed = previous.get('reference_version', 1)
    notes = findings(record, previous) + errors(record)
    table = markdown(record, notes, reference_changed, changes(record, previous), baseline=bool(previous),
                     baseline_failed=args.baseline_failed)
    issue_notes = notes + ([BASELINE_FAILED] if args.baseline_failed else [])
    print(table)
    for path, content in ((args.out, json.dumps(record, indent=2) + '\n'), (args.markdown, table),
                          (args.issue, 'The weekly record needs attention:\n\n' + '\n'.join(f'- {n}' for n in issue_notes) + '\n'
                           if issue_notes else None)):
        if path and content:
            with open(path, 'w') as f:
                f.write(content)


if __name__ == '__main__':
    main()
