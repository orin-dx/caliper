#!/usr/bin/env python3
"""Tests for the agent evaluation's reader and scoring, on small fixtures. Run: python3 scripts/test_agent_eval.py"""
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import agent_eval as ev
import weekly

TOON = """docgenComponents[2]:
  component:Button:
    filePath: /r/src/Button.tsx
    props[4] {name,required,type,default,source,degradedBy,description}:
      kind,false,'primary'|'secondary',,ButtonProps,,"Visual kind, one of two"
      size,false,'sm'|'md'|'lg'|...(+3),,ButtonProps,,
      onClick,false,"(MouseEvent<HTMLButtonElement>, boolean) => void",,DOMAttributes,,
      theme,true,Theme,,ButtonProps,unexpandedReference,
    inheritance[2] {typeName,htmlElement,totalProps,members}:
      React.ButtonHTMLAttributes,button,40,form|name|type
      React.HTMLAttributes,div,200,
  component:Card:
    filePath: /r/src/Card.tsx
    props[3] {name,required,type,default,source,description}:
      title,true,string,,CardProps,
      count,false,number|undefined,,CardProps,
      tone,false,'a'|'it\\'s',,CardProps,
diagnostics[1]:
  [Warning]: Something (src/Card.tsx)
stats: extracted=2, parsed=2, cacheHits=0, durationMs=3
"""


def question(kind, component='Card', file='src/Card.tsx', prop=None, id='q'):
    return {'id': id, 'corpus': 'c', 'kind': kind, 'file': file, 'component': component, **({'prop': prop} if prop else {})}


def views():
    found = {(f[len('/r/'):], n): s for (f, n), s in ev.parse_toon(TOON).items()}
    return found


class Rows(unittest.TestCase):
    def test_quoted_cells(self):
        self.assertEqual(ev.split_row('a,"b, c",d'), ['a', 'b, c', 'd'])
        self.assertEqual(ev.split_row('a,"say \\"hi\\"",'), ['a', 'say "hi"', ''])

    def test_commas_the_renderer_left_in_the_last_cell(self):
        self.assertEqual(ev.row('x,true,string,,P,one, two', ['name', 'required', 'type', 'default', 'source', 'description']),
                         {'name': 'x', 'required': 'true', 'type': 'string', 'default': '', 'source': 'P', 'description': 'one, two'})


class Parse(unittest.TestCase):
    def test_components_props_and_layers(self):
        found = views()
        button = found[('src/Button.tsx', 'Button')]
        self.assertEqual(list(button.props), ['kind', 'size', 'onClick', 'theme'])
        self.assertEqual(button.props['kind']['type'], "'primary'|'secondary'")
        self.assertEqual(button.props['onClick']['type'], '(MouseEvent<HTMLButtonElement>, boolean) => void')
        self.assertEqual(button.props['theme']['degraded_by'], ['unexpandedReference'])
        self.assertEqual(button.layers, [{'type_name': 'React.ButtonHTMLAttributes', 'total': 40, 'members': ['form', 'name', 'type']},
                                         {'type_name': 'React.HTMLAttributes', 'total': 200, 'members': []}])
        self.assertTrue(button.props['theme']['required'] and not button.props['kind']['required'])

    def test_a_table_without_the_degraded_column(self):
        card = views()[('src/Card.tsx', 'Card')]
        self.assertEqual(list(card.props), ['title', 'count', 'tone'])
        self.assertEqual(card.props['title'], {'required': True, 'type': 'string', 'degraded_by': []})
        self.assertEqual(card.layers, [])


class Types(unittest.TestCase):
    def test_kinds(self):
        for text, kind in (('boolean', 'boolean'), ('string', 'string'), ('number|undefined', 'number'),
                           ("'a'|'b'", 'literal_union'), ("'a'", 'literal_union'), ('Array<string>', 'array'),
                           ('(string) => void', 'function'), ('(number) => any|PromiseLike<any>', 'function'),
                           ('true|false', 'boolean')):
            self.assertEqual(ev.kind_of(text), kind, text)

    def test_what_a_reader_cannot_read(self):
        for text in ('ReactNode', 'ButtonProps', "'a'|'b'|...(+4)", 'opaque(x)', 'boolean|{ delay?: number }',
                     'string|number', '', "boolean|'indeterminate'"):
            self.assertIsNone(ev.kind_of(text), text)

    def test_literals(self):
        self.assertEqual(ev.literals_of("'b'|'a'|null"), ['a', 'b'])
        self.assertEqual(ev.literals_of("'it\\'s'"), ["it's"])
        self.assertIsNone(ev.literals_of("'a'|'b'|...(+4)"))
        self.assertIsNone(ev.literals_of("'a'|string"))

    def test_an_arrow_does_not_close_a_bracket(self):
        self.assertEqual(ev.split_top('Array<(a) => b>|string', '|'), ['Array<(a) => b>', 'string'])
        self.assertEqual(ev.split_top("'a|b'|'c'", '|'), ["'a|b'", "'c'"])


