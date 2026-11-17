"""Unit tests of the parsing and assertion helpers of ``exercise_base.py``.

Run from the crate directory, without the ``rucene-base`` binary::

    python3 -I -m unittest discover -s scripts/tests -t scripts
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from typing import ClassVar

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import exercise_base as eb


class ParseTsvTest(unittest.TestCase):
    def test_rows_and_exact_total(self) -> None:
        text = (
            "# total hits: 2\n"
            "idcontrato\tadjudicanteNome\tsort:precoContratual:desc\tscore\n"
            "1\tA | B\t10.5\tNaN\n"
            "2\t\t\tNaN\n"
        )
        out = eb.parse_tsv(text)
        self.assertEqual(out.total, 2)
        self.assertTrue(out.exact)
        self.assertEqual(out.header[-1], "score")
        self.assertEqual(out.rows[0]["adjudicanteNome"], "A | B")
        self.assertEqual(out.rows[1]["sort:precoContratual:desc"], "")

    def test_lower_bound_and_no_rows(self) -> None:
        out = eb.parse_tsv("# total hits: 1000 (lower bound)\nnif\tscore\n")
        self.assertEqual(out.total, 1000)
        self.assertFalse(out.exact)
        self.assertEqual(out.rows, ())

    def test_rejects_bad_shapes(self) -> None:
        for text in (
            "",
            "# total hits: 1\n",
            "total: 1\nnif\tscore\n",
            "# total hits: 1\nnif\tvalor\n",
            "# total hits: 1\nnif\tscore\n1\t2\t3\n",
        ):
            with self.subTest(text=text), self.assertRaises(ValueError):
                eb.parse_tsv(text)


class ParseOtherOutputsTest(unittest.TestCase):
    def test_stats(self) -> None:
        text = (
            "index: contratos\ndirectory: /r/contratos\ndocuments: 5\n"
            "deleted: 1\nmax_doc: 6\nsegments: 2\n"
        )
        st = eb.parse_stats(text)
        self.assertEqual(st["documents"], "5")
        self.assertEqual(st["directory"], "/r/contratos")
        with self.assertRaises(ValueError):
            eb.parse_stats("index: contratos\n")
        with self.assertRaises(ValueError):
            eb.parse_stats("garbage\n")

    def test_get_and_count(self) -> None:
        docs = eb.parse_get('{"nif": "1", "nome": ["Á"]}\n\n{"nif": "2"}\n')
        self.assertEqual([d["nif"] for d in docs], ["1", "2"])
        self.assertEqual(docs[0]["nome"], ["Á"])
        with self.assertRaises(TypeError):
            eb.parse_get("[1]\n")
        self.assertEqual(eb.parse_count("42\n"), 42)
        with self.assertRaises(ValueError):
            eb.parse_count("-1\n")

    def test_cell_values_and_as_list(self) -> None:
        self.assertEqual(eb.cell_values(""), [])
        self.assertEqual(eb.cell_values("a | b"), ["a", "b"])
        self.assertEqual(eb.as_list(None), [])
        self.assertEqual(eb.as_list(3), ["3"])
        self.assertEqual(eb.as_list(["x", 1]), ["x", "1"])


class FoldingModelTest(unittest.TestCase):
    def test_fold_groups(self) -> None:
        self.assertEqual({eb.fold(w) for w in ("ação", "acao", "AÇÃO")}, {"acao"})
        self.assertEqual({eb.fold(w) for w in ("ações", "AÇÕES", "acoes")}, {"acoes"})
        self.assertEqual(eb.fold("PAPÉIS"), "papeis")
        self.assertEqual(eb.fold("Évora"), "evora")

    def test_folded_words(self) -> None:
        self.assertEqual(
            eb.folded_words("Manutenção de AÇÕES, n.º 3"),
            {"manutencao", "de", "acoes", "n", "o", "3"},
        )

    def test_model_matches(self) -> None:
        records = [
            {"id": "1", "a": "Aquisição de AÇÕES", "b": []},
            {"id": "2", "a": "nada", "b": ["Uma ação"]},
            {"id": "3", "a": "acções de formação", "b": None},
            {"id": "4", "a": "transação", "b": ["acaozinha"]},
        ]
        got = eb.model_matches(records, "id", ("a", "b"), {"ação", "acoes"})
        self.assertEqual(got, {"1", "2"})


class SortViolationTest(unittest.TestCase):
    SPEC: ClassVar[list[tuple[str, bool, bool]]] = [("p", True, True)]

    def rows(self, *values: str) -> list[dict[str, str]]:
        return [{"p": v} for v in values]

    def test_descending_numbers(self) -> None:
        self.assertIsNone(
            eb.sort_violation(self.rows("100", "20", "20", "3"), self.SPEC)
        )
        self.assertEqual(eb.sort_violation(self.rows("3", "20"), self.SPEC), 1)

    def test_numeric_not_lexicographic(self) -> None:
        self.assertIsNone(eb.sort_violation(self.rows("9", "10"), [("p", False, True)]))
        self.assertEqual(
            eb.sort_violation(self.rows("9", "10"), [("p", False, False)]), 1
        )

    def test_missing_values_last(self) -> None:
        self.assertIsNone(eb.sort_violation(self.rows("5", "", ""), self.SPEC))
        self.assertEqual(eb.sort_violation(self.rows("", "5"), self.SPEC), 1)
        self.assertEqual(eb.sort_violation(self.rows("", "5"), [("p", False, True)]), 1)

    def test_multi_key(self) -> None:
        spec = [("a", True, True), ("b", False, False)]
        good = [{"a": "2", "b": "x"}, {"a": "2", "b": "y"}, {"a": "1", "b": "a"}]
        bad = [{"a": "2", "b": "y"}, {"a": "2", "b": "x"}]
        self.assertIsNone(eb.sort_violation(good, spec))
        self.assertEqual(eb.sort_violation(bad, spec), 1)

    def test_dates_compare_as_text(self) -> None:
        spec = [("d", False, False)]
        self.assertIsNone(
            eb.sort_violation([{"d": "2025-01-31"}, {"d": "2025-02-01"}], spec)
        )


class CommandLineHelpersTest(unittest.TestCase):
    def test_parse_only(self) -> None:
        self.assertEqual(eb.parse_only(None), list(eb.ALL_SCENARIOS))
        self.assertEqual(eb.parse_only("write, accent"), ["accent", "write"])
        for bad in ("nope", "", ","):
            with self.subTest(bad=bad), self.assertRaises(eb.UsageError):
                eb.parse_only(bad)

    def test_shown(self) -> None:
        self.assertEqual(eb.shown(["a", "b c", "it's", ""]), "a 'b c' 'it'\\''s' ''")

    def test_scratch_location(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "root"
            root.mkdir()
            eb.check_scratch_location(Path(tmp) / "scratch", root)
            eb.check_scratch_location(Path(tmp) / "scratch", None)
            for bad in (root, root / "x", Path(tmp)):
                with self.subTest(bad=bad), self.assertRaises(eb.UsageError):
                    eb.check_scratch_location(bad, root)

    def test_prepare_scratch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scratch = Path(tmp) / "s"
            eb.prepare_scratch(scratch)
            self.assertTrue((scratch / eb.MARKER).is_file())
            (scratch / "old").write_text("x", encoding="utf-8")
            eb.prepare_scratch(scratch)
            self.assertEqual(sorted(p.name for p in scratch.iterdir()), [eb.MARKER])
            foreign = Path(tmp) / "f"
            foreign.mkdir()
            (foreign / "data").write_text("x", encoding="utf-8")
            with self.assertRaises(eb.UsageError):
                eb.prepare_scratch(foreign)
            self.assertTrue((foreign / "data").exists())

    def test_jsonl_round_trip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.jsonl"
            records = [{"a": "ação", "b": [1, 2]}, {"a": None}]
            eb.write_jsonl(path, records)
            self.assertIn("ação", path.read_text(encoding="utf-8"))
            self.assertEqual(eb.read_jsonl(path), records)

    def test_fixtures_present(self) -> None:
        for entity in ("contratos", "fornecedores", "modificacoes"):
            self.assertTrue((eb.FIXTURES / f"{entity}.jsonl").is_file(), entity)


class ReportTest(unittest.TestCase):
    def test_pass_and_fail(self) -> None:
        ok = eb.ScenarioResult("stats", checks=3)
        self.assertIn("ALL SCENARIOS PASSED: 1 scenarios, 3 checks", eb.report([ok]))
        bad = eb.ScenarioResult("write", passed=False, checks=1)
        text = eb.report([ok, bad])
        self.assertIn("FAILED: write", text)
        self.assertIn("FAIL", text)


class MainUsageTest(unittest.TestCase):
    def test_usage_errors_exit_2(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(eb.main(["--binary", str(Path(tmp) / "none")]), 2)
            binary = Path(tmp) / "bin"
            binary.write_text("", encoding="utf-8")
            self.assertEqual(eb.main(["--binary", str(binary), "--only", "stats"]), 2)
            self.assertEqual(eb.main(["--binary", str(binary), "--only", "bogus"]), 2)
            self.assertEqual(
                eb.main(
                    [
                        "--binary",
                        str(binary),
                        "--only",
                        "write",
                        "--root",
                        tmp,
                        "--scratch",
                        str(Path(tmp) / "s"),
                    ]
                ),
                2,
            )


if __name__ == "__main__":
    unittest.main()
