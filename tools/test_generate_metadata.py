"""ROM-free C array-parameter ABI generation regressions."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from tree_sitter_language_pack import get_parser


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "generate_metadata", ROOT / "crates/tje-game/tools/generate_metadata.py"
)
metadata = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(metadata)


class ArrayParameterTests(unittest.TestCase):
    def setUp(self) -> None:
        self.parser = get_parser("c")

    def test_knr_arrays_adjust_to_pointers_including_multiple_declarators(self) -> None:
        header = b"""void example(text, names, palette)
char text[], *names[5];
unsigned short palette[16];
"""
        self.assertEqual(metadata.array_parameter_types(header, "example", self.parser), {
            "text": {"type": "char *", "source_declaration": "char text[]"},
            "names": {"type": "char **", "source_declaration": "char *names[5]"},
            "palette": {"type": "unsigned short *", "source_declaration": "unsigned short palette[16]"},
        })

    def test_prototype_arrays_adjust_only_the_outer_dimension(self) -> None:
        header = b"void example(const char text[32], char grid[][16], char (*row)[16]) "
        self.assertEqual(metadata.array_parameter_types(header, "example", self.parser), {
            "text": {"type": "const char *", "source_declaration": "const char text[32]"},
            "grid": {"type": "char (*)[16]", "source_declaration": "char grid[][16]"},
        })

    def test_scalar_and_explicit_pointer_parameters_are_not_rewritten(self) -> None:
        header = b"void example(text, length, glyph)\nchar *text;\nshort length;\nchar glyph;\n"
        self.assertEqual(metadata.array_parameter_types(header, "example", self.parser), {})

    def test_wrong_inventory_definition_fails_instead_of_guessing(self) -> None:
        with self.assertRaisesRegex(ValueError, "does not match inventory"):
            metadata.array_parameter_types(b"void other(text)\nchar text[];\n", "example", self.parser)

    def test_regeneration_reconciles_inventory_without_changing_its_source(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "src/cutscenes/example.c"
            source.parent.mkdir(parents=True)
            # The preceding K&R definition reproduces the full-unit parser
            # ambiguity; source_abis must use the inventory-identified header.
            source.write_text("""void before(source, count)
char *source;
short count;
{}
void example(text, length, tile_base)
char text[];
short length, tile_base;
{
    char local_array[10];
}
""")
            abi = {
                "kind": "mcc68k_c", "argument_count": 3,
                "arguments": [
                    {"name": "text", "type": "char", "declared_width_bits": 8,
                     "slot_bytes": 4, "entry_sp_slot_offset": 4, "entry_sp_value_offset": 7},
                    {"name": "length", "type": "short", "declared_width_bits": 16,
                     "slot_bytes": 4, "entry_sp_slot_offset": 8, "entry_sp_value_offset": 10},
                    {"name": "tile_base", "type": "short", "declared_width_bits": 16,
                     "slot_bytes": 4, "entry_sp_slot_offset": 12, "entry_sp_value_offset": 14},
                ],
                "return": {"declared_type": "void", "register": None},
                "saved_registers": [],
            }
            entry = {
                "id": "synthetic_example", "name": "example",
                "addresses": {"rev02": "0x1000", "rev00": "0x2000"},
                "sizes": {"rev02": 16, "rev00": 16}, "classification": "compiled_c",
                "kind": "function", "subsystem": "cutscenes",
                "source_file": "src/cutscenes/example.c", "source_line": 5,
                "leaf_first": {"wave": 1}, "all_dependency_callees": [],
                "calling_convention": abi,
            }
            inventory = root / "inventory.json"
            original = json.dumps({"functions": [entry]}).encode()
            inventory.write_bytes(original)
            generated = metadata.generate(inventory, root)
            encoded_abi = generated["part_0.rs"].split("details_json: ", 1)[1].split(
                ", argument_count: ", 1
            )[0]
            actual = json.loads(json.loads(encoded_abi))
            self.assertEqual(actual["arguments"][0], abi["arguments"][0] | {
                "type": "char *", "declared_width_bits": 32, "entry_sp_value_offset": 4,
                "source_declaration": "char text[]",
            })
            self.assertEqual(actual["arguments"][1:], abi["arguments"][1:])
            self.assertIn("void example(char * text, short length, short tile_base)", generated["part_0.rs"])
            self.assertIn("// C declaration SHA-256: ", generated["part_0.rs"])
            self.assertEqual(inventory.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
