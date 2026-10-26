#!/usr/bin/env python3
"""Which signals Caliper already emits predict that TypeScript 7.1's checker disagrees with a prop's type?

Reads the files `scripts/parity.py oracle --work DIR` leaves (`NAME.caliper.json`, `NAME.oracle-rows.json`,
`NAME.oracle-props.json`) and, for every shared prop, joins the oracle's verdict with the signals in the canonical JSON.
It then scores three ways of choosing which components to send to a checker, each judged leave-one-corpus-out:

  rule    today's: the component has a warning or error diagnostic (the Storybook addon's fallback rule)
  table   a smoothed risk table over coarse signal combinations, backing off to coarser ones
  model   logistic regression on the same signals

A component is routed when it ranks in the top part of the budget; a disagreement is caught when its component is
routed. Labels: `soft` is a verdict of `wrong` or `less precise`, `wrong` alone is rarer and reported separately.
Rows the checker has no type for (`checker any/error`) are not a disagreement either way and are left out.

    scripts/confidence.py --work DIR [--out report.md] [--json signals.json]
"""
import argparse
import json
import math
import os
import sys
from collections import defaultdict

BUDGETS = [0.02, 0.05, 0.10, 0.20]
PROP_BUDGETS = [0.002, 0.005, 0.01, 0.02, 0.05, 0.10]
TERMINAL_KINDS = {'reactNode', 'cssProperties', 'elementType', 'htmlAttributes', 'ref', 'sxProps'}
CODES = ['UNRESOLVABLE_IMPORT', 'OPAQUE_TYPE', 'INDEXED_ACCESS_OPAQUE', 'TEMPLATE_LITERAL_OPAQUE',
         'GENERIC_ARGUMENT_MISMATCH', 'UNKNOWN']


def bucket(n, edges):
    for i, edge in enumerate(edges):
        if n <= edge:
            return i
    return len(edges)


def shape(t):
    """(depth, named references, opaque nodes, widest union, has function) of one prop type."""
    depth = named = opaque = widest = 0
    fn = False

    def walk(x, level):
        nonlocal depth, named, opaque, widest, fn
        if isinstance(x, list):
            for v in x:
                walk(v, level)
        elif isinstance(x, dict):
            kind = x.get('kind')
            if kind:
                depth = max(depth, level)
                named += kind == 'named'
                opaque += kind == 'opaque'
                fn = fn or kind == 'function'
                if kind in ('union', 'literalUnion'):
                    widest = max(widest, len(x.get('members', [])))
            for v in x.values():
                walk(v, level + (1 if kind else 0))

    walk(t, 0)
    return depth, named, opaque, widest, fn


def load(work, name):
    base = os.path.join(work, name)
    with open(base + '.caliper.json') as f:
        caliper = json.load(f)
    with open(base + '.oracle-rows.json') as f:
        rows = json.load(f)
    dump = []
    if os.path.exists(base + '.oracle-props.json'):
        with open(base + '.oracle-props.json') as f:
            dump = json.load(f)
    with open(base + '.oracle-missing.json') as f:
        missing = json.load(f)
    with open(base + '.oracle.json') as f:
        summary = json.load(f)
    return caliper, rows, dump, missing, summary


def package_of(path):
    if '/node_modules/' not in path:
        return '(project)'
    parts = path.rsplit('/node_modules/', 1)[1].split('/')
    return '/'.join(parts[:2]) if parts[0].startswith('@') and len(parts) > 1 else parts[0]


def component_diagnostics(caliper):
    """component key -> (worst severity rank, warn/error count, set of codes among warnings and errors)."""
    lists = caliper.get('nameLists', {})
    info, warn, codes = defaultdict(int), defaultdict(int), defaultdict(set)
    for d in caliper['diagnostics']:
        affects = d.get('affects')
        keys = lists.get(affects, []) if isinstance(affects, str) else (affects or [])
        for key in keys:
            if d['severity'] == 'info':
                info[key] += 1
            else:
                warn[key] += 1
                codes[key].add(d.get('code') or 'OTHER')
    return info, warn, codes


