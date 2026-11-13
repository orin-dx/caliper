#!/usr/bin/env python3
"""The weekly headline: how correct the types are that Storybook and agents are shown, and when to open an issue.

The headline is read from what the oracle already measured (apps/oracle/src/tool-universe.ts `caliper_reported`): every
own prop TypeScript 7.1's checker finds on a component Caliper reports, each judged against the checker. A prop is
correct (the same type, or the same type spelled differently), less precise (vaguer than the checker's, not
contradicting it), wrong (describing something else), without a checker type (the checker has none either), not judged
(the oracle did not evaluate the checker's type, so nothing was compared), missing (Caliper reports the component but
not the prop), or named only (Caliper lists it by name as a member of a collapsed inheritance layer, so it is surfaced
but has no type to judge).

This module holds the arithmetic and the issue conditions so scripts/weekly.py only calls it. Matching react-docgen-
typescript (recall) is information, never a gate.
"""
import oracle_data

# The gate. Of the props Caliper lists whose type can be judged against the checker's, more than this share wrong opens
# an issue, and so does any rise from the last record: a wrong type in Storybook is the one thing a reader cannot see.
WRONG_GATE = 0.001

# Budgets for one whole run (process start included) of the checker, the default, over a pinned corpus with dependencies
# loaded, per corpus: `ms` and `rss_mib`. None is "record on first run": no checker run of the five corpora has been
# measured, so none has a budget, and a run over nothing opens nothing. The rule that fills them in once a scheduled run
# is recorded is in docs/measurement.md ("The budgets"). They are not week-over-week limits: a shared CI machine drifts
# by 10% or more on its own, and a drift that stays inside the budget is not a problem.
BUDGETS = {corpus: {'ms': None, 'rss_mib': None} for corpus in ('mantine', 'chakra', 'shadcn', 'antd', 'spectrum')}
BUDGET_LABELS = {'ms': 'time', 'rss_mib': 'peak memory'}
BUDGET_UNITS = {'ms': 'ms', 'rss_mib': 'MiB'}

# The keys of one corpus's headline, in the order the summary prints them.
KINDS = ('correct', 'less_precise', 'wrong', 'no_checker_type', 'not_judged', 'missing', 'named_only')

# The headline key each verdict of the oracle (apps/oracle/src/vocabulary.ts) is counted under; the verdicts that say
# Caliper's type is the checker's are `correct`. A verdict that is in neither is a KeyError, not a prop that vanishes.
VERDICT_KINDS = {'less precise': 'less_precise', 'wrong': 'wrong', 'checker any/error': 'no_checker_type',
                 'not judged': 'not_judged'}


def kind_of(verdict):
    return 'correct' if verdict in oracle_data.AGREE else VERDICT_KINDS[verdict]


def compute(oracle):
    """The headline of one corpus from its oracle entry, or None when the oracle did not run, failed, or is a record from
    before the headline existed (it has no `caliper_reported`)."""
    scored = ((oracle or {}).get('tools') or {}).get('caliper_reported')
    if not scored:
        return None
    verdicts, reasons = scored['verdicts'], scored['missing_by_reason']
    counts = {kind: 0 for kind in KINDS}
    for verdict in oracle_data.VERDICTS:
        counts[kind_of(verdict)] += verdicts[verdict]
    counts['missing'] = reasons['prop']
    counts['named_only'] = reasons['collapsed']
    # A component Caliper reports is never missing for want of the component, and Caliper never throws on a file; both
    # are zero here, and counted if they are not so the parts always add up to the whole.
    other = scored['universe'] - sum(counts.values())
    return {'components': scored['components'], 'props': scored['universe'], **counts, **({'other': other} if other else {})}


def judged(h):
    """The props Caliper lists whose type can be judged: correct, less precise or wrong."""
    return h['correct'] + h['less_precise'] + h['wrong']


def wrong_rate(h):
    """Wrong, of the props whose type can be judged."""
    return h['wrong'] / judged(h) if judged(h) else 0.0


def share(h, kind):
    """One kind, of every prop of the components Caliper reports."""
    return h[kind] / h['props'] if h['props'] else 0.0


def own_peak(entry):
    """Caliper's peak memory of a whole run, or None when the record cannot say it: absent, or the same figure to the MiB as
    both comparison tools' on the corpus. Three tools never peak together, so that figure is one process-wide maximum read for
    every run (the 2026-11-02 record, GitHub issue 279, read 1901 MiB for shadcn, antd and spectrum, each tool), not Caliper's."""
    peak = (entry.get('caliper') or {}).get('rss_mib')
    others = [(entry.get(tool) or {}).get('rss_mib') for tool in ('rdt', 'react_docgen')]
    if peak is None or all(other == peak for other in others):
        return None
    return peak


