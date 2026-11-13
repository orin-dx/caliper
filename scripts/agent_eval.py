#!/usr/bin/env python3
"""Whether a reader of Caliper's output gets right answers, and says so when it cannot.

An agent that reads one component's TOON slice (`sessionComponentToon`, the table `extract --format toon` prints) acts on
what it reads. This evaluation asks a fixed list of questions (scripts/agent_questions.json) about real components in the
pinned corpora and scores each answer against TypeScript 7.1's checker, never against Caliper itself:

  correct       the answer is what the checker says
  abstained     the output carried a signal that it may be wrong or incomplete, and the reader said so
  silent-wrong  the answer is not what the checker says and nothing in the output hinted at it

Silent-wrong is the number that matters: a wrong answer with a signal is a degraded answer, a wrong answer without one is a
defect. No model is involved. The reader is a fixed policy (`answer`) that reads only the slice: the component's props
table, its inheritance rows and the diagnostics that say its props may be incomplete. It abstains when

  - a prop it reads has a `degradedBy` reason, or a warning or error names the component,
  - a type is cut short (`...(+N)`), is `opaque(...)`, or is a name it cannot read as one of the kinds it is asked for,
  - the prop it is asked about is not in the table and an inheritance row could be hiding it, or
  - the component is not in the output and a diagnostic names its file.

Nothing here fails a build: weekly.py reports the numbers and opens its issue when silent-wrong is above zero. A correct
rate that falls by more than five points is listed under the summary's changes and opens nothing.

Usage: scripts/agent_eval.py --bin BIN [--corpus NAME]... [--corpora DIR] [--work DIR]
"""
import argparse
import json
import os
import subprocess
import sys
import time

import corpora
import schema_version
from corpora import ROOT

QUESTIONS = os.path.join(ROOT, 'scripts', 'agent_questions.json')
KINDS = ('present', 'prop_exists', 'prop_required', 'literal_values', 'type_kind')
CORRECT_DROP = 0.05
EXAMPLES = 20
ABSTAIN = None


# ─── The slice a reader sees ─────────────────────────────────────────────────


def split_row(line):
    """One TOON row as its cells: a cell with a comma, quote or newline is in double quotes, a quote inside it escaped."""
    cells, cell, quoted, i = [], [], False, 0
    while i < len(line):
        ch = line[i]
        if quoted:
            if ch == '\\' and i + 1 < len(line):
                cell.append(line[i + 1])
                i += 1
            elif ch == '"':
                quoted = False
            else:
                cell.append(ch)
        elif ch == '"':
            quoted = True
        elif ch == ',':
            cells.append(''.join(cell))
            cell = []
        else:
            cell.append(ch)
        i += 1
    cells.append(''.join(cell))
    return cells


def row(line, columns):
    """`line` as a dict over `columns`; commas the renderer left unquoted in the last cell stay in it."""
    cells = split_row(line)
    if len(cells) > len(columns):
        cells = cells[:len(columns) - 1] + [','.join(cells[len(columns) - 1:])]
    return dict(zip(columns, cells + [''] * (len(columns) - len(cells))))


def columns_of(header):
    return header[header.index('{') + 1:header.index('}')].split(',')


class Slice:
    """One component as its TOON table shows it: props by name, inheritance rows, and the diagnostics a reader sees."""

    def __init__(self, name, file):
        self.name, self.file = name, file
        self.props, self.layers, self.diagnostics = {}, [], []

    def warned(self):
        return any(severity != 'info' for severity, _ in self.diagnostics)