def signals(caliper, rows, dump):
    """One dict per labelled row: the discrete signals and the label."""
    info, warn, codes = component_diagnostics(caliper)
    # The autocomplete idiom `'a' | 'b' | (string & {})` prints in Caliper as literals plus `string`. The oracle drops
    # the covered literals from Caliper's side only (oracle-compare.ts `members`), so it compares `string` with the
    # checker's literals and says `less precise` though both list the same values. Not a disagreement for our purpose.
    def idiom_shaped(t):
        kinds = [m.get('kind') for m in t.get('members', [])] if isinstance(t, dict) and t.get('kind') == 'union' else []
        return 'string' in kinds and any(k in ('stringLiteral', 'numberLiteral') for k in kinds)

    idiom = {(d['key'], d['prop']) for d in dump if d['verdict'] == 'less precise' and idiom_shaped(d['caliper'])}
    props = caliper['propSets'] if 'propSets' in caliper else None
    out = []
    comp_cache = {}
    for key, prop, verdict in rows:
        if verdict in ('checker any/error', 'missing', 'collapsed'):
            continue
        comp = caliper['components'].get(key)
        if comp is None:
            continue
        if key not in comp_cache:
            ids = comp['props']
            merged = {}
            for pid in ids:
                merged.update(props[pid] if props else {})
            comp_cache[key] = merged if props else comp['props']
        pid = comp_cache[key].get(prop)
        if pid is None:
            continue
        p = caliper['props'][pid]
        t = p['type']
        depth, named, opaque, widest, fn = shape(t)
        decl = (p.get('declarations') or [{}])[0].get('fileName', '') or ''
        parent = p.get('parent')
        source = 'dep' if '/node_modules/' in decl else 'project'
        out.append({
            'component': key,
            'prop': prop,
            'verdict': verdict,
            'soft': verdict in ('wrong', 'less precise'),
            'real': verdict == 'wrong' or (verdict == 'less precise' and (key, prop) not in idiom),
            'hard': verdict == 'wrong',
            # prop signals
            'kind': t.get('kind', '?'),
            'terminal': t.get('kind') in TERMINAL_KINDS,
            'depth': bucket(depth, [1, 2, 3]),
            'named': bucket(named, [0, 1, 3]),
            'opaque': bucket(opaque, [0, 1]),
            'union': bucket(widest, [0, 3, 9]),
            'fn': fn,
            'inherited': parent is not None,
            'source': source,
            'required': bool(p.get('required')),
            'default': p.get('defaultValue') is not None,
            # component signals
            'rule': comp.get('detectedBy', '?'),
            'composes': bool(comp.get('composes')),
            'layers': bucket(len(comp.get('inheritance') or []), [0, 1, 2]),
            'size': bucket(len(comp_cache[key]), [3, 10, 30]),
            'warn': warn[key] > 0,
            'nwarn': warn[key],
            'info': info[key] > 0,
            **{f'code_{c}': c in codes[key] for c in CODES},
        })
    return out


def component_signals(caliper, rows):
    """Signals available before a checker run, with the count of checker props Caliper omitted as the label."""
    info, warn, codes = component_diagnostics(caliper)
    counts = defaultdict(lambda: [0, 0, 0])
    for key, _, verdict in rows:
        counts[key][0] += 1
        counts[key][1] += verdict == 'missing'
        counts[key][2] += verdict == 'collapsed'
    out = []
    for key, (checker_props, missing, collapsed) in counts.items():
        comp = caliper['components'].get(key)
        if comp is None:
            continue
        ids = comp['props']
        prop_count = len(set().union(*(caliper['propSets'][pid] for pid in ids))) if isinstance(ids, list) else len(ids)
        out.append({
            'component': key,
            'missing': missing,
            'collapsed': collapsed,
            'has_missing': missing > 0,
            'checker_props': checker_props,
            'rule': comp.get('detectedBy', '?'),
            'size': bucket(prop_count, [0, 3, 10, 30, 100]),
            'layers': bucket(len(comp.get('inheritance') or []), [0, 1, 2]),
            'composes': bool(comp.get('composes')),
            'warn': warn[key] > 0,
            'nwarn': warn[key],
            'info': info[key] > 0,
            **{f'code_{c}': c in codes[key] for c in CODES},
        })
    return out


