"""Structural tests of ``perguntas_portal_base.py`` and its questions document.

They need neither the ``rucene-base`` binary nor the indices. Run from the
repository root::

    python3 -I -m unittest discover -s crates/rucene-cli-base/scripts/tests -b
"""

from __future__ import annotations

import re
import sys
import types
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import perguntas_portal_base as pq

ROW = re.compile(r"^\| (\d+) \| `([a-z0-9]+)` \| .+ \| (\d+) \| (\d+) \| [^|]+ \| [^|]+ \|$")
SECTION_ID = re.compile(r"^\*\*Id:\*\* `([a-z0-9]+)` · \*\*R = (\d+), I = (\d+)\*\*")


def fake_ctx(*results: list[dict]) -> list[types.SimpleNamespace]:
    """Step results as `vals`/`rows` read them: search rows, no documents."""
    return [types.SimpleNamespace(rows=rows, docs=[]) for rows in results]


class QuestionSetTest(unittest.TestCase):
    def test_exactly_100_questions_with_unique_ids(self) -> None:
        ids = [q["id"] for q in pq.QUESTIONS]
        self.assertEqual(len(ids), 100)
        self.assertEqual(len(set(ids)), 100)

    def test_declared_r_and_i_equal_the_plan(self) -> None:
        for q in pq.QUESTIONS:
            with self.subTest(q=q["id"]):
                self.assertEqual(q["r"], len(q["steps"]))
                self.assertEqual(q["i"], len({s.entity for s in q["steps"]}))
                self.assertEqual((q["r"], q["i"]), (pq.plan_r(q), pq.plan_i(q)))
        pq.check_declarations()

    def test_every_question_uses_two_indices(self) -> None:
        for q in pq.QUESTIONS:
            with self.subTest(q=q["id"]):
                self.assertGreaterEqual(pq.plan_i(q), 2)

    def test_published_order_is_r_then_i_descending(self) -> None:
        keys = [(q["r"], q["i"]) for q in pq.ordered(pq.QUESTIONS)]
        self.assertEqual(keys, sorted(keys, reverse=True))

    def test_every_command_is_read_only(self) -> None:
        for q in pq.QUESTIONS:
            for s in q["steps"]:
                with self.subTest(q=q["id"], step=s.display()):
                    self.assertIn(s.sub, pq.READ_ONLY)
                    self.assertFalse(pq.WRITE_FLAGS.intersection(s.args))
                    self.assertTrue(s.display().startswith(f"rucene-base {s.entity} {s.sub} $R"))

    def test_write_commands_are_refused(self) -> None:
        for sub in ("create", "add", "update", "delete"):
            with self.subTest(sub=sub):
                with self.assertRaises(ValueError):
                    pq.S("contratos", sub, "--key", "1")
                with self.assertRaises(pq.PlanError):
                    pq.check_read_only(["rucene-base", "contratos", sub, "root"])
        with self.assertRaises(pq.PlanError):
            pq.check_read_only(["rucene-base", "contratos", "search", "root", "--set", "a=b"])
        pq.check_read_only(["rucene-base", "contratos", "count", "root", "--term", "Ano=2025"])

    def test_fields_and_operations_match_the_schemas(self) -> None:
        schemas = pq.load_schemas()
        self.assertEqual(sorted(schemas), sorted(pq.ENTITIES))
        self.assertEqual(schemas["modificacoes"]["fields"]["sequencia"]["kind"], "int")
        self.assertTrue(schemas["contratos"]["fields"]["cpvGrupo"]["derived"])
        self.assertEqual(schemas["contratos"]["fields"]["precoBaseProcedimento"]["absent"], "0")
        for q in pq.QUESTIONS:
            for s in q["steps"]:
                with self.subTest(q=q["id"], step=s.display()):
                    for field, kind, op, _mod in pq.step_ops(s, schemas):
                        if field in ("", "*"):
                            continue
                        self.assertIn(op, pq.SUPPORTED[kind])


