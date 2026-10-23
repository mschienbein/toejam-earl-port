"""ROM/G-free regression tests for the generator's deliberately restricted C grammar."""
from __future__ import annotations

import unittest
from pathlib import Path
import tempfile

from generate import CType, Model


class DeclaratorsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.model = Model.__new__(Model)
        self.model.constants = {"MAP_ROWS": "26", "MAP_COLUMNS": "19"}
        self.model.records = {}

    def test_multiplication_in_bounds_is_not_a_pointer(self) -> None:
        field, = self.model.parse_declaration("unsigned char grid[MAP_ROWS * 2][MAP_COLUMNS]")
        self.assertEqual(field.ctype, CType("unsigned char", 0, (52, 19)))
        self.assertEqual(self.model.shape(field.ctype)[0], 988)

    def test_parenthesized_bounds_are_not_discarded_as_function_declarations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "include").mkdir()
            (root / "include/test.h").write_text(
                "#define ROWS 26\nextern unsigned char grid[ROWS * (2)][19];\n"
                "extern long function();\nextern void (*callback)();\n")
            model = Model(root)
            self.assertEqual(model.declarations["grid"][0], CType("unsigned char", 0, (52, 19)))
            self.assertNotIn("function", model.declarations)
            self.assertEqual(model.declarations["callback"][0], CType("function", 1))

    def test_real_pointer_depth_is_independent_of_bounds(self) -> None:
        field, = self.model.parse_declaration("unsigned char **rows[3 * 2][5]")
        self.assertEqual(field.ctype, CType("unsigned char", 2, (6, 5)))
        self.assertEqual(self.model.shape(field.ctype)[0], 120)
        pointer, = self.model.parse_declaration("unsigned char * * * triple")
        self.assertEqual(pointer.ctype, CType("unsigned char", 3))

    def test_each_comma_declarator_has_its_own_pointer_prefix(self) -> None:
        left, right = self.model.parse_declaration("char *left, **right[2 * 3]")
        self.assertEqual(left.ctype, CType("char", 1))
        self.assertEqual(right.ctype, CType("char", 2, (6,)))

    def test_incomplete_outer_array_retains_its_row_type(self) -> None:
        field, = self.model.parse_declaration("unsigned short table[][3 * 4]")
        self.assertEqual(field.ctype, CType("unsigned short", 0, (None, 12)))

    def test_function_pointer_is_not_a_function_body_or_array_operator(self) -> None:
        field, = self.model.parse_declaration("void (*callback)()")
        self.assertEqual(field.ctype, CType("function", 1))

    def test_unmeasured_declarators_and_bitfields_are_rejected(self) -> None:
        for declaration in ("short flags:3", "char (*row)[3]", "char (*callbacks[2])()", "char invalid[2][]"):
            with self.subTest(declaration=declaration):
                with self.assertRaises(ValueError):
                    self.model.parse_declaration(declaration)


if __name__ == "__main__":
    unittest.main()