# ─── models ─────────────────────────────────────────────────────────────────────

COARSE = [('kind', 'terminal', 'depth', 'named', 'opaque', 'union', 'fn', 'inherited', 'source', 'rule', 'warn'),
          ('kind', 'named', 'opaque', 'inherited', 'source', 'rule', 'warn'),
          ('kind', 'opaque', 'source', 'warn'),
          ('warn',), ()]
LR_FEATURES = ['kind', 'terminal', 'depth', 'named', 'opaque', 'union', 'fn', 'inherited', 'source', 'required',
               'default', 'rule', 'composes', 'layers', 'size', 'warn', 'info'] + [f'code_{c}' for c in CODES]
PRESENCE_COARSE = [('rule', 'size', 'layers', 'composes', 'warn'), ('rule', 'size', 'warn'),
                   ('rule', 'warn'), ('warn',), ()]
PRESENCE_FEATURES = ['rule', 'size', 'layers', 'composes', 'warn', 'info'] + [f'code_{c}' for c in CODES]


def table_fit(rows, weights, label, coarse=COARSE):
    """Hierarchical smoothed rate: each level's rate is its own counts pulled toward the next coarser level's."""
    counts = [defaultdict(lambda: [0.0, 0.0]) for _ in coarse]
    for r, w in zip(rows, weights):
        for level, feats in enumerate(coarse):
            c = counts[level][tuple(r[f] for f in feats)]
            c[0] += w * r[label]
            c[1] += w
    return counts


def table_rate(counts, r, coarse=COARSE, strength=20.0):
    rate = None
    for level in range(len(coarse) - 1, -1, -1):
        pos, tot = counts[level].get(tuple(r[f] for f in coarse[level]), (0.0, 0.0))
        rate = (pos + strength * (rate if rate is not None else 0.01)) / (tot + strength) if rate is not None else \
            (pos + strength * 0.01) / (tot + strength)
    return rate


def lr_encode(rows):
    index = {}
    encoded = []
    for r in rows:
        active = []
        for f in LR_FEATURES:
            v = (f, r[f])
            if v not in index:
                index[v] = len(index) + 1
            active.append(index[v])
        encoded.append(active)
    return index, encoded


def lr_fit(rows, weights, label, features=LR_FEATURES, epochs=150, l2=1e-3, lr=0.5):
    """Weighted logistic regression on one-hot signals, full-batch gradient descent over distinct signatures."""
    groups = defaultdict(lambda: [0.0, 0.0])
    for r, w in zip(rows, weights):
        sig = tuple(r[f] for f in features)
        groups[sig][0] += w * r[label]
        groups[sig][1] += w
    index, _ = lr_encode([])
    sigs = list(groups)
    active = []
    for sig in sigs:
        a = []
        for f, v in zip(features, sig):
            a.append(index.setdefault((f, v), len(index) + 1))
        active.append(a)
    n = len(index) + 1
    w = [0.0] * n
    total = sum(g[1] for g in groups.values())
    prior = sum(g[0] for g in groups.values()) / total
    w[0] = math.log(prior / (1 - prior))
    m = [0.0] * n
    for _ in range(epochs):
        grad = [0.0] * n
        for sig, a in zip(sigs, active):
            pos, tot = groups[sig]
            z = w[0] + sum(w[i] for i in a)
            p = 1 / (1 + math.exp(-z))
            e = tot * p - pos
            grad[0] += e
            for i in a:
                grad[i] += e
        for i in range(n):
            g = grad[i] / total + (l2 * w[i] if i else 0)
            m[i] = 0.9 * m[i] + g
            w[i] -= lr * m[i]
    return index, w


