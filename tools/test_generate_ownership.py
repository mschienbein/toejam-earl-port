from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import generate_ownership as ownership
from trace_functions import TraceError


class OwnershipTests(unittest.TestCase):
    def test_metadata_uses_the_current_trace_catalogue_api(self) -> None:
        entries = ownership.metadata(ownership.ROOT)
        document = json.loads((ownership.ROOT / ownership.DESTINATION).read_text())
        self.assertEqual([entry["constant"] for entry in entries],
                         [entry["constant"] for entry in document["functions"]])
        self.assertTrue(all(entry["callees"] is not None for entry in entries))

    def test_transitive_metadata_and_source_reference_edges(self) -> None:
        entries = [{"index": 0, "callees": [1]}, {"index": 1, "callees": []},
                   {"index": 2, "callees": []}, {"index": 3, "callees": []}]
        references = [{"caller": 1, "target": 2}]
        self.assertEqual(ownership.closure(entries, references, [], [0]), [0, 1, 2])
        self.assertEqual(ownership.closure(entries, references, [{"caller": 1}], [0]), [0, 1, 2, 3])
        with self.assertRaises(TraceError):
            ownership.closure(entries, [], [], [4])

    def test_source_installs_and_indirect_calls_are_not_lost(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.c").write_text(
                "void First() { object->process = Second; (*object->process)(); }\n"
                "void Second() { Third(); }\n"
                "void Third() {}\n"
            )
            entries = [{"index": index, "name": name, "source": "source.c", "source_line": index + 1}
                       for index, name in enumerate(["First", "Second", "Third"])]
            edges, indirect, identities = ownership.source_edges(root, entries)
            self.assertTrue(any(edge["caller"] == 0 and edge["target"] == 1 for edge in edges))
            self.assertTrue(any(edge["caller"] == 1 and edge["target"] == 2 for edge in edges))
            self.assertTrue(any(site["caller"] == 0 for site in indirect))
            self.assertEqual(indirect[0]["unknown_runtime_target"], "reject")
            self.assertEqual(len(identities[0]["sha256"]), 64)

    def test_assembly_irq_or_music_indirect_transfer_is_conservative(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.s").write_text("First:\n    jsr (%a1)\n    lea Second,%a0\nSecond:\n    rte\n")
            entries = [{"index": 0, "name": "First", "source": "source.s", "source_line": 1},
                       {"index": 1, "name": "Second", "source": "source.s", "source_line": 4}]
            edges, indirect, _ = ownership.source_edges(root, entries)
            self.assertEqual(indirect[0]["kind"], "asm-indirect-transfer")
            self.assertEqual(edges[0]["target"], 1)

    def test_descriptor_inventory_contains_schemas_not_rom_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            unit = {"name": "AuthoredDescriptor", "kind": "records", "address": "0x001000", "size": 12,
                    "values": "PRIVATE-RESOURCE-CONTENTS-MUST-NOT-ESCAPE",
                    "params": {"fields": [{"name": "tag", "width": 2},
                               {"name": "callback", "width": 4, "type": "pointer"}]}}
            for revision in ["rev00", "rev02"]:
                destination = root / "config" / revision
                destination.mkdir(parents=True)
                (destination / "data_units.json").write_text(json.dumps({"units": [unit]}))
            result = ownership.descriptor_schemas(root)
            self.assertEqual(len(result), 2)
            self.assertEqual(result[0]["pointer_fields"], [{"field": "callback", "offset": 2, "width": 4}])
            self.assertNotIn("PRIVATE-RESOURCE", json.dumps(result))

    def test_checked_in_manifest_authenticates_complete_callback_and_irq_closure(self) -> None:
        root = ownership.ROOT
        document = json.loads((root / ownership.DESTINATION).read_text())
        identity = document.pop("identity_sha256")
        canonical = json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
        self.assertEqual(identity, hashlib.sha256(canonical).hexdigest())
        entries = document["functions"]
        self.assertEqual(len(entries), 648)
        expected = [{"function": entry["constant"]} for entry in entries]
        self.assertEqual(document["closure"], expected)
        self.assertEqual(document["irq_closure"], expected)
        reached = ownership.closure(entries, document["source_references"],
                                    document["indirect_sites"], document["roots"])
        self.assertEqual([{"function": entries[index]["constant"]} for index in reached], document["closure"])
        self.assertTrue(all(entry["disposition"] in {"mode_owned", "read_only", "device_audio_original"}
                            for entry in entries))
        self.assertTrue(all(entry["body_version"] > 0 for entry in entries))
        self.assertTrue(any(site["caller"] == 18 for site in document["indirect_sites"]))
        self.assertTrue(any(site["caller"] == 639 for site in document["indirect_sites"]))
        self.assertEqual({schema["revision"] for schema in document["descriptor_schemas"]}, {"rev00", "rev02"})
        document["identity_sha256"] = identity
        self.assertEqual((root / "crates/tje-simulation/src/ownership_generated.rs").read_text(),
                         ownership.render_rust(document))


if __name__ == "__main__":
    unittest.main()
