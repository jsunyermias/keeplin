from __future__ import annotations

import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "companion_tool.py"
SPEC = importlib.util.spec_from_file_location("companion_tool", SCRIPT)
assert SPEC and SPEC.loader
TOOL = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = TOOL
SPEC.loader.exec_module(TOOL)
FIXTURES = Path(__file__).parent / "fixtures"


def fixture_text(name: str) -> str:
    return (FIXTURES / name).read_text(encoding="utf-8")


class CompanionToolTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.source = self.root / "shapes.rs"
        self.companion = self.root / "shapes.md"
        shutil.copyfile(FIXTURES / "shapes.rs.fixture", self.source)
        shutil.copyfile(FIXTURES / "shapes.md.fixture", self.companion)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def errors(self) -> list[str]:
        return TOOL.verify_pair(self.source, self.companion)

    def test_fixture_covers_functions_impl_attributes_and_containers(self) -> None:
        self.assertEqual(self.errors(), [])

    def test_one_source_line_change_is_stale(self) -> None:
        text = self.source.read_text(encoding="utf-8").replace("format!(\"{value:?}\")", "format!(\"value={value:?}\")")
        self.source.write_text(text, encoding="utf-8")
        self.assertTrue(any("STALE/TRUNCATED" in error for error in self.errors()))

    def test_old_and_truncated_fences_are_detected(self) -> None:
        text = self.companion.read_text(encoding="utf-8")
        text = text.replace("        2\n", "")
        self.companion.write_text(text, encoding="utf-8")
        self.assertTrue(any("STALE/TRUNCATED" in error for error in self.errors()))

    def test_orphan_fence_is_detected(self) -> None:
        with self.companion.open("a", encoding="utf-8") as handle:
            handle.write("\n```rust\n// md:fn absent\nfn absent() {}\n```\n")
        self.assertTrue(any("ORPHAN fence" in error for error in self.errors()))

    def test_duplicate_fence_is_detected(self) -> None:
        text = self.companion.read_text(encoding="utf-8")
        fence = "```rust\n// md:Overview\nuse std::fmt::Debug;\n```"
        self.companion.write_text(text + "\n" + fence + "\n", encoding="utf-8")
        self.assertTrue(any("DUPLICATE fence" in error for error in self.errors()))

    def test_duplicate_source_marker_is_detected(self) -> None:
        with self.source.open("a", encoding="utf-8") as handle:
            handle.write("\n// md:fn labelled\nfn duplicate() {}\n")
        self.assertTrue(any("duplicate source marker" in error for error in self.errors()))

    def test_code_before_first_container_child_is_uncovered(self) -> None:
        text = self.source.read_text(encoding="utf-8").replace(
            "impl Container {\n    // md:impl Container > fn value",
            "impl Container {\n    fn hidden() {}\n\n    // md:impl Container > fn value",
        )
        self.source.write_text(text, encoding="utf-8")
        self.assertTrue(any("UNCOVERED" in error for error in self.errors()))

    def test_only_initial_scaffolding_is_allowed_before_first_marker(self) -> None:
        path = self.root / "leading.rs"
        path.write_text(fixture_text("leading_scaffolding.rs.fixture"), encoding="utf-8")
        self.assertEqual([block.name for block in TOOL.parse_source(path)], ["Overview"])

    def test_real_code_before_first_marker_is_uncovered(self) -> None:
        path = self.root / "leading.rs"
        path.write_text(fixture_text("leading_uncovered.rs.fixture"), encoding="utf-8")
        with self.assertRaisesRegex(TOOL.CompanionError, "UNCOVERED code before first"):
            TOOL.parse_source(path)

    def test_type_definition_container_names_the_rule_it_breaks(self) -> None:
        with self.source.open("a", encoding="utf-8") as handle:
            handle.write("\n// md:enum Kind\npub enum Kind {\n    // md:enum Kind > A\n    A,\n}\n")
        self.assertTrue(any("must be an impl, mod or trait" in error for error in self.errors()))

    def test_reordered_fences_are_detected(self) -> None:
        text = self.companion.read_text(encoding="utf-8")
        overview = "```rust\n// md:Overview\nuse std::fmt::Debug;\n```"
        labelled = (
            "```rust\n// md:fn labelled\n#[inline]\nfn labelled<T: Debug>(value: T) -> String {\n"
            "    format!(\"{value:?}\")\n}\n```"
        )
        text = text.replace(overview, "SWAP", 1).replace(labelled, overview, 1).replace("SWAP", labelled, 1)
        self.companion.write_text(text, encoding="utf-8")
        self.assertTrue(any("REORDERED" in error for error in self.errors()))

    def test_check_mode_is_repeatable_and_read_only(self) -> None:
        before = self.companion.read_bytes()
        first = TOOL.sync(self.root, ["shapes.rs"], check=True)
        second = TOOL.sync(self.root, ["shapes.rs"], check=True)
        self.assertEqual(first, second)
        self.assertEqual(self.companion.read_bytes(), before)

    def test_sync_changes_only_fence_content(self) -> None:
        prose = "Human prose must survive exactly."
        with self.companion.open("a", encoding="utf-8") as handle:
            handle.write("\n" + prose + "\n")
        text = self.source.read_text(encoding="utf-8").replace("        2\n", "        3\n")
        self.source.write_text(text, encoding="utf-8")
        checked, changed, errors = TOOL.sync(self.root, ["shapes.rs"], check=False)
        self.assertEqual((checked, changed, errors), (1, 1, []))
        self.assertIn(prose, self.companion.read_text(encoding="utf-8"))
        self.assertEqual(self.errors(), [])

    def test_sync_preserves_mixed_eol_bytes_outside_valid_fence(self) -> None:
        case = json.loads(fixture_text("mixed_eol_sync.json.fixture"))
        self.source.write_bytes(case["source"].encode("utf-8"))
        self.companion.write_bytes(case["companion"].encode("utf-8"))
        checked, changed, errors = TOOL.sync(self.root, ["shapes.rs"], check=False)
        self.assertEqual((checked, changed, errors), (1, 1, []))
        self.assertEqual(self.companion.read_bytes(), case["expected"].encode("utf-8"))
        first = self.companion.read_bytes()
        self.assertEqual(TOOL.sync(self.root, ["shapes.rs"], check=False), (1, 0, []))
        self.assertEqual(self.companion.read_bytes(), first)

    def test_sync_refuses_invalid_mixed_eol_fence_without_writing(self) -> None:
        case = json.loads(fixture_text("mixed_eol_invalid.json.fixture"))
        self.source.write_bytes(case["source"].encode("utf-8"))
        self.companion.write_bytes(case["companion"].encode("utf-8"))
        before = self.companion.read_bytes()
        checked, changed, errors = TOOL.sync(self.root, ["shapes.rs"], check=False)
        self.assertEqual((checked, changed), (1, 0))
        self.assertTrue(any("ORPHAN" in error for error in errors))
        self.assertEqual(self.companion.read_bytes(), before)

    def test_shell_markers_and_bash_fences_are_exact(self) -> None:
        source = self.root / "script.sh"
        companion = self.root / "script.sh.md"
        shutil.copyfile(FIXTURES / "shell_valid.sh.fixture", source)
        shutil.copyfile(FIXTURES / "shell_valid.md.fixture", companion)
        self.assertEqual(TOOL.source_kind(source), "shell")
        self.assertEqual(TOOL.verify_pair(source, companion), [])
        self.assertEqual([block.name for block in TOOL.parse_source(source)], ["setup", "run"])

    def test_extensionless_shell_is_supported_but_yaml_is_not(self) -> None:
        launcher = self.root / "launcher"
        shutil.copyfile(FIXTURES / "shell_valid.sh.fixture", launcher)
        shutil.copyfile(FIXTURES / "shell_valid.md.fixture", Path(str(launcher) + ".md"))
        yaml = self.root / "config.yml"
        shutil.copyfile(FIXTURES / "unsupported.yml.fixture", yaml)
        (self.root / "config.yml.md").write_text("# prose\n", encoding="utf-8")
        self.assertEqual(TOOL.source_kind(launcher), "shell")
        self.assertIsNone(TOOL.source_kind(yaml))
        discovered = TOOL.iter_sources(self.root)
        self.assertIn(launcher, discovered)
        self.assertNotIn(yaml, discovered)

    def test_shell_code_before_first_marker_is_uncovered(self) -> None:
        source = self.root / "script.sh"
        source.write_text(fixture_text("shell_uncovered.sh.fixture"), encoding="utf-8")
        with self.assertRaisesRegex(TOOL.CompanionError, "UNCOVERED code before first '# md:'"):
            TOOL.parse_source(source)

    def test_shell_sync_changes_only_bash_fence_body(self) -> None:
        source = self.root / "script.sh"
        companion = self.root / "script.sh.md"
        shutil.copyfile(FIXTURES / "shell_valid.sh.fixture", source)
        text = fixture_text("shell_valid.md.fixture").replace('echo "ready"', 'echo "old"')
        companion.write_text("Prose stays.\n\n" + text, encoding="utf-8")
        checked, changed, errors = TOOL.sync(self.root, ["script.sh"], check=False)
        self.assertEqual((checked, changed, errors), (1, 1, []))
        self.assertTrue(companion.read_text(encoding="utf-8").startswith("Prose stays.\n"))
        self.assertEqual(TOOL.verify_pair(source, companion), [])

    def test_shell_illustrative_bash_fence_is_not_a_fidelity_block(self) -> None:
        source = self.root / "script.sh"
        companion = self.root / "script.sh.md"
        shutil.copyfile(FIXTURES / "shell_valid.sh.fixture", source)
        text = fixture_text("shell_valid.md.fixture")
        companion.write_text(text + '\n```bash\necho "example only"\n```\n', encoding="utf-8")
        self.assertEqual(TOOL.verify_pair(source, companion), [])

    def test_sql_requires_one_complete_verbatim_fence(self) -> None:
        source = self.root / "migration.sql"
        companion = self.root / "migration.md"
        shutil.copyfile(FIXTURES / "sql_complete.sql.fixture", source)
        shutil.copyfile(FIXTURES / "sql_complete.md.fixture", companion)
        self.assertEqual(TOOL.source_kind(source), "sql")
        self.assertEqual(TOOL.verify_pair(source, companion), [])

    def test_sql_stale_fence_is_detected_and_synced_without_source_write(self) -> None:
        source = self.root / "migration.sql"
        companion = self.root / "migration.md"
        shutil.copyfile(FIXTURES / "sql_complete.sql.fixture", source)
        shutil.copyfile(FIXTURES / "sql_stale.md.fixture", companion)
        source_before = source.read_bytes()
        self.assertTrue(any("STALE/TRUNCATED" in error for error in TOOL.verify_pair(source, companion)))
        checked, changed, errors = TOOL.sync(self.root, ["migration.sql"], check=False)
        self.assertEqual((checked, changed, errors), (1, 1, []))
        self.assertEqual(source.read_bytes(), source_before)
        self.assertEqual(TOOL.verify_pair(source, companion), [])

    def test_sql_markers_are_forbidden_and_companion_is_required(self) -> None:
        source = self.root / "migration.sql"
        companion = self.root / "migration.md"
        source.write_text("-- md:never\nSELECT 1;\n", encoding="utf-8")
        shutil.copyfile(FIXTURES / "sql_complete.md.fixture", companion)
        self.assertTrue(any("FORBIDDEN '-- md:'" in error for error in TOOL.verify_pair(source, companion)))
        companion.unlink()
        checked, changed, errors = TOOL.sync(self.root, ["migration.sql"], check=True)
        self.assertEqual((checked, changed), (0, 0))
        self.assertTrue(any("MISSING companion doc" in error for error in errors))

    def test_sql_rejects_multiple_fences(self) -> None:
        source = self.root / "migration.sql"
        companion = self.root / "migration.md"
        shutil.copyfile(FIXTURES / "sql_complete.sql.fixture", source)
        text = fixture_text("sql_complete.md.fixture")
        companion.write_text(text + "\n```sql\nSELECT 2;\n```\n", encoding="utf-8")
        self.assertTrue(
            any("DUPLICATE sql fences" in error for error in TOOL.verify_pair(source, companion))
        )

    def test_pack_is_byte_for_byte_reproducible(self) -> None:
        manifest = TOOL.build_manifest(self.root)
        self.assertEqual(len(manifest["entries"]), 1)
        first = self.root / "first.zip"
        second = self.root / "second.zip"
        TOOL.build_pack(self.root, "shapes.rs", "understand", first, False, 100_000, 5)
        TOOL.build_pack(self.root, "shapes.rs", "understand", second, False, 100_000, 5)
        self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_manifest_recognizes_extracted_origin_with_details(self) -> None:
        with self.companion.open("a", encoding="utf-8") as handle:
            handle.write(
                "\n**Direct dependencies**\n\n"
                "- (EXTRACTED; local module) `shapes.rs`\n\n"
                "**Direct dependents**\n\n"
                "- (EXTRACTED: fixture link) `shapes.rs`\n\n"
                "**Invariants**\n\n"
                "- (EXTRACTED; source contract) markers remain ordered\n"
            )
        entry = TOOL.build_manifest(self.root)["entries"][0]
        self.assertEqual(entry["direct_dependencies"][0]["origin"], "EXTRACTED")
        self.assertEqual(entry["direct_dependents"][0]["origin"], "EXTRACTED")
        self.assertEqual(entry["invariants"]["value"][0]["origin"], "EXTRACTED")

    def test_cross_repo_contracts_are_explicit_deterministic_and_symmetric(self) -> None:
        left = TOOL._cross_repo_contracts(fixture_text("cross_repo_contracts.left.md.fixture"))
        right = TOOL._cross_repo_contracts(fixture_text("cross_repo_contracts.right.md.fixture"))
        self.assertEqual(left, ["collab-wire", "format-limits"])
        self.assertEqual(left, right)

    def test_contract_registry_detects_drift_in_both_directions(self) -> None:
        manifest = {
            "entries": [
                {"cross_repo_contracts": {"origin": "INFERRED", "value": ["collab-wire"]}},
                {"cross_repo_contracts": {"origin": "INFERRED", "value": ["protocol-version"]}},
            ]
        }
        registry = self.root / TOOL.CONTRACT_REGISTRY
        registry.parent.mkdir(parents=True, exist_ok=True)

        registry.write_text("# comment\n\ncollab-wire\nprotocol-version\n", encoding="utf-8")
        self.assertEqual(TOOL.verify_contract_registry(self.root, manifest), [])

        registry.write_text("collab-wire\n", encoding="utf-8")
        unpinned = TOOL.verify_contract_registry(self.root, manifest)
        self.assertEqual(len(unpinned), 1)
        self.assertIn("UNPINNED cross-repo contract 'protocol-version'", unpinned[0])

        registry.write_text("collab-wire\nprotocol-version\nresource-blobs\n", encoding="utf-8")
        missing = TOOL.verify_contract_registry(self.root, manifest)
        self.assertEqual(len(missing), 1)
        self.assertIn("MISSING cross-repo contract 'resource-blobs'", missing[0])

    def test_contract_registry_is_required(self) -> None:
        errors = TOOL.verify_contract_registry(self.root, {"entries": []})
        self.assertEqual(len(errors), 1)
        self.assertIn("MISSING cross-repo contract registry", errors[0])

    def test_cross_repo_contracts_ignore_incidental_prose(self) -> None:
        text = fixture_text("cross_repo_contracts.incidental.md.fixture")
        self.assertEqual(TOOL._cross_repo_contracts(text), [])

    def test_manifest_invariants_cover_authored_and_dependency_expectations(self) -> None:
        values = TOOL._manifest_invariants(fixture_text("invariants.explicit.md.fixture"))
        self.assertEqual(
            values,
            [
                {
                    "origin": "EXTRACTED",
                    "value": "(EXTRACTED; source contract) markers remain ordered",
                    "basis": "authored-invariant",
                },
                {
                    "origin": "INFERRED",
                    "value": "Authored ordering is stable across runs.",
                    "basis": "authored-invariant",
                },
                {
                    "origin": "INFERRED",
                    "value": "unknown input returns an error instead of a partial result.",
                    "basis": "dependency-expectation",
                },
            ],
        )

    def test_manifest_invariants_ignore_ambiguous_prose(self) -> None:
        text = fixture_text("invariants.ambiguous.md.fixture")
        self.assertEqual(TOOL._manifest_invariants(text), [])


if __name__ == "__main__":
    unittest.main()