def lr_rate(model, r, features=LR_FEATURES):
    index, w = model
    z = w[0] + sum(w[index[(f, r[f])]] for f in features if (f, r[f]) in index)
    return 1 / (1 + math.exp(-z))


# ─── evaluation ─────────────────────────────────────────────────────────────────

def caught_at(rows, score, budgets, label):
    """For each budget: the share of positive rows whose component ranks inside the routed top fraction."""
    by_comp = defaultdict(list)
    for r in rows:
        by_comp[r['component']].append(r)
    # Expected disagreements first (the sum of the props' risks), then the riskiest single prop.
    ranked = sorted(by_comp, key=lambda c: (-sum(score(r) for r in by_comp[c]), -max(score(r) for r in by_comp[c]), c))
    positives = sum(r[label] for r in rows)
    result = {}
    for b in budgets:
        k = max(1, round(b * len(ranked)))
        routed = set(ranked[:k])
        got = sum(r[label] for c in routed for r in by_comp[c])
        result[b] = got / positives if positives else float('nan')
    return result, len(ranked), positives


def caught_props_at(rows, score, budgets, label):
    """The same, routing individual props: ask the checker about the top fraction of props by risk."""
    ranked = sorted(rows, key=lambda r: (-score(r), r['component'], r['prop']))
    positives = sum(r[label] for r in rows)
    out = {}
    for b in budgets:
        k = max(1, round(b * len(ranked)))
        out[b] = sum(r[label] for r in ranked[:k]) / positives if positives else float('nan')
    return out


def routed_set(rows, score, budget):
    by_comp = defaultdict(list)
    for r in rows:
        by_comp[r['component']].append(r)
    ranked = sorted(by_comp, key=lambda c: (-sum(score(r) for r in by_comp[c]), -max(score(r) for r in by_comp[c]), c))
    return set(ranked[:max(1, round(budget * len(ranked)))])