class FillTest(unittest.TestCase):
    def test_display_and_argv_fill_from_earlier_steps(self) -> None:
        step = pq.S(
            "contratos", "search", "--terms", "adjudicanteNif={x}", "--term", "Ano=2025",
            x=pq.J(1, "nif"),
        )
        self.assertEqual(
            step.display(),
            "rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --term Ano=2025",
        )
        ctx = fake_ctx([{"nif": "600000001 | 500000002"}, {"nif": "600000001"}])
        self.assertEqual(
            step.argv(ctx, "bin", "root"),
            ["bin", "contratos", "search", "root", "--terms", "adjudicanteNif=600000001,500000002",
             "--term", "Ano=2025"],
        )

    def test_get_keys_expand_and_empty_fill_fails(self) -> None:
        step = pq.S("fornecedores", "get", "{*k}", k=pq.KEYS(1, "adjudicatarioNif"))
        ctx = fake_ctx([{"adjudicatarioNif": "500000002"}, {"adjudicatarioNif": "500000003"}])
        self.assertEqual(step.argv(ctx)[4:], ["--key", "500000002", "--key", "500000003"])
        with self.assertRaises(pq.PlanError):
            step.argv(fake_ctx([]))


class MaskingTest(unittest.TestCase):
    def test_personal_nifs_are_masked(self) -> None:
        for personal in ("123456789", "234567890", "345678901", "456789012", "712345678", "812345678"):
            with self.subTest(nif=personal):
                self.assertEqual(pq.nif(personal), "<NIF singular omitido>")
                self.assertNotIn(personal, pq.mask(f"adjudicatário {personal};"))
        for collective in ("501306099", "600010686", "980123456"):
            with self.subTest(nif=collective):
                self.assertEqual(pq.nif(collective), collective)
                self.assertEqual(pq.mask(collective), collective)
                self.assertTrue(pq.collective(collective))

    def test_ids_codes_and_urls_are_kept(self) -> None:
        text = ("contrato 11040286; idINCM=419675763; IdIncm=419675763; "
                "https://files.example.pt/419958342.pdf; 1234567890")
        self.assertEqual(pq.mask(text), text)
        self.assertEqual(pq.mask("x 212345678 y"), "x <NIF singular omitido> y")


class DocumentTest(unittest.TestCase):
    def setUp(self) -> None:
        self.text = pq.DOCUMENT.read_text(encoding="utf-8")

    def test_summary_lists_the_same_questions_in_order(self) -> None:
        rows = [ROW.match(line) for line in self.text.splitlines()]
        got = [(m.group(2), int(m.group(3)), int(m.group(4))) for m in rows if m]
        want = [(q["id"], q["r"], q["i"]) for q in pq.ordered(pq.QUESTIONS)]
        self.assertEqual(got, want)
        self.assertEqual([int(m.group(1)) for m in rows if m], list(range(1, 101)))

    def test_sections_follow_the_same_order(self) -> None:
        got = [(m.group(1), int(m.group(2)), int(m.group(3)))
               for m in map(SECTION_ID.match, self.text.splitlines()) if m]
        want = [(q["id"], q["r"], q["i"]) for q in pq.ordered(pq.QUESTIONS)]
        self.assertEqual(got, want)

    def test_no_machine_paths(self) -> None:
        for path in ("/tmp/", "/home/", "/data/dev"):
            with self.subTest(path=path):
                self.assertNotIn(path, self.text)


class MainUsageTest(unittest.TestCase):
    def test_usage_errors_exit_2(self) -> None:
        for argv in (
            ["--binary", "no/such/binary"],
            ["--binary", __file__, "--root", "no/such/root"],
            ["--binary", __file__, "--root", str(Path(__file__).parent), "--only", "zz99"],
            ["--binary", __file__, "--root", str(Path(__file__).parent), "--workers", "5"],
            ["--binary", __file__, "--root", str(Path(__file__).parent), "--only", "h01", "--write-md"],
        ):
            with self.subTest(argv=argv):
                self.assertEqual(pq.main(argv), 2)


if __name__ == "__main__":
    unittest.main()
