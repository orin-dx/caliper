#!/usr/bin/env python3
"""The weekly headline: how correct the types are that Storybook and agents are shown, and when to open an issue.

The headline is read from what the oracle already measured (apps/oracle/src/tool-universe.ts `caliper_reported`): every
own prop TypeScript 7.1's checker finds on a component Caliper reports, each judged against the checker. A prop is
correct (the same type, or the same type spelled differently), less precise (vaguer than the checker's, not
contradicting it), wrong (describing something else), without a checker type (the checker has none either), missing
(Caliper reports the component but not the prop), or named only (Caliper lists it by name as a member of a collapsed
inheritance layer, so it is surfaced but has no type to judge).

This module holds the arithmetic and the issue conditions so scripts/weekly.py only calls it. Matching react-docgen-
typescript (recall) is information, never a gate.
"""

# The gate. Of the props Caliper lists whose type can be judged against the checker's, more than this share wrong opens
# an issue, and so does any rise from the last record: a wrong type in Storybook is the one thing a reader cannot see.
WRONG_GATE = 0.001

# User-facing budgets for one whole run (process start included) over a pinned corpus with dependencies loaded, the
# default. They are not week-over-week limits: a shared CI machine drifts by 10% or more on its own, and a drift that
# stays inside the budget is not a problem. Chakra, the largest corpus, runs in about 1 s and 446 MiB.
BUDGETS = {'ms': 2000, 'rss_mib': 600}
BUDGET_LABELS = {'ms': 'time', 'rss_mib': 'peak memory'}
BUDGET_UNITS = {'ms': 'ms', 'rss_mib': 'MiB'}

# The keys of one corpus's headline, in the order the summary prints them.
KINDS = ('correct', 'less_precise', 'wrong', 'no_checker_type', 'missing', 'named_only')


def compute(oracle):
    """The headline of one corpus from its oracle entry, or None when the oracle did not run, failed, or is a record from
    before the headline existed (it has no `caliper_reported`)."""
    scored = ((oracle or {}).get('tools') or {}).get('caliper_reported')
    if not scored:
        return None
    verdicts, reasons = scored['verdicts'], scored['missing_by_reason']
    counts = {
        'correct': verdicts['identical'] + verdicts['spelled'],
        'less_precise': verdicts['less precise'],
        'wrong': verdicts['wrong'],
        'no_checker_type': verdicts['checker any/error'],
        'missing': reasons['prop'],
        'named_only': reasons['collapsed'],
    }
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


def findings(name, entry, previous):
    """What opens the issue for one corpus: a wrong type above the gate or higher than last week, or a whole run over a
    budget. `previous` is the corpus entry of the last record, or None."""
    out = []
    h = entry.get('headline')
    if h:
        rate = wrong_rate(h)
        old = (previous or {}).get('headline')
        rose = old is not None and rate > wrong_rate(old) + 1e-9
        was = f', up from {100 * wrong_rate(old):.2f}% last week' if rose else ''
        if rate > WRONG_GATE:
            out.append(f"{name}: {h['wrong']:,} of the {judged(h):,} props whose type can be judged are wrong, "
                       f"{100 * rate:.2f}%{was}; the gate is {100 * WRONG_GATE:.1f}%")
        elif rose:
            out.append(f"{name}: wrong types rose to {h['wrong']:,} of {judged(h):,}, {100 * rate:.2f}%{was}")
    caliper = entry.get('caliper') or {}
    for key, budget in BUDGETS.items():
        if key in caliper and caliper[key] > budget:
            out.append(f"{name}: a whole run's {BUDGET_LABELS[key]} is {caliper[key]:,} {BUDGET_UNITS[key]}, over the "
                       f"{budget:,} {BUDGET_UNITS[key]} budget")
    return out


HOW_TO_READ = (
    'How to read these numbers. The headline asks one question: for the components Caliper reports, are the types in its '
    "output right? Each prop is judged against TypeScript 7.1's checker, which is the truth here, not against another "
    'tool. A prop that Caliper lists only by name in a collapsed inheritance layer is surfaced, but a reader sees no type '
    'for it, so it is counted apart. Recall against react-docgen-typescript (RDT) asks a different question, how much '
    'of what RDT reports Caliper also reports; it is information, because RDT is itself wrong on some props. Gates, '
    f'which open an issue: more than {100 * WRONG_GATE:.1f}% wrong (or any rise), a silent-wrong answer to a question a '
    f"reader is asked, or a whole run over {BUDGETS['ms'] / 1000:.0f} s or {BUDGETS['rss_mib']} MiB. Time and memory "
    'drift from week to week is shown, not alarmed on.'
)

DEFINITIONS = (
    'Correct: the same type as the checker, or the same type spelled differently. Less precise: vaguer than the '
    "checker's (a name left unexpanded, `string` for a union of literals), not contradicting it. Wrong: describes "
    'something else; this is the gate. No checker type: the checker has none either (an unresolved import, `any`). '
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
              '| Corpus | Props | Correct | Less precise | Wrong | No checker type | Missing | Named only |',
              '| --- | --: | --: | --: | --: | --: | --: | --: |']
    for name, h in rows:
        cells = [f'{100 * share(h, kind):.{2 if kind == "wrong" else 1}f}%' for kind in KINDS]
        lines.append(f"| {name} | {h['props']:,} | " + ' | '.join(cells) + ' |')
    for name, error in errors:
        lines.append(f'| {name} | unavailable: {error[:80]} | | | | | | |')
    return lines + ['']