def natural_rule_point(rows, label):
    comps = defaultdict(list)
    for r in rows:
        comps[r['component']].append(r)
    routed = [c for c, rs in comps.items() if rs[0]['warn']]
    positives = sum(r[label] for r in rows)
    got = sum(r[label] for c in routed for r in comps[c])
    return len(routed) / len(comps), (got / positives if positives else float('nan'))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--work', required=True)
    ap.add_argument('--out')
    ap.add_argument('--json')
    args = ap.parse_args()
    names = sorted({f.split('.')[0] for f in os.listdir(args.work) if f.endswith('.oracle-rows.json')})
    data, presence, dumps, missing_details, summaries = {}, {}, {}, {}, {}
    for name in names:
        caliper, rows, dump, missing, summary = load(args.work, name)
        data[name] = signals(caliper, rows, dump)
        presence[name] = component_signals(caliper, rows)
        dumps[name] = dump
        missing_details[name] = missing
        summaries[name] = summary
        print(f'{name}: {len(data[name])} labelled props, {sum(r["soft"] for r in data[name])} soft, '
              f'{sum(r["real"] for r in data[name])} real, {sum(r["hard"] for r in data[name])} wrong', file=sys.stderr)
    evals = [n for n in names if n != 'fixtures'] + (['fixtures'] if 'fixtures' in names else [])
    lines = ['# Confidence score: what predicts an oracle disagreement', '']
    lines.append('| Corpus | Labelled props | Soft (wrong or less precise) | Real (soft minus idiom) | Wrong | Components |')
    lines.append('| --- | --: | --: | --: | --: | --: |')
    for n in evals:
        rs = data[n]
        lines.append(f'| {n} | {len(rs)} | {sum(r["soft"] for r in rs)} | {sum(r["real"] for r in rs)} | {sum(r["hard"] for r in rs)} | '
                     f'{len({r["component"] for r in rs})} |')
    lines += ['', '## Missing props in matched components', '',
              'A checker-only prop has no Caliper prop type to score. These labels train a separate component-level '
              'route using only signals available before the checker runs. They are not all extraction defects: '
              'Caliper intentionally summarizes large inherited sources, and the checker also includes ambient '
              'module augmentations. Layers are excluded by matching declaration files, which can still misclassify '
              'a prop when several types share a file. Components the checker finds but Caliper does not match are '
              'outside this analysis.', '',
              '| Corpus | Matched / checker components | Components missing props | Missing props | Props summarized in layers |',
              '| --- | --: | --: | --: | --: |']
    for n in evals:
        rs = presence[n]
        lines.append(f'| {n} | {summaries[n]["components_matched"]} / {summaries[n]["components_checker"]} | '
                     f'{sum(r["has_missing"] for r in rs)} | '
                     f'{sum(r["missing"] for r in rs)} | {sum(r["collapsed"] for r in rs)} |')
    lines += ['', 'Top origins of missing props after excluding layers (the first checker declaration):', '',
              '| Corpus | Package | Missing props |', '| --- | --- | --: |']
    for n in evals:
        origins = defaultdict(int)
        for item in missing_details[n]:
            if item['verdict'] == 'missing':
                origins[package_of(item['declarations'][0] if item['declarations'] else '')] += 1
        for package, count in sorted(origins.items(), key=lambda pair: -pair[1])[:5]:
            lines.append(f'| {n} | {package} | {count} |')
    lines += ['', 'Share of missing props caught by routing the top fraction of matched components '
              '(leave-one-corpus-out, equal corpus weight):', '',
              '| Corpus | Missing props | Method | ' + ' | '.join(f'{int(b * 100)}%' for b in BUDGETS) +
              ' | Rule natural point |', '| --- | --: | --- |' + ' --: |' * len(BUDGETS) + ' --: |']
    macro = {m: [0.0] * len(BUDGETS) for m in ('rule', 'table', 'model')}
    used = 0
    for held in evals:
        train = [(r, 1 / len(presence[n])) for n in names if n != held for r in presence[n]]
        rows_t, w_t = [r for r, _ in train], [w for _, w in train]
        counts = table_fit(rows_t, w_t, 'has_missing', PRESENCE_COARSE)
        model = lr_fit(rows_t, w_t, 'has_missing', PRESENCE_FEATURES)
        scores = {
            'rule': lambda r: r['nwarn'] + (1 if r['warn'] else 0),
            'table': lambda r, c=counts: table_rate(c, r, PRESENCE_COARSE),
            'model': lambda r, m=model: lr_rate(m, r, PRESENCE_FEATURES),
        }
        nat = natural_rule_point(presence[held], 'missing')
        for method, score in scores.items():
            caught, _, positives = caught_at(presence[held], score, BUDGETS, 'missing')
            if positives and held != 'fixtures':
                if method == 'rule':
                    used += 1
                macro[method] = [a + caught[b] for a, b in zip(macro[method], BUDGETS)]
            lines.append(f'| {held} | {positives} | {method} | ' +
                         ' | '.join(f'{caught[b] * 100:.0f}%' for b in BUDGETS) +
                         (f' | {nat[0] * 100:.0f}% routed, {nat[1] * 100:.0f}% caught |'
                          if method == 'rule' else ' | |'))
    if used:
        for method in ('rule', 'table', 'model'):
            lines.append(f'| **macro (5 corpora)** | | {method} | ' +
                         ' | '.join(f'{v / used * 100:.0f}%' for v in macro[method]) + ' | |')
    lines += ['', '## Shared prop type disagreements', '',
              'The following tables cover only props both tools report. They cannot measure a router’s ability to '
              'recover the missing props above.', '']
    results = {}
    for label in ('soft', 'real', 'hard'):
        results[label] = {}
        for held in evals:
            train = [(r, 1 / len(data[n])) for n in names if n != held for r in data[n]]
            rows_t, w_t = [r for r, _ in train], [w for _, w in train]
            counts = table_fit(rows_t, w_t, label) if label != 'hard' else None
            model = lr_fit(rows_t, w_t, label) if label != 'hard' else None
            test = data[held]
            if label == 'hard':
                # `wrong` alone is too rare to fit: score with the soft-label models trained above.
                counts, model = results['real'][held]['_fit']
            fits = {
                'rule': lambda r: r['nwarn'] + (1 if r['warn'] else 0),
                'table': lambda r, c=counts: table_rate(c, r),
                'model': lambda r, m=model: lr_rate(m, r),
            }
            results[label][held] = {'_fit': (counts, model)}
            for method, score in fits.items():
                caught, ncomp, pos = caught_at(test, score, BUDGETS, label)
                results[label][held][method] = caught
                results[label][held]['p_' + method] = caught_props_at(test, score, PROP_BUDGETS, label)
            results[label][held]['_n'] = (len(test), pos)
            results[label][held]['_natural'] = natural_rule_point(test, label)
    for label, title in (('soft', 'Soft label (wrong or less precise)'),
                         ('real', 'Real label (soft, minus the `(string & {})` idiom)'), ('hard', 'Wrong only')):
        lines += ['', f'## {title}: share of disagreements caught when the top B of components are routed',
                  '(leave-one-corpus-out; `rule` ranks by warning/error diagnostic count, its natural point is the '
                  'share routed and caught when every component with one is routed)', '']
        lines.append('| Corpus | Positives | Method | ' + ' | '.join(f'{int(b * 100)}%' for b in BUDGETS) +
                     ' | Rule natural point |')
        lines.append('| --- | --: | --- |' + ' --: |' * len(BUDGETS) + ' --: |')
        macro = {m: [0.0] * len(BUDGETS) for m in ('rule', 'table', 'model')}
        used = 0
        for held in evals:
            res = results[label][held]
            pos = res['_n'][1]
            if not pos:
                continue
            if held != 'fixtures':
                used += 1
            for m in ('rule', 'table', 'model'):
                vals = [res[m][b] for b in BUDGETS]
                if held != 'fixtures':
                    macro[m] = [a + v for a, v in zip(macro[m], vals)]
                nat = res['_natural']
                lines.append(f'| {held} | {pos} | {m} | ' + ' | '.join(f'{v * 100:.0f}%' for v in vals) +
                             (f' | {nat[0] * 100:.0f}% routed, {nat[1] * 100:.0f}% caught |' if m == 'rule' else ' | |'))
        if used:
            for m in ('rule', 'table', 'model'):
                lines.append(f'| **macro (5 corpora)** | | {m} | ' +
                             ' | '.join(f'{v / used * 100:.0f}%' for v in macro[m]) + ' | |')
    for label, title in (('soft', 'soft label'), ('real', 'real label'), ('hard', 'wrong only')):
        lines += ['', f'## Routing props, not components ({title}): share of disagreements caught when the top B of '
                  'props are sent to the checker', '', '| Corpus | Positives | Method | ' +
                  ' | '.join(f'{b * 100:g}%' for b in PROP_BUDGETS) + ' |', '| --- | --: | --- |' +
                  ' --: |' * len(PROP_BUDGETS)]
        macro = {m: [0.0] * len(PROP_BUDGETS) for m in ('rule', 'table', 'model')}
        used = 0
        for held in evals:
            res = results[label][held]
            pos = res['_n'][1]
            if not pos:
                continue
            used += held != 'fixtures'
            for m in ('rule', 'table', 'model'):
                vals = [res['p_' + m][b] for b in PROP_BUDGETS]
                if held != 'fixtures':
                    macro[m] = [a + v for a, v in zip(macro[m], vals)]
                lines.append(f'| {held} | {pos} | {m} | ' + ' | '.join(f'{v * 100:.0f}%' for v in vals) + ' |')
        if used:
            for m in ('rule', 'table', 'model'):
                lines.append(f'| **macro (5 corpora)** | | {m} | ' +
                             ' | '.join(f'{v / used * 100:.0f}%' for v in macro[m]) + ' |')
    # Risk table: the highest-risk and the biggest signal groups.
    allrows = [r for n in names for r in data[n]]
    lines += ['', '## Disagreement rate by signal (all corpora, pooled, soft label)', '',
              '| Signal | Value | Props | Soft rate | Wrong rate |', '| --- | --- | --: | --: | --: |']
    for f in ('kind', 'terminal', 'opaque', 'named', 'depth', 'union', 'fn', 'inherited', 'source', 'rule', 'composes',
              'warn', 'info', 'code_UNRESOLVABLE_IMPORT', 'code_OPAQUE_TYPE', 'code_INDEXED_ACCESS_OPAQUE',
              'code_GENERIC_ARGUMENT_MISMATCH', 'code_UNKNOWN'):
        by = defaultdict(lambda: [0, 0, 0])
        for r in allrows:
            c = by[r[f]]
            c[0] += 1
            c[1] += r['soft']
            c[2] += r['hard']
        for v, (n, s, h) in sorted(by.items(), key=lambda kv: -kv[1][0]):
            if n >= 50:
                lines.append(f'| {f} | {v} | {n} | {s / n * 100:.2f}% | {h / n * 100:.3f}% |')
    # What the table misses at a 10% component budget, by the kind of type and how the resolver left it.
    missed = defaultdict(lambda: [0, 0])
    total_missed = 0
    for n in evals:
        if n in ('fixtures', 'chakra'):
            continue
        counts, _ = results['real'][n]['_fit']
        routed = routed_set(data[n], lambda r, c=counts: table_rate(c, r), 0.10)
        for r in data[n]:
            if r['real'] and r['component'] not in routed:
                m = missed[(r['kind'], r['named'], r['opaque'], r['source'])]
                m[0] += 1
                total_missed += 1
    for n in evals:
        if n in ('fixtures', 'chakra'):
            continue
        for r in data[n]:
            m = missed.get((r['kind'], r['named'], r['opaque'], r['source']))
            if m is not None:
                m[1] += 1
    lines += ['', '## What the table misses at a 10% component budget (mantine, shadcn, antd, spectrum, pooled)', '',
              f'{total_missed} disagreeing props sit in unrouted components. Largest groups (kind, named references '
              'bucket, opaque nodes bucket, declared in):', '', '| Kind | Named | Opaque | Source | Missed | Share of missed | '
              'Props in group |', '| --- | --: | --: | --- | --: | --: | --: |']
    for (kind, named, opaque, source), (miss, size) in sorted(missed.items(), key=lambda kv: -kv[1][0])[:14]:
        lines.append(f'| {kind} | {named} | {opaque} | {source} | {miss} | {miss / total_missed * 100:.0f}% | {size} |')
    # Confidently wrong: oracle `wrong`, no warning or error on the component, low risk-table rank.
    lines += ['', '## Confidently wrong: `wrong` where the component has no warning or error diagnostic', '']
    seen = 0
    checker = {}
    for n in names:
        for d in dumps[n]:
            checker[(n, d['component'], d['prop'])] = d
    for n in evals:
        counts, _ = results['real'][n]['_fit']
        for r in data[n]:
            if r['hard'] and not r['warn'] and seen < 40:
                d = checker.get((n, r['component'], r['prop']))
                lines.append(f'- {n} `{r["component"]}.{r["prop"]}` kind `{r["kind"]}`, rule `{r["rule"]}`, '
                             f'source `{r["source"]}`, table risk {table_rate(counts, r) * 100:.1f}%' +
                             (f', checker `{d["checker"][:90]}`' if d else ''))
                seen += 1
    text = '\n'.join(lines) + '\n'
    if args.out:
        with open(args.out, 'w') as f:
            f.write(text)
    else:
        print(text)
    if args.json:
        with open(args.json, 'w') as f:
            json.dump({n: data[n] for n in names}, f)


if __name__ == '__main__':
    main()
