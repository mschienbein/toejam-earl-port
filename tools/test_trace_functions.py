"""ROM-free traceability tests using synthetic Rust metadata and modules."""

from __future__ import annotations

import contextlib
import io
import tempfile
import textwrap
import unittest
from dataclasses import replace
from pathlib import Path
from unittest import mock

import trace_functions as trace


class TraceFunctionsTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.put("crates/tje-game/src/lib.rs", "mod metadata; pub mod core;")
        self.put("crates/tje-game/src/metadata/mod.rs", "pub mod ids; mod part_0;")
        constants = ("LIVE", "CANDIDATE", "SOURCE_ONLY", "MISSING", "ORPHAN", "SHARED_GROUP")
        self.put("crates/tje-game/src/metadata/ids.rs", "\n".join(
            f"pub const {constant}: FunctionId = FunctionId({index});"
            for index, constant in enumerate(constants)
        ))
        classifications = ("compiled_c", "handwritten_asm", "not_reproducible_c",
                           "runtime_helper", "compiled_c", "handwritten_asm")
        names = ("CompiledLive", "WrittenAsm", "SourceOnly", "MissingHelper", "Orphan", "SharedGroup")
        entries = []
        for index, (name, classification) in enumerate(zip(names, classifications)):
            entries.append(f'''FunctionMetadata {{
                id: FunctionId({index}), inventory_id: "synthetic_{index}",
                name: "{name}", classification: "{classification}",
                kind: "{'fragment' if index == 5 else 'function'}", subsystem: "core",
                source: "src/core/example_{index}.c", source_line: {index + 10},
                addresses: [0x{0x1000 + index * 16:06X}, 0x{0x2000 + index * 16:06X}],
            }}''')
        self.put("crates/tje-game/src/metadata/part_0.rs",
                 "pub static FUNCTIONS: [FunctionMetadata; 6] = [" + ",".join(entries) + "];")
        self.module = textwrap.dedent('''
            use crate::{Context, Result, ids};
            mod body;
            mod contracts;
            pub use body::{live_body as live_alias};
            pub use body::*;
            pub use contracts::EXECUTION_CONTRACTS;
            pub static TRANSLATIONS: &[Translation] = &[
                Translation { id: ids::LIVE, function: live_alias },
            ];
            pub static PROVEN_REPLAYED: &[Translation] = &[];
            pub static PROVEN_REPLAYED_FRAMED: &[FramedTranslation] = &[];
            /// Shared pending-list blocker.
            pub static PENDING: &[FunctionId] = &[
                ids::CANDIDATE, // Specific proof <failure> | trace mismatch.
                // More detail on that same blocker.
                // The following standalone group starts after a blank line.

                // Source-only group blocker.
                ids::SOURCE_ONLY,
                ids::SHARED_GROUP,
                // No body: an unwritten stack-frame prerequisite.
                ids::MISSING,
            ];
            /// Explicit candidate registry awaiting proof.
            pub static PENDING_TRANSLATIONS: &[Translation] = &[
                Translation { id: ids::CANDIDATE, function: body::candidate },
                Translation { id: ids::SHARED_GROUP, function: crate::core::group_body },
            ];
            pub static ATOMIC_CAPABLE: &[FunctionId] = &[ids::LIVE, ids::ORPHAN];
            // pub static TRANSLATIONS: &[Translation] = &[ids::MISSING];
            const NOT_A_REGISTRY: &str = "pub static PENDING: &[FunctionId] = &[ids::MISSING];";
            #[cfg(test)]
            #[path = "not_present_test_module.rs"]
            mod tests;
            #[cfg(test)]
            mod inline_tests {
                pub static TRANSLATIONS: &[Translation] = &[ids::MISSING];
            }
        ''')
        self.put("crates/tje-game/src/core/mod.rs", self.module)
        self.contracts = textwrap.dedent('''
            pub static EXECUTION_CONTRACTS: &[ExecutionContract] = &[
                ExecutionContract { function: ids::CANDIDATE, class: ExecutionClass::Functional,
                    proof_envelope: ProofEnvelope::ClassDefined, operational_ceiling: None, evidence: None },
                ExecutionContract { function: ids::SOURCE_ONLY, class: ExecutionClass::TimeAware,
                    proof_envelope: ProofEnvelope::ClassDefined, operational_ceiling: None, evidence: None },
                ExecutionContract { function: ids::SHARED_GROUP, class: ExecutionClass::Functional,
                    proof_envelope: ProofEnvelope::ClassDefined, operational_ceiling: None, evidence: None },
            ];
        ''')
        self.put("crates/tje-game/src/core/contracts.rs", self.contracts)
        self.body = textwrap.dedent('''
            pub fn live_body(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
            pub fn candidate(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
            /// G/src/core/example_2.c:12 (`SourceOnly`).
            pub fn different_rust_name(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
            /// G/src/core/example_4.c:14 (`Orphan`).
            pub fn unregistered(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
            pub fn group_body(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
            /// `MissingHelper` documents a helper, not an ABI entry.
            pub fn not_an_entry(value: u32) -> u32 { value }
        ''')
        self.put("crates/tje-game/src/core/body.rs", self.body)
        (self.root / "docs").mkdir()

    def put(self, path: str, value: str) -> None:
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(textwrap.dedent(value), encoding="utf-8")

    def rows(self) -> list[trace.TraceRow]:
        return trace.Sources(self.root).rows(expected_count=6)

    def register_replayed_candidate(self, *, framed: bool = False) -> str:
        source = self.module.replace(
            "ids::CANDIDATE, // Specific proof <failure> | trace mismatch.",
            "// Candidate moved out of pending after proof.",
        ).replace("Translation { id: ids::CANDIDATE, function: body::candidate },", "")
        table = "PROVEN_REPLAYED_FRAMED" if framed else "PROVEN_REPLAYED"
        entry_type = "FramedTranslation" if framed else "Translation"
        source = source.replace(
            f"pub static {table}: &[{entry_type}] = &[];",
            f"pub static {table}: &[{entry_type}] = &["
            f"{entry_type} {{ id: ids::CANDIDATE, function: body::candidate }}];",
        )
        self.put("crates/tje-game/src/core/mod.rs", source)
        self.put("crates/tje-game/src/core/contracts.rs", self.contracts.replace(
            "ProofEnvelope::ClassDefined", "ProofEnvelope::ReplayedEnvironment", 1))
        if framed:
            self.put("crates/tje-game/src/core/body.rs", self.body.replace(
                "pub fn candidate(context: &mut dyn Context, _: &[u32])",
                "pub fn candidate(context: &mut dyn Context)",
            ))
        return source

    def test_replayed_value_and_framed_status_counts_and_source_locations(self) -> None:
        for framed in [False, True]:
            with self.subTest(framed=framed):
                self.register_replayed_candidate(framed=framed)
                rows = self.rows()
                self.assertEqual(rows[1].status, "proven (replayed environment)")
                self.assertEqual(rows[1].execution_class, "Functional")
                self.assertEqual(rows[1].rust.qualified, "core::body::candidate")
                self.assertEqual(rows[1].rust.path, Path("crates/tje-game/src/core/body.rs"))
                self.assertEqual(rows[1].blocker, "")
                document = trace.render(rows)
                self.assertIn("| [core](#core) | 6 | 1 | 1 | 3 | 1 | 2 | 2 | 1 | 1 |", document)
                self.assertIn("| **Total** | **6** | **1** | **1** | **3** | **1**", document)
                self.assertIn("| proven (replayed environment) |", document)
                self.assertIn("callees substituted (proven separately)", document)
                self.assertEqual(sum(row.status == "production" for row in rows), 1)

    def test_replayed_envelope_declaration_alone_does_not_prove_or_reclassify(self) -> None:
        self.put("crates/tje-game/src/core/contracts.rs", self.contracts.replace(
            "ProofEnvelope::ClassDefined", "ProofEnvelope::ReplayedEnvironment"))
        self.assertEqual([row.status for row in self.rows()], [
            "production", "pending", "pending", "not yet written", "pending", "pending",
        ])
        self.assertEqual(self.rows()[1].execution_class, "Functional")

    def test_production_body_rejects_replayed_contract_without_proven_marker(self) -> None:
        self.put("crates/tje-game/src/core/contracts.rs", self.contracts.replace(
            "pub static EXECUTION_CONTRACTS: &[ExecutionContract] = &[",
            "pub static EXECUTION_CONTRACTS: &[ExecutionContract] = &["
            "ExecutionContract { function: ids::LIVE, class: ExecutionClass::Atomic,"
            "proof_envelope: ProofEnvelope::ReplayedEnvironment, operational_ceiling: None, evidence: None },",
        ))
        for table in ["TRANSLATIONS", "FRAMED_TRANSLATIONS"]:
            with self.subTest(table=table):
                source = self.module.replace(
                    "&[ids::LIVE, ids::ORPHAN]", "&[ids::ORPHAN]")
                if table == "FRAMED_TRANSLATIONS":
                    source = source.replace(
                        "pub static TRANSLATIONS: &[Translation]",
                        "pub static FRAMED_TRANSLATIONS: &[FramedTranslation]",
                    )
                self.put("crates/tje-game/src/core/mod.rs", source)
                with self.assertRaisesRegex(trace.TraceError, "production body declares check-only"):
                    self.rows()

    def test_replayed_requires_a_unique_declared_replayed_contract(self) -> None:
        self.register_replayed_candidate()
        for contracts, message in [
            (self.contracts, "needs a ReplayedEnvironment contract"),
            ("pub static EXECUTION_CONTRACTS: &[ExecutionContract] = &[];",
             "needs a ReplayedEnvironment contract"),
            (self.contracts.replace("function: ids::SOURCE_ONLY", "function: ids::CANDIDATE"),
             "duplicate execution classification"),
        ]:
            with self.subTest(message=message):
                self.put("crates/tje-game/src/core/contracts.rs", contracts)
                with self.assertRaisesRegex(trace.TraceError, message):
                    self.rows()

    def test_replayed_rejects_all_production_and_pending_body_forms(self) -> None:
        source = self.register_replayed_candidate()
        for table, message in [
            ("TRANSLATIONS", "production/replayed overlap"),
            ("FRAMED_TRANSLATIONS", "production/replayed overlap"),
            ("PENDING_TRANSLATIONS", "pending/replayed overlap"),
            ("PENDING_FRAMED_TRANSLATIONS", "pending/replayed overlap"),
        ]:
            with self.subTest(table=table):
                entry_type = "FramedTranslation" if "FRAMED" in table else "Translation"
                registration = f"{entry_type} {{ id: ids::CANDIDATE, function: body::candidate }}"
                if f"pub static {table}:" in source:
                    changed = source.replace(
                        f"pub static {table}: &[{entry_type}] = &[",
                        f"pub static {table}: &[{entry_type}] = &[{registration},",
                    )
                else:
                    changed = source + f"pub static {table}: &[{entry_type}] = &[{registration}];"
                self.put("crates/tje-game/src/core/mod.rs", changed)
                with self.assertRaisesRegex(trace.TraceError, message):
                    self.rows()
        self.put("crates/tje-game/src/core/mod.rs", source.replace(
            "// Candidate moved out of pending after proof.", "ids::CANDIDATE,"))
        with self.assertRaisesRegex(trace.TraceError, "pending/replayed overlap"):
            self.rows()

    def test_replayed_duplicate_same_and_cross_form_bodies_fail_closed(self) -> None:
        source = self.register_replayed_candidate()
        body = "Translation { id: ids::CANDIDATE, function: body::candidate }"
        for changed in [
            source.replace(body, f"{body}, {body}"),
            source.replace(
                "pub static PROVEN_REPLAYED_FRAMED: &[FramedTranslation] = &[];",
                "pub static PROVEN_REPLAYED_FRAMED: &[FramedTranslation] = &["
                "FramedTranslation { id: ids::CANDIDATE, function: body::candidate }];",
            ),
        ]:
            with self.subTest(source=changed):
                self.put("crates/tje-game/src/core/mod.rs", changed)
                with self.assertRaisesRegex(trace.TraceError, "duplicate translation registration"):
                    self.rows()

    def test_replayed_missing_and_id_only_body_registrations_fail_closed(self) -> None:
        source = self.register_replayed_candidate()
        for changed, message in [
            (source.replace("function: body::candidate", "function: missing"),
             "expected one function"),
            (source.replace("Translation { id: ids::CANDIDATE, function: body::candidate }",
                            "ids::CANDIDATE"),
             "requires literal body registrations"),
            (source.replace("id: ids::CANDIDATE", "id: ids::UNKNOWN"),
             "uncatalogued registry identity"),
        ]:
            with self.subTest(message=message):
                self.put("crates/tje-game/src/core/mod.rs", changed)
                with self.assertRaisesRegex(trace.TraceError, message):
                    self.rows()

    def test_replayed_tables_must_exist_and_be_literal_in_each_owner(self) -> None:
        for table, entry_type in [
            ("PROVEN_REPLAYED", "Translation"),
            ("PROVEN_REPLAYED_FRAMED", "FramedTranslation"),
        ]:
            declaration = f"pub static {table}: &[{entry_type}] = &[];"
            for replacement, message in [
                ("", "missing literal replayed registry"),
                (declaration + declaration, "duplicate replayed registry table"),
                (f"pub static {table}: &[{entry_type}] = body::OTHER;", "literal registry array"),
                (f"pub static {table}: &[{entry_type}] = join!();", "literal registry array"),
            ]:
                with self.subTest(table=table, replacement=replacement):
                    self.put("crates/tje-game/src/core/mod.rs", self.module.replace(
                        declaration, replacement))
                    with self.assertRaisesRegex(trace.TraceError, message):
                        self.rows()
        self.put("crates/tje-game/src/core/mod.rs", self.module)
        self.put("crates/tje-game/src/core/body.rs", self.body + '''
            pub static PROVEN_REPLAYED: &[Translation] = &[];
        ''')
        with self.assertRaisesRegex(trace.TraceError, "must be in its subsystem module"):
            self.rows()

    def test_replayed_body_and_execution_contracts_reject_wrong_owner(self) -> None:
        source = self.register_replayed_candidate()
        part = self.root / "crates/tje-game/src/metadata/part_0.rs"
        metadata = part.read_text(encoding="utf-8")
        changed = metadata.replace('subsystem: "core"', 'subsystem: "other"', 2).replace(
            'subsystem: "other"', 'subsystem: "core"', 1)
        self.put("crates/tje-game/src/metadata/part_0.rs", changed)
        with self.assertRaisesRegex(trace.TraceError, "wrong replayed registry owner"):
            self.rows()
        self.put("crates/tje-game/src/core/mod.rs", source.replace(
            "Translation { id: ids::CANDIDATE, function: body::candidate }", ""))
        with self.assertRaisesRegex(trace.TraceError, "wrong execution classification owner"):
            self.rows()

    def test_registry_kinds_addresses_execution_and_source_locations(self) -> None:
        rows = self.rows()
        self.assertEqual(len(rows), 6)
        self.assertEqual([row.status for row in rows], [
            "production", "pending", "pending", "not yet written", "pending", "pending",
        ])
        self.assertEqual([row.entry.kind for row in rows], [
            "compiled C", "hand-written asm", "collision query", "runtime helper",
            "compiled C", "hand-written asm",
        ])
        self.assertEqual([row.execution_class for row in rows], [
            "Atomic", "Functional", "TimeAware", "unclassified", "Atomic", "Functional",
        ])
        self.assertEqual(rows[0].entry.addresses, (0x1000, 0x2000))
        self.assertEqual(rows[0].rust.qualified, "core::body::live_body")
        self.assertEqual(rows[0].rust.path, Path("crates/tje-game/src/core/body.rs"))
        self.assertEqual(rows[0].rust.line, 2)
        self.assertEqual(rows[2].rust.qualified, "core::body::different_rust_name")
        self.assertEqual(rows[5].rust.qualified, "core::body::group_body")
        self.assertIsNone(rows[3].rust)
        self.assertTrue(rows[5].entry.fragment)

    def test_literal_contract_envelope_does_not_reclassify_the_source(self) -> None:
        for envelope in ["ClockedStrict", "ClockedCommutingInterrupts", "ReplayedEnvironment"]:
            with self.subTest(envelope=envelope):
                self.put("crates/tje-game/src/core/contracts.rs", self.contracts.replace(
                    "ProofEnvelope::ClassDefined", f"ProofEnvelope::{envelope}"))
                rows = self.rows()
                self.assertEqual(rows[1].execution_class, "Functional")
                self.assertEqual(rows[2].execution_class, "TimeAware")
                self.assertEqual(rows[1].status, "pending")

    def test_legacy_tuple_missing_policy_and_unknown_envelope_fail_closed(self) -> None:
        for source, message in [
            ("pub static EXECUTION_CONTRACTS: &[(FunctionId, ExecutionClass)] = &[(ids::CANDIDATE, ExecutionClass::Functional)];",
             "literal ExecutionContract"),
            (self.contracts.replace(", evidence: None", ""), "all five policy fields"),
            (self.contracts.replace("ProofEnvelope::ClassDefined", "ProofEnvelope::Unknown"),
             "unknown proof envelope"),
            (self.contracts.replace("ExecutionClass::Functional", "ExecutionClass::Atomic"),
             "plain Atomic declarations"),
        ]:
            with self.subTest(message=message):
                self.put("crates/tje-game/src/core/contracts.rs", source)
                with self.assertRaisesRegex(trace.TraceError, message):
                    self.rows()

    def test_pending_comments_groups_and_absent_comment_are_honest(self) -> None:
        rows = self.rows()
        self.assertIn("Specific proof <failure> | trace mismatch.", rows[1].blocker)
        self.assertIn("More detail on that same blocker.", rows[1].blocker)
        self.assertNotIn("Specific proof", rows[2].blocker)
        self.assertEqual(rows[2].blocker, "Source-only group blocker.")
        self.assertIn("Source-only group blocker.", rows[5].blocker)
        self.assertIn("No body: an unwritten stack-frame prerequisite.", rows[3].blocker)
        self.assertIn("no blocker comment recorded", rows[4].blocker)
        self.assertEqual(rows[0].blocker, "")

    def test_render_has_exactly_one_row_per_entry_and_subsystem_totals(self) -> None:
        rows = self.rows()
        rows[-1] = replace(rows[-1], entry=replace(rows[-1].entry, subsystem="second"))
        document = trace.render(rows)
        table_rows = [line for line in document.splitlines() if line.startswith("| `")]
        self.assertEqual(len(table_rows), 6)
        for row in rows:
            self.assertEqual(document.count(f"| `{row.entry.name}` |"), 1)
        self.assertIn("| [core](#core) | 5 | 1 | 0 | 3 | 1 | 2 | 1 | 1 | 1 |", document)
        self.assertIn("| [second](#second) | 1 | 0 | 0 | 1 | 0 | 0 | 1 | 0 | 0 |", document)
        self.assertIn("| **Total** | **6** | **1** | **0** | **4** | **1**", document)
        self.assertIn("&lt;failure&gt; \\| trace mismatch.", document)
        self.assertIn("`0x001000` | `0x002000`", document)
        self.assertIn("#L10)", document)
        self.assertEqual(document, trace.render(rows))
        self.assertTrue(document.endswith("\n"))

    def test_source_line_provenance_does_not_require_original_name(self) -> None:
        self.put("crates/tje-game/src/core/body.rs", self.body.replace(" (`SourceOnly`)", ""))
        self.assertEqual(self.rows()[2].rust.qualified, "core::body::different_rust_name")

    def test_path_attribute_in_non_mod_rs_file_is_relative_to_its_directory(self) -> None:
        candidate = "pub fn candidate(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }\n"
        self.put("crates/tje-game/src/core/body.rs", self.body.replace(candidate, "") + textwrap.dedent('''
            #[path = "body_parts/candidate.rs"]
            mod candidate_part;
            pub use candidate_part::candidate;
        '''))
        self.put("crates/tje-game/src/core/body_parts/candidate.rs", "use crate::{Context, Result};\n" + candidate)
        rust = self.rows()[1].rust
        self.assertEqual(rust.qualified, "core::body::candidate_part::candidate")
        self.assertEqual(rust.path, Path("crates/tje-game/src/core/body_parts/candidate.rs"))

    def test_unlinked_and_test_only_functions_do_not_count(self) -> None:
        self.put("crates/tje-game/src/core/unused.rs", '''
            /// G/src/core/example_3.c:13 (`MissingHelper`).
            pub fn unused(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
        ''')
        self.assertEqual(self.rows()[3].status, "not yet written")

    def test_duplicate_catalogue_identity_and_count_fail_closed(self) -> None:
        with self.assertRaisesRegex(trace.TraceError, "exactly 648"):
            trace.Sources(self.root).rows()
        self.put("crates/tje-game/src/metadata/ids.rs", '''
            pub const LIVE: FunctionId = FunctionId(0);
            pub const COLLIDING_ID: FunctionId = FunctionId(0);
        ''')
        with self.assertRaisesRegex(trace.TraceError, "duplicate metadata numeric identity"):
            self.rows()

    def test_production_pending_overlap_and_unknown_ids_fail_closed(self) -> None:
        self.put("crates/tje-game/src/core/mod.rs", self.module.replace(
            "ids::CANDIDATE, // Specific", "ids::LIVE, // Specific"))
        with self.assertRaisesRegex(trace.TraceError, "production/pending overlap"):
            self.rows()
        self.put("crates/tje-game/src/core/mod.rs", self.module.replace("id: ids::LIVE", "id: ids::UNKNOWN"))
        with self.assertRaisesRegex(trace.TraceError, "uncatalogued registry identity"):
            self.rows()

    def test_framed_registries_count_ctx_only_bodies_and_reject_cross_form_duplicates(self) -> None:
        framed = self.module.replace(
            "pub static TRANSLATIONS: &[Translation]",
            "pub static FRAMED_TRANSLATIONS: &[FramedTranslation]",
        ).replace(
            "pub static PENDING_TRANSLATIONS: &[Translation]",
            "pub static PENDING_FRAMED_TRANSLATIONS: &[FramedTranslation]",
        )
        self.put("crates/tje-game/src/core/mod.rs", framed)
        self.put("crates/tje-game/src/core/body.rs", self.body.replace(", _: &[u32]", ""))
        rows = self.rows()
        self.assertEqual(rows[0].status, "production")
        self.assertEqual(rows[1].status, "pending")
        self.assertEqual(rows[0].rust.qualified, "core::body::live_body")
        self.assertEqual(rows[1].rust.qualified, "core::body::candidate")
        for table in ["TRANSLATIONS", "PENDING_TRANSLATIONS"]:
            with self.subTest(table=table):
                self.put("crates/tje-game/src/core/mod.rs", framed + f'''
                    pub static {table}: &[Translation] = &[
                        Translation {{ id: ids::LIVE, function: live_alias }},
                    ];
                ''')
                message = "duplicate translation registration" if table == "TRANSLATIONS" else "production/pending overlap"
                with self.assertRaisesRegex(trace.TraceError, message):
                    self.rows()
        self.put("crates/tje-game/src/core/mod.rs", framed + '''
            pub static PENDING_TRANSLATIONS: &[Translation] = &[
                Translation { id: ids::CANDIDATE, function: body::candidate },
            ];
        ''')
        with self.assertRaisesRegex(trace.TraceError, "duplicate translation registration"):
            self.rows()

    def test_missing_registered_body_and_ambiguous_provenance_fail_closed(self) -> None:
        self.put("crates/tje-game/src/core/mod.rs", self.module.replace("function: live_alias", "function: absent"))
        with self.assertRaisesRegex(trace.TraceError, "expected one function"):
            self.rows()
        self.put("crates/tje-game/src/core/mod.rs", self.module)
        self.put("crates/tje-game/src/core/body.rs", self.body + '''
            /// G/src/core/example_2.c:12 (`SourceOnly`).
            pub fn second_candidate(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> { Ok(None) }
        ''')
        with self.assertRaisesRegex(trace.TraceError, "ambiguous source-only translation"):
            self.rows()

    def test_duplicate_execution_class_is_not_silently_atomic(self) -> None:
        self.put("crates/tje-game/src/core/contracts.rs", '''
            pub static EXECUTION_CONTRACTS: &[ExecutionContract] = &[
                ExecutionContract { function: ids::LIVE, class: ExecutionClass::TimeAware,
                    proof_envelope: ProofEnvelope::ClassDefined, operational_ceiling: None, evidence: None },
            ];
        ''')
        with self.assertRaisesRegex(trace.TraceError, "duplicate execution classification"):
            self.rows()

    def test_generate_check_and_stale_missing_document_exit_codes(self) -> None:
        expected = trace.generate(self.root, expected_count=6)
        stderr = io.StringIO()
        stdout = io.StringIO()
        with (mock.patch.object(trace, "generate", return_value=expected),
              contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout)):
            self.assertEqual(trace.main(["--check"], root=self.root), 1)
            self.assertIn("is stale", stderr.getvalue())
            self.assertEqual(trace.main([], root=self.root), 0)
            destination = self.root / "docs/FUNCTIONS.md"
            self.assertEqual(destination.read_text(encoding="utf-8"), expected)
            self.assertEqual(trace.main(["--check"], root=self.root), 0)
            destination.write_text("stale document\n", encoding="utf-8")
            self.assertEqual(trace.main(["--check"], root=self.root), 1)
            self.assertEqual(destination.read_text(encoding="utf-8"), "stale document\n")

    def test_invalid_source_reports_error_without_overwriting_document(self) -> None:
        self.put("docs/FUNCTIONS.md", "existing document\n")
        with (mock.patch.object(trace, "generate", side_effect=trace.TraceError("synthetic source error")),
              contextlib.redirect_stderr(io.StringIO())):
            self.assertEqual(trace.main([], root=self.root), 2)
        self.assertEqual((self.root / "docs/FUNCTIONS.md").read_text(), "existing document\n")


if __name__ == "__main__":
    unittest.main()