def parse_toon(text):
    """The components in `extract --format toon` output, as (file, display name) -> Slice."""
    found, current, section, columns = {}, None, None, []
    for line in text.split('\n'):
        stripped = line.strip()
        if stripped.startswith('component:') and line.startswith('  component:'):
            name = stripped[len('component:'):]
            current = Slice(name[:-1] if name.endswith(':') else name, '')
            section = None
        elif current is None:
            continue
        elif line.startswith('    filePath: '):
            current.file = line[len('    filePath: '):].strip()
            found[(current.file, current.name)] = current
        elif line.startswith('    props['):
            section, columns = 'props', columns_of(line)
        elif line.startswith('    inheritance['):
            section, columns = 'inheritance', columns_of(line)
        elif line.startswith('      ') and section:
            cells = row(line.strip(), columns)
            if section == 'props':
                current.props[cells['name']] = {
                    'required': cells['required'] == 'true', 'type': cells['type'],
                    'degraded_by': [r for r in cells.get('degradedBy', '').split('|') if r]}
            else:
                current.layers.append({
                    'type_name': cells['typeName'], 'total': int(cells['totalProps'] or 0),
                    'members': [m for m in cells.get('members', '').split('|') if m]})
        elif not line.startswith('  '):
            current = None
    return found


# ─── Reading a type the way an agent does ────────────────────────────────────


def split_top(text, sep):
    """`text` split at `sep` outside brackets, quotes and arrows (`=>`)."""
    parts, cell, depth, quote, i = [], [], 0, False, 0
    while i < len(text):
        ch = text[i]
        if quote:
            cell.append(ch)
            if ch == '\\' and i + 1 < len(text):
                cell.append(text[i + 1])
                i += 1
            elif ch == "'":
                quote = False
        elif ch == "'":
            quote = True
            cell.append(ch)
        elif ch in '<([{':
            depth += 1
            cell.append(ch)
        elif ch == '>' and i > 0 and text[i - 1] == '=':
            cell.append(ch)
        elif ch in '>)]}':
            depth -= 1
            cell.append(ch)
        elif ch == sep and depth == 0:
            parts.append(''.join(cell))
            cell = []
        else:
            cell.append(ch)
        i += 1
    parts.append(''.join(cell))
    return parts


def has_arrow(text):
    return len(split_top(text.replace('=>', '\0'), '\0')) > 1


def quoted(member):
    return len(member) >= 2 and member[0] == "'" and member[-1] == "'"


def unquote(member):
    return member[1:-1].replace("\\'", "'").replace('\\\\', '\\')


def members_of(text):
    """The members of a union as written, without `undefined` and `null` (an optional prop's type has neither)."""
    return [m for m in split_top(text.strip(), '|') if m not in ('undefined', 'null')]


def kind_of(text):
    """The kind a reader would give a prop's type: boolean, string, number, array, function or literal_union, else None."""
    text = text.strip()
    if has_arrow(text):
        return 'function'
    members = members_of(text)
    if not members or any(m.startswith('...(+') for m in members):
        return None
    if len(members) == 1:
        only = members[0]
        if only in ('boolean', 'string', 'number'):
            return only
        if only.startswith(('Array<', 'ReadonlyArray<')):
            return 'array'
        return 'literal_union' if quoted(only) else None
    if all(quoted(m) for m in members):
        return 'literal_union'
    return 'boolean' if set(members) == {'true', 'false'} else None


def literals_of(text):
    """The string literals of a union, sorted, or None when the type is anything else or is cut short."""
    members = members_of(text)
    if not members or not all(quoted(m) for m in members):
        return None
    return sorted(unquote(m) for m in members)


# ─── The reader ──────────────────────────────────────────────────────────────


def answer(question, found, file_diagnostics):
    """What the reader says about `question`: a value, or ABSTAIN (None) when the output gives it a reason to."""
    kind = question['kind']
    entry = found.get((question['file'], question['component']))
    if kind == 'present':
        if entry:
            return True
        return ABSTAIN if file_diagnostics.get(question['file']) else False
    if not entry or entry.warned():
        return ABSTAIN
    name = question['prop']
    prop = entry.props.get(name)
    if kind == 'prop_exists':
        if prop or any(name in layer['members'] for layer in entry.layers):
            return True
        # A layer with props and no member list could be holding it.
        return ABSTAIN if any(layer['total'] and not layer['members'] for layer in entry.layers) else False
    if not prop or prop['degraded_by']:
        return ABSTAIN
    if kind == 'prop_required':
        return prop['required']
    if kind == 'literal_values':
        return literals_of(prop['type'])
    return kind_of(prop['type'])