class Reader(unittest.TestCase):
    def ask(self, kind, **kw):
        return ev.answer(question(kind, **kw), views(), kw.pop('files', {}))

    def test_present(self):
        self.assertTrue(ev.answer(question('present'), views(), {}))
        # Absent with nothing said about the file: the reader says no, and can be silently wrong.
        self.assertIs(ev.answer(question('present', component='Ghost'), views(), {}), False)
        self.assertIs(ev.answer(question('present', component='Ghost'), views(), {'src/Card.tsx': [{'severity': 'info'}]}), ev.ABSTAIN)

    def test_prop_exists(self):
        self.assertIs(self.ask('prop_exists', prop='title'), True)
        self.assertIs(self.ask('prop_exists', prop='nope'), False)
        # A name only a layer's member list carries.
        self.assertIs(self.ask('prop_exists', component='Button', file='src/Button.tsx', prop='form'), True)
        # A layer with props and no list could be holding it.
        self.assertIs(self.ask('prop_exists', component='Button', file='src/Button.tsx', prop='nope'), ev.ABSTAIN)
        self.assertIs(self.ask('prop_exists', component='Ghost', prop='x'), ev.ABSTAIN)

    def test_prop_required(self):
        self.assertIs(self.ask('prop_required', prop='title'), True)
        self.assertIs(self.ask('prop_required', prop='count'), False)
        self.assertIs(self.ask('prop_required', prop='nope'), ev.ABSTAIN)

    def test_a_degraded_prop_is_not_answered(self):
        self.assertIs(self.ask('prop_required', component='Button', file='src/Button.tsx', prop='theme'), ev.ABSTAIN)

    def test_a_warning_on_the_component_is_not_answered_over(self):
        found = views()
        found[('src/Card.tsx', 'Card')].diagnostics.append(('warning', 'props may be incomplete'))
        self.assertIs(ev.answer(question('prop_required', prop='title'), found, {}), ev.ABSTAIN)
        found[('src/Card.tsx', 'Card')].diagnostics[:] = [('info', 'fyi')]
        self.assertIs(ev.answer(question('prop_required', prop='title'), found, {}), True)

    def test_literals_and_kinds(self):
        self.assertEqual(self.ask('literal_values', component='Button', file='src/Button.tsx', prop='kind'), ['primary', 'secondary'])
        self.assertEqual(self.ask('literal_values', prop='tone'), ["a", "it's"])
        # Cut short: the reader cannot say what the values are.
        self.assertIs(self.ask('literal_values', component='Button', file='src/Button.tsx', prop='size'), ev.ABSTAIN)
        self.assertEqual(self.ask('type_kind', prop='count'), 'number')
        self.assertEqual(self.ask('type_kind', component='Button', file='src/Button.tsx', prop='onClick'), 'function')
        self.assertIs(self.ask('type_kind', prop='nope'), ev.ABSTAIN)


class Scoring(unittest.TestCase):
    def test_classes(self):
        self.assertEqual(ev.classify(True, True), 'correct')
        self.assertEqual(ev.classify(['a'], ['a']), 'correct')
        self.assertEqual(ev.classify(ev.ABSTAIN, True), 'abstained')
        self.assertEqual(ev.classify(False, True), 'silent_wrong')
        # False is an answer, not a missing one.
        self.assertEqual(ev.classify(False, False), 'correct')

    def test_a_record(self):
        questions = [question('prop_required', prop='title', id='right'), question('prop_required', prop='count', id='wrong'),
                     question('prop_required', prop='nope', id='unsure'), question('prop_exists', prop='x', id='skipped')]
        truth = {'right': {'value': True}, 'wrong': {'value': True}, 'unsure': {'value': False},
                 'skipped': {'invalid': 'the checker finds no component Card in src/Card.tsx'}}
        record = ev.score(questions, views(), {}, truth)
        self.assertEqual((record['questions'], record['correct'], record['abstained'], record['silent_wrong']), (3, 1, 1, 1))
        self.assertEqual(record['correct_rate'], 0.3333)
        self.assertEqual(record['silent_wrong_items'], [{'id': 'wrong', 'answer': False, 'checker': True}])
        self.assertEqual(record['invalid'], [{'id': 'skipped', 'why': 'the checker finds no component Card in src/Card.tsx'}])
        self.assertEqual(record['outcomes'], {'right': 'correct', 'wrong': 'silent_wrong', 'unsure': 'abstained'})
        self.assertEqual(record['by_kind']['prop_required'], {'correct': 1, 'abstained': 1, 'silent_wrong': 1})

    def test_a_question_the_checker_did_not_answer_is_not_scored(self):
        record = ev.score([question('present', id='a')], views(), {}, {})
        self.assertEqual((record['questions'], record['correct_rate'], len(record['invalid'])), (0, 0, 1))