def own_metric(entry, key):
    """Caliper's own `key` ('ms' or 'rss_mib') of a corpus entry, or None when it is absent or not Caliper's own."""
    return own_peak(entry) if key == 'rss_mib' else (entry.get('caliper') or {}).get(key)


def findings(name, entry, previous):
    """What opens the issue for one corpus: a wrong type above the gate or higher than last week, or a whole run over a
    budget. `previous` is the corpus entry of the last record, or None."""
    out = []
    h = entry.get('headline')
    if h:
        rate = wrong_rate(h)
        old = (previous or {}).get('headline')
        rose = old is not None and rate > wrong_rate(old) + 1e-9
        was = f", up from {old['wrong']:,} of {judged(old):,}, {100 * wrong_rate(old):.2f}% last week" if rose else ''
        if rate > WRONG_GATE:
            out.append(f"{name}: {h['wrong']:,} of the {judged(h):,} props whose type can be judged are wrong, "
                       f"{100 * rate:.2f}%{was}; the gate is {100 * WRONG_GATE:.1f}%")
        elif rose:
            out.append(f"{name}: wrong types rose to {h['wrong']:,} of {judged(h):,}, {100 * rate:.2f}%{was}")
    for key, budget in BUDGETS.get(name, {}).items():
        value = own_metric(entry, key)
        if budget is not None and value is not None and value > budget:
            out.append(f"{name}: a whole run's {BUDGET_LABELS[key]} is {value:,} {BUDGET_UNITS[key]}, over the "
                       f"{budget:,} {BUDGET_UNITS[key]} budget")
    return out


HOW_TO_READ = (
    'How to read these numbers. The headline asks one question: for the components Caliper reports, are the types in its '
    "output right? Each prop is judged against TypeScript 7.1's checker, which is the truth here, not against another "
    'tool. A prop that Caliper lists only by name in a collapsed inheritance layer is surfaced, but a reader sees no type '
    'for it, so it is counted apart. Recall against react-docgen-typescript (RDT) asks a different question, how much '
    'of what RDT reports Caliper also reports; it is information, because RDT is itself wrong on some props. Gates, '
    f'which open an issue: more than {100 * WRONG_GATE:.1f}% wrong (or any rise), a silent-wrong answer to a question a '
    "reader is asked, or a whole run over its corpus's time or memory budget (a corpus has none until its first run is "
    'recorded). Time and memory drift from week to week is shown, not alarmed on.'
)

DEFINITIONS = (
    'Correct: the same type as the checker, or the same type spelled differently. Less precise: vaguer than the '
    "checker's (a name left unexpanded, `string` for a union of literals), not contradicting it. Wrong: describes "
    'something else; this is the gate. No checker type: the checker has none either (an unresolved import, `any`). '
    "Not judged: the oracle did not evaluate the checker's type (it was cut at the depth limit, or is a kind the "
    'oracle does not read), so nothing was compared; it is neither agreement nor a difference. '
    'Missing: Caliper reports the component but not the prop. Named only: listed by name in a collapsed layer, with no '
    'type to judge.'
)


def markdown(record):
    """The top of the weekly summary: how to read the numbers, then the headline when the oracle ran."""
    lines = [HOW_TO_READ, '']
    rows = [(n, e['headline']) for n, e in record['corpora'].items() if e.get('headline')]
    errors = [(n, e['oracle']['error']) for n, e in record['corpora'].items() if 'error' in e.get('oracle', {})]
    if not rows and not errors:
        return lines
    lines += ['### Headline: are the types Storybook shows right?', '', DEFINITIONS, '',
              '| Corpus | Props | Correct | Less precise | Wrong | No checker type | Not judged | Missing | Named only |',
              '| --- | --: | --: | --: | --: | --: | --: | --: | --: |']
    for name, h in rows:
        cells = [f'{100 * share(h, kind):.{2 if kind == "wrong" else 1}f}%' for kind in KINDS]
        lines.append(f"| {name} | {h['props']:,} | " + ' | '.join(cells) + ' |')
    for name, error in errors:
        lines.append(f'| {name} | unavailable: {error[:80]} | | | | | | | |')
    return lines + ['']