def classify(given, truth):
    if given is ABSTAIN:
        return 'abstained'
    return 'correct' if given == truth else 'silent_wrong'


# ─── The checker's side ──────────────────────────────────────────────────────


def load_questions(path=QUESTIONS):
    with open(path) as f:
        return json.load(f)['questions']


def checker_truth(corpus, root, tsconfig, work, path=QUESTIONS):
    out = os.path.join(work, f'{corpus}.agent-truth.json')
    subprocess.run(['pnpm', '--dir', os.path.join(ROOT, 'apps', 'oracle'), 'exec', 'tsx', 'src/agent-truth.ts', '--root', root,
                    '--tsconfig', tsconfig, '--questions', path, '--corpus', corpus, '--out', out], check=True, timeout=1800)
    with open(out) as f:
        return json.load(f)


def relative(path, root):
    return os.path.relpath(os.path.realpath(path), os.path.realpath(root))


def caliper_views(binary, src, root):
    """Caliper's TOON components keyed by (file relative to `root`, name), the diagnostics that say a component's props may
    be incomplete attached to them, and every diagnostic by the file it names."""
    # The default's output (the checker answers), as the weekly record's other runs (docs/measurement.md).
    base = [binary, 'extract', '--src', src, '--html-attributes', 'full', '--quiet', '--load-dependencies']
    toon = subprocess.run(base + ['--format', 'toon'], capture_output=True, text=True, check=True).stdout
    raw = subprocess.run(base + ['--json'], capture_output=True, text=True, check=True).stdout
    document = schema_version.check(json.loads(raw[raw.index('{'):]), 'agent_eval')
    found = {(relative(file, root), name): entry for (file, name), entry in parse_toon(toon).items()}
    key_of = {(relative(c['filePath'], root), c['displayName']): key for key, c in document['components'].items()}
    by_key = {key: found[pair] for pair, key in key_of.items() if pair in found}
    lists, file_diagnostics = document.get('nameLists', {}), {}
    for diagnostic in document['diagnostics']:
        if diagnostic.get('file'):
            file_diagnostics.setdefault(relative(diagnostic['file'], root), []).append(diagnostic)
        if diagnostic['severity'] == 'info':
            continue
        for key in lists.get(diagnostic.get('affects'), []):
            if key in by_key:
                by_key[key].diagnostics.append((diagnostic['severity'], diagnostic['message']))
    return found, file_diagnostics


def evaluate(binary, corpus, src, root, tsconfig, work, path=QUESTIONS):
    """The record for one corpus, or None when no question is about it."""
    questions = [q for q in load_questions(path) if q['corpus'] == corpus]
    if not questions:
        return None
    start = time.monotonic()
    found, file_diagnostics = caliper_views(binary, src, root)
    truth = checker_truth(corpus, root, tsconfig, work, path)
    return score(questions, found, file_diagnostics, truth, round(1000 * (time.monotonic() - start)))


def score(questions, found, file_diagnostics, truth, ms=0):
    counts = {'correct': 0, 'abstained': 0, 'silent_wrong': 0}
    by_kind = {k: dict(counts) for k in KINDS}
    wrong, invalid, outcomes = [], [], {}
    for q in questions:
        fact = truth.get(q['id'], {'invalid': 'no answer from the checker'})
        if 'invalid' in fact:
            invalid.append({'id': q['id'], 'why': fact['invalid']})
            continue
        given = answer(q, found, file_diagnostics)
        outcome = classify(given, fact['value'])
        counts[outcome] += 1
        by_kind[q['kind']][outcome] += 1
        outcomes[q['id']] = outcome
        if outcome == 'silent_wrong' and len(wrong) < EXAMPLES:
            wrong.append({'id': q['id'], 'answer': given, 'checker': fact['value']})
    scored = sum(counts.values())
    return {'questions': scored, **counts, 'correct_rate': round(counts['correct'] / scored, 4) if scored else 0,
            'by_kind': by_kind, 'outcomes': outcomes, 'silent_wrong_items': wrong, 'invalid': invalid, 'ms': ms}