def entry(**kw):
    base = {'questions': 20, 'correct': 16, 'abstained': 4, 'silent_wrong': 0, 'correct_rate': 0.8, 'invalid': [],
            'silent_wrong_items': []}
    return {'agent_eval': {**base, **kw}}


class Findings(unittest.TestCase):
    def test_a_silent_wrong_answer_is_a_finding(self):
        wrong = entry(silent_wrong=1, correct=15, correct_rate=0.75,
                      silent_wrong_items=[{'id': 'q1', 'answer': False, 'checker': True}])
        notes = ev.findings('antd', wrong)
        self.assertEqual(len(notes), 1)
        self.assertIn('antd: 1 silent-wrong answer(s) of 20', notes[0])
        self.assertIn("q1 (said False, checker True)", notes[0])

    def test_five_points_lower_is_a_change_and_not_a_finding(self):
        self.assertEqual(ev.changes('antd', entry(correct_rate=0.7), entry(correct_rate=0.8)),
                         ['antd: agent questions answered correctly 80% -> 70%'])
        self.assertEqual(ev.changes('antd', entry(correct_rate=0.76), entry(correct_rate=0.8)), [])
        self.assertEqual(ev.findings('antd', entry(correct_rate=0.7)), [])

    def test_nothing_to_compare(self):
        self.assertEqual(ev.changes('antd', entry(), None), [])
        self.assertEqual(ev.changes('antd', entry(), {'caliper': {}}), [])
        self.assertEqual(ev.findings('antd', {'agent_eval': {'error': 'boom'}}), [])
        self.assertEqual(ev.findings('antd', {}), [])

    def test_the_summary(self):
        record = {'corpora': {'antd': entry(), 'shadcn': {'agent_eval': {'error': 'no checker'}}, 'mantine': {}}}
        lines = ev.markdown(record)
        self.assertIn('| antd | 20 | 16 (80%) | 4 | 0 | 0 |', lines)
        self.assertIn('| shadcn | unavailable: no checker | | | | |', lines)
        self.assertFalse(any('mantine' in line for line in lines))
        self.assertEqual(ev.markdown({'corpora': {'antd': {}}}), [])


class InTheWeeklyRecord(unittest.TestCase):
    def record(self, **kw):
        caliper = {'ms': 100, 'rss_mib': 100}
        return {'caliper': 'x', 'runs': 1, 'reference_version': weekly.REFERENCE_VERSION,
                'corpora': {'antd': {'caliper': caliper, 'recall': {'caliper': {}}, **entry(**kw)}}}

    def test_a_silent_wrong_answer_is_reported_without_a_previous_record(self):
        wrong = self.record(silent_wrong=1, correct_rate=0.7, silent_wrong_items=[{'id': 'q', 'answer': 1, 'checker': 2}])
        notes = weekly.findings(wrong, {})
        self.assertEqual(len(notes), 1)
        self.assertTrue(notes[0].startswith('antd: 1 silent-wrong answer(s)'))

    def test_a_clean_record_has_no_findings(self):
        self.assertEqual(weekly.findings(self.record(), self.record()), [])

    def test_a_fall_in_the_correct_rate_is_a_change_and_opens_no_issue(self):
        now, before = self.record(correct_rate=0.7), self.record(correct_rate=0.8)
        self.assertEqual(weekly.findings(now, before), [])
        self.assertEqual(weekly.changes(now, before), ['antd: agent questions answered correctly 80% -> 70%'])

    def test_a_record_without_the_evaluation_has_no_findings(self):
        record = self.record()
        del record['corpora']['antd']['agent_eval']
        self.assertEqual(weekly.findings(record, {}), [])


class Questions(unittest.TestCase):
    def test_the_file_is_well_formed(self):
        questions = ev.load_questions()
        self.assertEqual(len({q['id'] for q in questions}), len(questions))
        for q in questions:
            self.assertIn(q['kind'], ev.KINDS, q['id'])
            self.assertTrue(q['file'] and q['component'], q['id'])
            self.assertEqual('prop' in q, q['kind'] != 'present', q['id'])
        self.assertEqual({q['kind'] for q in questions}, set(ev.KINDS))
        self.assertTrue(json.dumps(questions))


if __name__ == '__main__':
    unittest.main()
