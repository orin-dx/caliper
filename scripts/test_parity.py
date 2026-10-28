"""Tests for the loss patterns in scripts/parity.py. Run: python3 scripts/test_parity.py"""

import importlib.util
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('parity', os.path.join(HERE, 'parity.py'))
parity = importlib.util.module_from_spec(spec)
sys.modules['parity'] = parity
spec.loader.exec_module(parity)


class UnboundTypeParameter(unittest.TestCase):
    pattern = staticmethod(parity.LOSS_PATTERNS['unbound type parameter'])

    def test_a_parameter_RDT_does_not_name_is_a_loss(self):
        self.assertTrue(self.pattern('T | string', 'string | NumberInputNumericType'))

    def test_single_capital_letters_inside_string_literals_are_not_parameters(self):
        self.assertEqual(parity.type_params('"S" | "M" | "L"'), set())
        self.assertFalse(self.pattern('"S" | "M" | "L"', 'string'))

    def test_a_parameter_beside_string_literals_is_still_found(self):
        self.assertEqual(parity.type_params('"a" | T'), {'T'})


class RestParameterLost(unittest.TestCase):
    pattern = staticmethod(parity.LOSS_PATTERNS['rest parameter lost'])

    def test_a_dropped_rest_parameter_is_a_loss(self):
        self.assertTrue(self.pattern('(args: any[]) => void', '(...args: any[]) => void'))

    def test_a_kept_rest_parameter_is_not(self):
        self.assertFalse(self.pattern('(...args: any[]) => void', '(...args: any[]) => void'))

    def test_typescripts_own_elision_is_not_a_rest_parameter(self):
        self.assertFalse(self.pattern('"a" | "b" | "c"', '"a" | "b" | ... 40 more ... | "c"'))


if __name__ == '__main__':
    unittest.main()