# ─── What weekly.py reports ──────────────────────────────────────────────────


def findings(name, entry):
    """What opens the issue: any silent-wrong answer. A fall in the correct rate is `changes`, which opens nothing."""
    now = entry.get('agent_eval')
    if not now or 'error' in now or not now['silent_wrong']:
        return []
    shown = ', '.join(f"{w['id']} (said {w['answer']!r}, checker {w['checker']!r})" for w in now['silent_wrong_items'][:5])
    return [f"{name}: {now['silent_wrong']} silent-wrong answer(s) of {now['questions']}: {shown}"]


def changes(name, entry, previous):
    """What moved since `previous` (a corpus entry of the last record): a correct rate more than five points lower."""
    now, old = entry.get('agent_eval'), (previous or {}).get('agent_eval')
    if not now or 'error' in now or not old or 'error' in old:
        return []
    if now['correct_rate'] < old['correct_rate'] - CORRECT_DROP:
        return [f"{name}: agent questions answered correctly {100 * old['correct_rate']:.0f}% -> {100 * now['correct_rate']:.0f}%"]
    return []


def markdown(record):
    """The section of the weekly summary for the evaluation, or no lines when it did not run."""
    rows = [(n, e['agent_eval']) for n, e in record['corpora'].items() if 'agent_eval' in e]
    if not rows:
        return []
    lines = ['', "### What a reader of Caliper's output gets right (checker is the truth)", '',
             '| Corpus | Questions | Correct | Abstained | Silent-wrong | Not scored |', '| --- | --: | --: | --: | --: | --: |']
    for name, e in rows:
        if 'error' in e:
            lines.append(f'| {name} | unavailable: {e["error"][:80]} | | | | |')
        else:
            lines.append(f"| {name} | {e['questions']} | {e['correct']} ({100 * e['correct_rate']:.0f}%) | {e['abstained']} | "
                         f"{e['silent_wrong']} | {len(e['invalid'])} |")
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', required=True)
    parser.add_argument('--corpora', default=corpora.DEFAULT_DIR)
    parser.add_argument('--corpus', action='append', default=[])
    parser.add_argument('--work', default=os.path.join(ROOT, '.corpora', 'work'))
    parser.add_argument('--questions', default=QUESTIONS)
    parser.add_argument('--out')
    args = parser.parse_args()
    work = os.path.abspath(args.work)
    os.makedirs(work, exist_ok=True)
    record = {'corpora': {}}
    asked = {q['corpus'] for q in load_questions(args.questions)}
    for corpus in (c for c in corpora.selected(args.corpus) if c['name'] in asked):
        root = os.path.join(os.path.abspath(args.corpora), corpus['clone'])
        src = corpora.ready(corpus, os.path.abspath(args.corpora), sparse=False)
        if not src:
            sys.exit(f"{corpus['name']}: not installed; run `scripts/corpora.py setup` first")
        tsconfig = os.path.join(work, f"{corpus['name']}.tsconfig.json")
        with open(tsconfig, 'w') as f:
            json.dump({'include': [f'{src}/**/*.ts', f'{src}/**/*.tsx'],
                       'compilerOptions': {'target': 'ES2022', 'module': 'ESNext', 'moduleResolution': 'bundler',
                                           'jsx': 'react-jsx', 'strict': True, 'skipLibCheck': True, 'noEmit': True}}, f)
        result = evaluate(args.bin, corpus['name'], src, root, tsconfig, work, args.questions)
        if result:
            record['corpora'][corpus['name']] = {'agent_eval': result}
    print('\n'.join(markdown(record)))
    if args.out:
        with open(args.out, 'w') as f:
            json.dump(record, f, indent=2)
            f.write('\n')


if __name__ == '__main__':
    main()
