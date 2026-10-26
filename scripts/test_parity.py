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


if __name__ == '__main__':
    unittest.main()
