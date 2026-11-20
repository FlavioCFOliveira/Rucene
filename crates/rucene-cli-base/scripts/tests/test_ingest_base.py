"""Unit tests of the clean, derive and write stages of ``ingest_base.py``.

Run from the crate directory, without network or the ``rucene-base`` binary::

    python3 -I -m unittest discover -s scripts/tests -t scripts

The fixture under ``fixtures/`` holds real Portal BASE records (public domain,
snapshot of 2026-11-15): seven 2025 contracts plus one identical duplicate,
two 2012 contracts each emitted twice (once per contracting entity), three
modifications, three 2012 and two 2025 announcements (one notice repeated per
contracting entity, one ``PrecoBase`` "Inexistente") and the matching rows of
the entity register.
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import stat
import sys
import tempfile
import textwrap
import unicodedata
import unittest
from decimal import Decimal
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import ingest_base as ib

FIXTURES = Path(__file__).resolve().parent / "fixtures"

# Field names of the bulk input format v1 (cli_input_format.md, section 4),
# with the kind that constrains the JSON value: "s" scalar, "l" list.
SPEC: dict[str, dict[str, str]] = {
    "contratos": dict.fromkeys(
        [
            "idcontrato",
            "idprocedimento",
            "tipoprocedimento",
            "objectoContrato",
            "descContrato",
            "dataPublicacao",
            "dataCelebracaoContrato",
            "dataDecisaoAdjudicacao",
            "dataFechoContrato",
            "precoContratual",
            "precoBaseProcedimento",
            "PrecoTotalEfetivo",
            "precoEstimadoProc",
            "prazoExecucao",
            "Ano",
            "regime",
            "fundamentacao",
            "fundamentAjusteDireto",
            "tipoFimContrato",
            "TipoMEC",
            "estadoContrato",
            "ProcedimentoCentralizado",
            "CritMateriais",
            "ContratEcologico",
            "compraConjunta",
            "ocorrencias",
            "numAcordoQuadro",
            "DescrAcordoQuadro",
            "nAnuncio",
            "IDAnuncio",
            "idINCM",
            "TipoAnuncio",
            "cpvPrincipal",
            "numPropostas",
            "linkPecasProc",
            "Observacoes",
            "CausasAlteraPrazo",
            "CausasAlteraPreco",
            "numModificacoes",
        ],
        "s",
    )
    | dict.fromkeys(
        [
            "tipoContrato",
            "adjudicanteNif",
            "adjudicanteNome",
            "adjudicatarioNif",
            "adjudicatarioNome",
            "cpv",
            "cpvDescricao",
            "localExecucao",
            "nuts",
            "justifNReducEscrContrato",
            "concorrenteNif",
            "concorrenteNome",
            "convidadaNif",
            "convidadaNome",
            "loteDesignacao",
            "criterioLote",
            "criterioAdjudicacaoTipo",
            "criterioAdjudicacaoTipoFator",
            "criterioAdjudicacaoFator",
        ],
        "l",
    ),
    "fornecedores": dict.fromkeys(
        [
            "nif",
            "pme",
            "numContratos",
            "valorTotal",
            "primeiroAno",
            "ultimoAno",
            "entNumContratos",
            "entTotAdjudicatario",
            "entTotValorContratIni",
            "descPais",
        ],
        "s",
    )
    | dict.fromkeys(["nome", "codPais", "nutsCod", "nutsDesc"], "l"),
    "adjudicantes": dict.fromkeys(
        [
            "nif",
            "numContratos",
            "valorTotal",
            "primeiroAno",
            "ultimoAno",
            "entNumContratos",
            "entTotAdjudicante",
            "entTotAdjudicanteValorContratIni",
            "descPais",
        ],
        "s",
    )
    | dict.fromkeys(
        ["nome", "codPais", "nutsCod", "nutsDesc", "tipoEntidade", "atividadeEntidade"],
        "l",
    ),
    "procedimentos": dict.fromkeys(
        [
            "idprocedimento",
            "tipoprocedimento",
            "precoBaseProcedimento",
            "precoEstimadoProc",
            "numContratos",
            "valorTotal",
            "primeiraPublicacao",
            "ultimaPublicacao",
        ],
        "s",
    )
    | dict.fromkeys(
        ["idcontrato", "objectoContrato", "adjudicanteNif", "IDAnuncio", "nAnuncio"],
        "l",
    ),
    "cpv": dict.fromkeys(
        [
            "cpv",
            "descricao",
            "numContratos",
            "numPrincipal",
            "numComplementar",
            "valorTotal",
        ],
        "s",
    ),
    "locais": dict.fromkeys(
        ["local", "pais", "distrito", "concelho", "numContratos", "valorTotal"], "s"
    ),
    "nuts": dict.fromkeys(
        [
            "nuts",
            "descricao",
            "numContratos",
            "numAdjudicantes",
            "numFornecedores",
            "valorTotal",
        ],
        "s",
    ),
    "modificacoes": dict.fromkeys(
        [
            "idcontrato",
            "sequencia",
            "modifContratoFundamento",
            "modifContratoTipoAto",
            "modifContratoData",
            "modifDataPublicacao",
            "modifContratoPrecoAlterado",
            "modifPrazoExecucao",
            "Ano",
            "modifJustNContratEscrito",
        ],
        "s",
    )
    | dict.fromkeys(["subcontratadaNif", "subcontratadaNome"], "l"),
    "anuncios": dict.fromkeys(
        [
            "IDAnuncio",
            "nAnuncio",
            "IdIncm",
            "dataPublicacao",
            "descricaoAnuncio",
            "url",
            "numDR",
            "serie",
            "tipoActo",
            "PrecoBase",
            "modeloAnuncio",
            "Ano",
            "CriterAmbient",
            "PrazoPropostas",
            "PecasProcedimento",
            "DataLimitePropostas",
            "AcordoQuadro",
            "sistemaAquisicaoDinamico",
        ],
        "s",
    )
    | dict.fromkeys(
        [
            "nifEntidade",
            "designacaoEntidade",
            "tiposContrato",
            "cpv",
            "cpvDescricao",
            "loteDesignacao",
            "loteValorBase",
            "criterioLote",
            "criterioAquisicaoTipo",
            "criterioAquisicaoTipoFator",
            "anteriorIDAnuncio",
            "anteriorNAnuncio",
        ],
        "l",
    ),
    "convidadas": dict.fromkeys(["nif", "numConvites", "primeiroAno", "ultimoAno"], "s")
    | dict.fromkeys(["nome", "aliasPais", "pais"], "l"),
}
KEYS = {
    "contratos": "idcontrato",
    "fornecedores": "nif",
    "adjudicantes": "nif",
    "procedimentos": "idprocedimento",
    "cpv": "cpv",
    "locais": "local",
    "nuts": "nuts",
    "anuncios": "IDAnuncio",
    "convidadas": "nif",
}


def fixture_records(name: str) -> list[dict[str, Any]]:
    """Return the raw records of one fixture file, numbers as ``Decimal``."""
    return list(ib.read_records(FIXTURES / name, None))


def raw_contract(cid: str) -> dict[str, Any]:
    """Return the first raw fixture contract with ``idcontrato == cid``."""
    for name in ("contratos2025.json", "contratos2012.json"):
        for r in fixture_records(name):
            if r["idcontrato"] == cid:
                return r
    raise KeyError(cid)


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    """Read a JSON Lines file."""
    with path.open(encoding="utf-8") as fh:
        return [json.loads(line) for line in fh]


class TextCleaningTest(unittest.TestCase):
    def test_name_unescapes_normalises_and_collapses(self) -> None:
        decomposed = unicodedata.normalize("NFD", "Câmara  de\tLobos &amp; Cª ")
        self.assertEqual(ib.clean_name(decomposed), "Câmara de Lobos & Cª")
        self.assertTrue(
            unicodedata.is_normalized("NFC", ib.clean_name(decomposed) or "")
        )

    def test_name_empty_is_none(self) -> None:
        self.assertIsNone(ib.clean_name("   "))
        self.assertIsNone(ib.clean_name(None))

    def test_name_rejects_non_string(self) -> None:
        with self.assertRaises(ib.IngestError):
            ib.clean_name(["a"])

    def test_text_keeps_paragraphs(self) -> None:
        raw = "  Linha 1  \r\n\r\n\r\n  linha   2 &#8211; fim\t\n"
        self.assertEqual(ib.clean_text(raw), "Linha 1\n\nlinha 2 – fim")

    def test_text_placeholders(self) -> None:
        for raw in ("n/a", " . ", "-", "N/A"):
            self.assertIsNone(
                ib.clean_text(raw, placeholders=ib.FREE_TEXT_PLACEHOLDERS)
            )
        self.assertEqual(
            ib.clean_text("n/a.", placeholders=ib.FREE_TEXT_PLACEHOLDERS), "n/a."
        )

    def test_nif_placeholder(self) -> None:
        self.assertIsNone(ib.clean_nif("-"))
        self.assertIsNone(ib.clean_nif(""))
        self.assertEqual(ib.clean_nif(" 500188629 "), "500188629")


class ValueCleaningTest(unittest.TestCase):
    def setUp(self) -> None:
        self.report = ib.Report()

    def test_dates(self) -> None:
        r = self.report
        self.assertEqual(ib.clean_date("2025-01-03", r, "k"), "2025-01-03")
        self.assertEqual(ib.clean_date("15/04/2026 00:00:00", r, "k"), "2026-04-15")
        self.assertEqual(ib.clean_date("09/03/2026", r, "k"), "2026-03-09")
        self.assertIsNone(ib.clean_date("", r, "k"))
        self.assertIsNone(ib.clean_date("31/02/2020 00:00:00", r, "k"))
        self.assertIsNone(ib.clean_date("2020/01/01", r, "k"))
        self.assertEqual(r.events["k: invalid date (dropped)"], 2)

    def test_money(self) -> None:
        r = self.report
        self.assertEqual(ib.clean_money(Decimal("11500.00"), r, "p"), 11500.0)
        self.assertEqual(ib.clean_money(Decimal("0.0"), r, "p"), 0.0)
        self.assertIsNone(ib.clean_money(Decimal("0.0"), r, "p", zero_is_null=True))
        self.assertEqual(ib.clean_money("30000000.00", r, "p"), 30000000.0)
        self.assertEqual(ib.clean_money(Decimal(-10566000), r, "p"), -10566000.0)
        self.assertIsNone(ib.clean_money("Inexistente", r, "p"))
        self.assertIsNone(ib.clean_money("12,5", r, "p"))
        self.assertIsNone(ib.clean_money("NaN", r, "p"))
        self.assertEqual(r.events["p: placeholder 'Inexistente' -> null"], 1)
        self.assertEqual(r.events["p: invalid number (dropped)"], 2)

    def test_integers(self) -> None:
        r = self.report
        self.assertEqual(ib.clean_int(730, r, "t", ib.TERM_SENTINEL), 730)
        self.assertEqual(ib.clean_int("480", r, "t", ib.TERM_SENTINEL), 480)
        self.assertIsNone(ib.clean_int(99999999, r, "t", ib.TERM_SENTINEL))
        self.assertIsNone(ib.clean_int("99999999", r, "t", ib.TERM_SENTINEL))
        self.assertIsNone(ib.clean_int(2**31, r, "t"))
        self.assertIsNone(ib.clean_int("4x", r, "t"))
        self.assertIsNone(ib.clean_int(True, r, "t"))

    def test_flags(self) -> None:
        r = self.report
        self.assertEqual(ib.clean_flag("Sim", r, "f"), "Sim")
        self.assertEqual(ib.clean_flag("NÃO", r, "f"), "Não")
        self.assertEqual(ib.clean_flag("nao", r, "f"), "Não")
        self.assertIsNone(ib.clean_flag("talvez", r, "f"))

    def test_competitor_split(self) -> None:
        self.assertEqual(
            ib.split_competitor("511005083-Mendes Gomes &amp; Companhia, Lda"),
            {"nif": "511005083", "nome": "Mendes Gomes & Companhia, Lda"},
        )
        self.assertEqual(
            ib.split_competitor("--B2Mobility,GmbH"),
            {"nif": None, "nome": "B2Mobility,GmbH"},
        )
        self.assertEqual(
            ib.split_competitor("501689168-LUSITANIA-COMPANHIA DE SEGUROS, S.A."),
            {"nif": "501689168", "nome": "LUSITANIA-COMPANHIA DE SEGUROS, S.A."},
        )
        self.assertEqual(
            ib.split_competitor("Sem separador"), {"nif": None, "nome": "Sem separador"}
        )

    def test_code_description_split(self) -> None:
        self.assertEqual(
            ib.split_code_desc("PT300 - Região Autónoma da Madeira", "n", ib.NUTS_RE),
            {"codigo": "PT300", "descricao": "Região Autónoma da Madeira"},
        )
        self.assertEqual(
            ib.split_code_desc(
                "45234100-7 - Construção de vias férreas", "c", ib.CPV_RE
            ),
            {"codigo": "45234100-7", "descricao": "Construção de vias férreas"},
        )
        self.assertEqual(
            ib.split_code_desc("Sem código", "c", ib.CPV_RE),
            {"codigo": None, "descricao": "Sem código"},
        )

    def test_codes(self) -> None:
        r = self.report
        self.assertEqual(ib.clean_nuts_code("pt11a", r, "n"), "PT11A")
        self.assertEqual(ib.clean_nuts_code("PT", r, "n"), "PT")
        self.assertIsNone(ib.clean_nuts_code("Portugal", r, "n"))
        self.assertEqual(ib.clean_cpv_code("79970000-4", r, "c"), "79970000-4")
        self.assertIsNone(ib.clean_cpv_code("79970000", r, "c"))


class StreamingJsonTest(unittest.TestCase):
    def parse(self, text: str, chunk: int = 7) -> list[dict[str, Any]]:
        old = ib.CHUNK
        ib.CHUNK = chunk
        try:
            return list(ib.iter_json_array(io.StringIO(text), "t.json"))
        finally:
            ib.CHUNK = old

    def test_small_chunks(self) -> None:
        objs = [{"a": i, "b": "x" * i, "c": Decimal("1.50")} for i in range(30)]
        text = "[\n" + ",\n".join(json.dumps({**o, "c": 1.5}) for o in objs) + "\n]\n"
        got = self.parse(text)
        self.assertEqual(len(got), 30)
        self.assertEqual(got[29]["b"], "x" * 29)
        self.assertIsInstance(got[0]["c"], Decimal)

    def test_empty_array(self) -> None:
        self.assertEqual(self.parse(" [ ] "), [])

    def test_errors(self) -> None:
        for bad in (
            "",
            "{}",
            "[{}{}]",
            "[{},]",
            '[{} , {"a": }]',
            "[{}] x",
            "[1]",
            "[{}",
        ):
            with self.subTest(bad=bad), self.assertRaises(ib.IngestError):
                self.parse(bad)


class ContractCleaningTest(unittest.TestCase):
    def setUp(self) -> None:
        self.report = ib.Report()

    def test_placeholders_and_formats(self) -> None:
        c = ib.clean_contract(raw_contract("11070315"), self.report)
        self.assertEqual(c["dataCelebracaoContrato"], "2025-01-03")
        self.assertIsNone(c["dataFechoContrato"])
        self.assertIsNone(c["PrecoTotalEfetivo"])  # 0.0 = not filled
        self.assertEqual(c["precoBaseProcedimento"], 26000.0)
        self.assertEqual(c["prazoExecucao"], 730)
        self.assertEqual(
            c["NUTs"], [{"codigo": "PT300", "descricao": "Região Autónoma da Madeira"}]
        )
        self.assertEqual(
            c["concorrentes"],
            [{"nif": "511005083", "nome": "Mendes Gomes & Companhia, Lda"}],
        )
        self.assertEqual(c["adjudicatarios"][0]["pme"], "Sim")
        self.assertIs(c["ocorrencias"], False)
        self.assertEqual(self.report.events["contratos: unknown key ignored"], 0)

    def test_keyless_supplier(self) -> None:
        c = ib.clean_contract(raw_contract("11126545"), self.report)
        self.assertIsNone(c["adjudicatarios"][0]["nif"])
        self.assertEqual(c["adjudicatarios"][0]["nome"], "B2Mobility,GmbH")
        self.assertEqual(c["concorrentes"], [{"nif": None, "nome": "B2Mobility,GmbH"}])

    def test_missing_key_fails(self) -> None:
        raw = dict(raw_contract("11070315"), idcontrato="")
        with self.assertRaises(ib.IngestError):
            ib.clean_contract(raw, self.report)

    def test_unknown_key_counted(self) -> None:
        raw = dict(raw_contract("11070315"), campoNovo="x")
        ib.clean_contract(raw, self.report)
        self.assertEqual(self.report.events["contratos: unknown key ignored"], 1)

    def test_contract_to_cli(self) -> None:
        c = ib.clean_contract(raw_contract("11160183"), self.report)
        out = ib.contract_to_cli(c, 0)
        self.assertEqual(out["cpv"], ["43262000-7", "43251000-7"])
        self.assertEqual(out["cpvPrincipal"], "43251000-7")
        self.assertEqual(out["concorrenteNome"], ["LEONARDO DIAS & DIAS, Lda."])
        self.assertEqual(out["numModificacoes"], 0)
        self.assertEqual(out["ocorrencias"], "Não")
        if c["dataFechoContrato"] is None:
            self.assertNotIn("dataFechoContrato", out)
        for value in out.values():
            self.assertNotIsInstance(value, dict)
            self.assertNotEqual(value, [])


class ModificationTest(unittest.TestCase):
    def test_sequence_order(self) -> None:
        report = ib.Report()
        mods = [
            ib.clean_modification(r, report)
            for r in fixture_records("modcontrats2025.json")
        ]
        seq = ib.sequence_modifications(mods)
        got = [(m["idcontrato"], m["sequencia"], m["modifContratoData"]) for m in seq]
        self.assertEqual(
            got,
            [
                ("11103352", 1, "2026-04-15"),
                ("11104095", 1, "2025-07-28"),
                ("11104095", 2, "2026-03-09"),
            ],
        )
        out = ib.modification_to_cli(seq[0])
        self.assertEqual(out["modifPrazoExecucao"], 480)
        self.assertEqual(out["modifDataPublicacao"], "2026-04-16")
        self.assertNotIn("modifJustNContratEscrito", out)

    def test_missing_dates_sort_last(self) -> None:
        base = {"modifDataPublicacao": None}
        mods = [
            {"idcontrato": "1", "modifContratoData": None, **base, "n": 0},
            {"idcontrato": "1", "modifContratoData": "2020-01-01", **base, "n": 1},
        ]
        self.assertEqual([m["n"] for m in ib.sequence_modifications(mods)], [1, 0])


class NoticeTest(unittest.TestCase):
    def test_merge_joint_procurement_rows(self) -> None:
        report = ib.Report()
        notices = [
            ib.clean_notice(r, report) for r in fixture_records("anuncios2012.json")
        ]
        merged = ib.merge_notices(notices, report)
        self.assertEqual(len(merged), 2)
        first = merged[0]
        self.assertEqual(first["IDAnuncio"], "19268")
        self.assertEqual(
            [e["nif"] for e in first["entidades"]], ["501356126", "600082490"]
        )
        self.assertEqual(report.events["anuncios: duplicate IDAnuncio rows merged"], 1)
        out = ib.notice_to_cli(first)
        # Bulk format v1.2: every contracting entity of the merged notice.
        self.assertEqual(out["nifEntidade"], ["501356126", "600082490"])
        self.assertEqual(
            out["designacaoEntidade"], [e["nome"] for e in first["entidades"]]
        )
        self.assertEqual(len(out["designacaoEntidade"]), 2)
        self.assertIsNone(merged[1]["PrecoBase"])  # "Inexistente"

    def test_cpv_and_previous(self) -> None:
        report = ib.Report()
        n = ib.clean_notice(fixture_records("anuncios2025.json")[0], report)
        out = ib.notice_to_cli(n)
        self.assertEqual(out["cpv"], ["45234100-7"])
        self.assertEqual(out["cpvDescricao"], ["Construção de vias férreas"])
        self.assertEqual(out["PrecoBase"], 30000000.0)
        self.assertEqual(out["anteriorIDAnuncio"], ["400107"])
        self.assertEqual(out["criterioAquisicaoTipoFator"], ["Qualidade", "Preço"])


class DeriverTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.report = ib.Report()
        self.deriver = ib.Deriver(Path(self.tmp.name) / "spill", self.report)

    def tearDown(self) -> None:
        self.deriver.close()
        self.tmp.cleanup()

    def add(self, cid: str) -> dict[str, Any]:
        c = ib.clean_contract(raw_contract(cid), self.report)
        self.deriver.add(c)
        return c

    def test_value_split_among_co_suppliers(self) -> None:
        c = self.add("11124453")
        recs = {r["nif"]: r for r in self.deriver.suppliers_records({})}
        self.assertEqual(set(recs), {"500150141", "500188629"})
        half = c["precoContratual"] / 2
        for r in recs.values():
            self.assertAlmostEqual(r["valorTotal"], half, places=2)
            self.assertEqual(r["numContratos"], 1)

    def test_keyless_supplier_not_keyed(self) -> None:
        self.add("11126545")
        self.assertEqual(list(self.deriver.suppliers_records({})), [])
        self.assertEqual(
            self.deriver.keyless["adjudicatarios references without NIF"], 1
        )

    def test_cpv_roles(self) -> None:
        self.add("11160183")
        recs = {r["cpv"]: r for r in self.deriver.cpv_records()}
        self.assertEqual(recs["43251000-7"]["numPrincipal"], 1)
        self.assertEqual(recs["43262000-7"]["numComplementar"], 1)
        self.assertEqual(recs["43262000-7"]["valorTotal"], 2800.0)

    def test_places_and_procedures(self) -> None:
        self.add("11070315")
        places = list(self.deriver.places_records())
        self.assertEqual(places[0]["concelho"], "Câmara de Lobos")
        self.assertEqual(places[0]["distrito"], "Região Autónoma da Madeira")
        procs = list(self.deriver.procedure_records())
        self.assertEqual(procs[0]["idprocedimento"], "7333485")
        self.assertEqual(procs[0]["idcontrato"], ["11070315"])
        self.assertEqual(procs[0]["numContratos"], 1)

    def test_place_with_one_part(self) -> None:
        self.deriver.places["Portugal"].contracts = 1
        rec = next(self.deriver.places_records())
        self.assertEqual(
            rec,
            {
                "local": "Portugal",
                "pais": "Portugal",
                "numContratos": 1,
                "valorTotal": 0.0,
            },
        )


class FixtureBuildTest(unittest.TestCase):
    """Stages 2-4 (no load) over the whole offline fixture."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cache = Path(cls.tmp.name) / "cache"
        datasets = {"contracts", "mods", "notices", "entities"}
        files = ib.offline_files(datasets, FIXTURES, allow_missing_years=True)
        cls.report = ib.Report()
        cls.outs = ib.build(files, ib.ENTITIES, cls.cache, None, cls.report)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def bulk(self, entity: str) -> list[dict[str, Any]]:
        return [r for p in self.outs.parts[entity] for r in read_jsonl(p)]

    def test_counts(self) -> None:
        self.assertEqual(self.outs.counts["contratos"], 9)
        self.assertEqual(self.outs.counts["modificacoes"], 3)
        self.assertEqual(self.outs.counts["anuncios"], 4)
        self.assertEqual(
            self.report.events[
                "contratos: identical duplicate idcontrato (newest kept)"
            ],
            3,
        )
        self.assertNotIn(
            "contratos: non-identical duplicate idcontrato (newest kept)",
            self.report.events,
        )
        for entity in ib.ENTITIES:
            self.assertGreater(self.outs.counts[entity], 0, entity)

    def test_bulk_records_follow_the_format(self) -> None:
        for entity in ib.ENTITIES:
            spec = SPEC[entity]
            for rec in self.bulk(entity):
                with self.subTest(
                    entity=entity, rec=rec.get(KEYS.get(entity, "idcontrato"))
                ):
                    self.assertLessEqual(set(rec), set(spec))
                    for key, value in rec.items():
                        if spec[key] == "l":
                            self.assertIsInstance(value, list)
                            self.assertTrue(value)
                            for v in value:
                                self.assertIsInstance(v, (str, int, float))
                        else:
                            self.assertIsInstance(value, (str, int, float))
                    if entity == "modificacoes":
                        self.assertIn("sequencia", rec)
                    self.assertIn(KEYS.get(entity, "idcontrato"), rec)

    def test_keys_unique(self) -> None:
        for entity, key in KEYS.items():
            values = [r[key] for r in self.bulk(entity)]
            self.assertEqual(len(values), len(set(values)), entity)
        mods = [(r["idcontrato"], r["sequencia"]) for r in self.bulk("modificacoes")]
        self.assertEqual(len(mods), len(set(mods)))

    def test_contract_aggregates(self) -> None:
        recs = {r["idcontrato"]: r for r in self.bulk("contratos")}
        self.assertEqual(recs["11104095"]["numModificacoes"], 2)
        self.assertEqual(recs["10855918"]["numModificacoes"], 0)
        self.assertEqual(recs["409724"]["adjudicanteNif"], ["600054110", "503606251"])

    def test_entity_register_join(self) -> None:
        sup = {r["nif"]: r for r in self.bulk("fornecedores")}
        self.assertIn("entNumContratos", sup["500188629"])
        adj = {r["nif"]: r for r in self.bulk("adjudicantes")}
        self.assertIn("entTotAdjudicante", adj["511233620"])
        self.assertEqual(adj["511233620"]["nutsCod"], ["PT300"])

    def test_intermediate_files(self) -> None:
        for entity in ib.ENTITIES:
            path = self.cache / "clean" / f"{entity}.jsonl"
            self.assertEqual(len(read_jsonl(path)), self.outs.counts[entity], entity)
        notices = read_jsonl(self.cache / "clean" / "anuncios.jsonl")
        joint = next(n for n in notices if n["IDAnuncio"] == "19268")
        self.assertEqual(len(joint["entidades"]), 2)

    def test_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as other:
            files = ib.offline_files(
                {"contracts", "mods", "notices", "entities"}, FIXTURES, True
            )
            outs = ib.build(files, ib.ENTITIES, Path(other), None, ib.Report())
            for entity in ib.ENTITIES:
                a = [p.read_bytes() for p in self.outs.parts[entity]]
                b = [p.read_bytes() for p in outs.parts[entity]]
                self.assertEqual(a, b, entity)


FAKE_CLI = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json, pathlib, sys
    import time
    entity, cmd, root = sys.argv[1], sys.argv[2], sys.argv[3]
    # One state file per index: indices load concurrently.
    state = pathlib.Path(sys.argv[0]).with_suffix(f".{entity}.state")
    log = json.loads(state.read_text()) if state.exists() else {"calls": [], "n": {}}
    log.setdefault("t", []).append(time.monotonic())
    if cmd == "add":
        time.sleep(0.2)
    log["calls"].append(sys.argv[1:])
    if cmd == "create":
        log["n"][entity] = 0
    elif cmd == "add":
        lines = pathlib.Path(sys.argv[5]).read_text(encoding="utf-8").splitlines()
        log["n"][entity] += len(lines) + (1 if entity == "cpv" else 0)
    elif cmd == "count":
        print(log["n"][entity])
    state.write_text(json.dumps(log))
    """
)


class LoadTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.binary = base / "fake-cli"
        self.binary.write_text(
            FAKE_CLI.replace("/usr/bin/env python3", sys.executable), encoding="utf-8"
        )
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)
        self.root = base / "index"
        self.outs = ib.Outputs()
        for entity, n in (("locais", 3), ("cpv", 2)):
            pw = ib.PartWriter(base / "cli" / entity, batch=2)
            for i in range(n):
                pw.write({"local" if entity == "locais" else "cpv": f"k{i}"})
            pw.close()
            self.outs.parts[entity] = pw.parts
            self.outs.counts[entity] = pw.count

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def log(self, entity: str) -> dict[str, Any]:
        state = self.binary.with_suffix(f".{entity}.state")
        return json.loads(state.read_text())

    def calls(self, entity: str = "locais") -> list[list[str]]:
        return self.log(entity)["calls"]

    def test_create_then_add_each_part(self) -> None:
        ib.load(self.binary, self.root, self.outs, ("locais",))
        calls = self.calls()
        self.assertEqual(
            [c[:2] for c in calls],
            [
                ["locais", "create"],
                ["locais", "add"],
                ["locais", "add"],
                ["locais", "count"],
            ],
        )
        self.assertEqual(calls[1][4], str(self.outs.parts["locais"][0]))
        self.assertEqual(calls[2][4], str(self.outs.parts["locais"][1]))
        self.assertTrue(all(c[2] == str(self.root) for c in calls))

    def test_indices_load_side_by_side(self) -> None:
        self.outs.counts["cpv"] = 3  # the fake CLI counts one extra cpv document
        out = io.StringIO()
        old, sys.stdout = sys.stdout, out
        try:
            counted = ib.load(self.binary, self.root, self.outs, ("cpv", "locais"))
        finally:
            sys.stdout = old
        self.assertEqual(counted, {"cpv": "3", "locais": "3"})
        # Each index keeps create, its parts in order, then count.
        for entity in ("cpv", "locais"):
            calls = self.calls(entity)
            self.assertEqual(
                [c[1] for c in calls],
                ["create", *(["add"] * len(self.outs.parts[entity])), "count"],
            )
            self.assertEqual(
                [c[4] for c in calls if c[1] == "add"],
                [str(p) for p in self.outs.parts[entity]],
            )
        # By default both run at once: each started before the other finished.
        cpv, locais = self.log("cpv")["t"], self.log("locais")["t"]
        self.assertLess(max(cpv[0], locais[0]), min(cpv[-1], locais[-1]))
        self.assertEqual(out.getvalue().count("loaded"), 2)

    def test_jobs_one_loads_one_index_at_a_time(self) -> None:
        self.outs.counts["cpv"] = 3
        with open(os.devnull, "w") as null:
            old, sys.stdout = sys.stdout, null
            try:
                ib.load(self.binary, self.root, self.outs, ("cpv", "locais"), 1)
            finally:
                sys.stdout = old
        cpv, locais = self.log("cpv")["t"], self.log("locais")["t"]
        self.assertTrue(cpv[-1] < locais[0] or locais[-1] < cpv[0])

    def test_load_order_is_largest_first(self) -> None:
        counts = {"cpv": 7, "contratos": 100, "nuts": 7, "locais": 1}
        self.assertEqual(
            ib.load_order(("locais", "nuts", "cpv", "contratos"), counts),
            ["contratos", "nuts", "cpv", "locais"],
        )

    def test_first_failure_is_raised_after_running_loads(self) -> None:
        # cpv's count check fails (fake +1); locais still completes.
        with (
            self.assertRaises(ib.IngestError) as ctx,
            open(os.devnull, "w") as null,
        ):
            old, sys.stdout = sys.stdout, null
            try:
                ib.load(self.binary, self.root, self.outs, ("cpv", "locais"))
            finally:
                sys.stdout = old
        self.assertIn("cpv", str(ctx.exception))
        self.assertEqual(self.calls("locais")[-1][1], "count")

    def test_jobs_must_be_positive(self) -> None:
        with self.assertRaises(ib.IngestError):
            ib.load(self.binary, self.root, self.outs, ("locais",), 0)

    def test_count_mismatch_fails(self) -> None:
        with self.assertRaises(ib.IngestError):
            ib.load(self.binary, self.root, self.outs, ("cpv",))

    def test_missing_binary_fails(self) -> None:
        with self.assertRaises(ib.IngestError):
            ib.load(self.binary.with_name("absent"), self.root, self.outs, ("locais",))


class ReportMergeTest(unittest.TestCase):
    def test_merge_in_order_equals_one_pass(self) -> None:
        one = ib.Report()
        parts = [ib.Report(), ib.Report(), ib.Report()]
        for i in range(9):
            for r in (one, parts[i // 3]):
                r.note("k: sampled", f"s{i}")
                r.note("k: counted")
        merged = ib.Report()
        for part in parts:
            merged.merge(part)
        self.assertEqual(merged.lines(), one.lines())
        self.assertEqual(merged.samples["k: sampled"], ["s0", "s1", "s2", "s3", "s4"])


def fixture_files() -> dict[str, list[Path]]:
    with contextlib.redirect_stdout(io.StringIO()):
        return ib.offline_files(
            {"contracts", "mods", "notices", "entities"}, FIXTURES, True
        )


class ParallelBuildTest(unittest.TestCase):
    """The number of preparation processes changes no output."""

    def build(
        self,
        cache: Path,
        prep_jobs: int,
        on_ready: Any = None,
    ) -> tuple[ib.Outputs, ib.Report, list[str]]:
        report = ib.Report()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            outs = ib.build(
                fixture_files(), ib.ENTITIES, cache, None, report, on_ready, prep_jobs
            )
        return outs, report, out.getvalue().splitlines()

    def test_same_files_report_and_lines_for_any_process_count(self) -> None:
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            _, report_a, lines_a = self.build(Path(a), 1)
            _, report_b, lines_b = self.build(Path(b), 3)
            files_a = sorted(p.relative_to(a) for p in Path(a).rglob("*") if p.is_file())
            files_b = sorted(p.relative_to(b) for p in Path(b).rglob("*") if p.is_file())
            self.assertEqual(files_a, files_b)
            self.assertTrue(files_a)
            for rel in files_a:
                self.assertEqual(
                    (Path(a) / rel).read_bytes(), (Path(b) / rel).read_bytes(), rel
                )
            self.assertEqual(report_a.lines(), report_b.lines())
            self.assertEqual(lines_a, lines_b)
            self.assertFalse((Path(b) / "prep").exists())
            self.assertFalse((Path(b) / "spill").exists())

    def test_lines_print_in_sequential_order(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, _, lines = self.build(Path(tmp), 3)
        names = [line.split(":")[0].strip() for line in lines]
        self.assertEqual(
            names,
            [
                "entidades",
                "modificacoes",
                "anuncios",
                "contratos2025.json",
                "contratos2012.json",
                "contratos",
                *ib.DERIVED,
            ],
        )

    def test_on_ready_once_per_index_with_closed_files(self) -> None:
        calls: list[tuple[str, int, int]] = []

        def ready(entity: str, outs: ib.Outputs) -> None:
            lines = sum(len(read_jsonl(p)) for p in outs.parts[entity])
            calls.append((entity, outs.counts[entity], lines))

        with tempfile.TemporaryDirectory() as tmp:
            self.build(Path(tmp), 2, ready)
        self.assertEqual(sorted(c[0] for c in calls), sorted(ib.ENTITIES))
        for entity, count, lines in calls:
            self.assertEqual(count, lines, entity)
            self.assertGreater(count, 0, entity)

    def test_prep_jobs_default_and_bounds(self) -> None:
        self.assertGreaterEqual(ib.default_prep_jobs(), 1)
        self.assertLessEqual(ib.default_prep_jobs(), ib._cpu_count())
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(ib.IngestError):
            self.build(Path(tmp), 0)


class IngestCommandTest(unittest.TestCase):
    """``ingest`` end to end on the fixture with a fake ``rucene-base``."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.binary = base / "fake-cli"
        self.binary.write_text(
            FAKE_CLI.replace("/usr/bin/env python3", sys.executable), encoding="utf-8"
        )
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)
        self.argv = [
            "ingest",
            "--offline",
            str(FIXTURES),
            "--limit",
            "100",
            "--cache",
            str(base / "cache"),
            "--root",
            str(base / "index"),
            "--only",
            "contratos,modificacoes,anuncios,locais",
            "--prep-jobs",
            "2",
        ]

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_every_index_loads_its_parts_in_order(self) -> None:
        with contextlib.redirect_stdout(io.StringIO()) as out:
            status = ib.main([*self.argv, "--binary", str(self.binary)])
        self.assertEqual(status, 0, out.getvalue())
        for entity in ("contratos", "modificacoes", "anuncios", "locais"):
            state = self.binary.with_suffix(f".{entity}.state")
            calls = json.loads(state.read_text())["calls"]
            parts = sorted(
                (Path(self.tmp.name) / "cache" / "cli" / entity).glob("part-*.jsonl")
            )
            self.assertEqual(
                [c[1] for c in calls], ["create", *(["add"] * len(parts)), "count"]
            )
            self.assertEqual([c[4] for c in calls if c[1] == "add"], [str(p) for p in parts])
        self.assertEqual(out.getvalue().count(": loaded "), 4)

    def test_missing_binary_fails_before_preparing(self) -> None:
        with (
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()) as err,
        ):
            status = ib.main([*self.argv, "--binary", str(self.binary) + "-absent"])
        self.assertEqual(status, 1)
        self.assertIn("binary not found", err.getvalue())
        self.assertFalse((Path(self.tmp.name) / "cache" / "clean").exists())


class ArgsTest(unittest.TestCase):
    def quiet_exit(self, argv: list[str]) -> None:
        with self.assertRaises(SystemExit), open(os.devnull, "w") as null:
            old, sys.stderr = sys.stderr, null
            try:
                ib.parse_args(argv)
            finally:
                sys.stderr = old

    def test_subcommands(self) -> None:
        args = ib.parse_args(["ingest", "--only", "cpv,contratos", "--limit", "5"])
        self.assertEqual(
            (args.command, args.only, args.limit), ("ingest", ("contratos", "cpv"), 5)
        )
        self.assertTrue(ib.parse_args(["fetch", "--force"]).force)
        self.assertTrue(ib.parse_args(["status", "--verify"]).verify)
        self.assertEqual(ib.parse_args(["fetch"]).only, ib.ENTITIES)
        self.assertEqual(ib.parse_args(["ingest", "--jobs", "3"]).jobs, 3)
        self.assertIsNone(ib.parse_args(["ingest"]).jobs)
        self.assertEqual(ib.parse_args(["ingest", "--prep-jobs", "2"]).prep_jobs, 2)
        self.assertIsNone(ib.parse_args(["ingest"]).prep_jobs)

    def test_validation(self) -> None:
        self.quiet_exit([])
        self.quiet_exit(["ingest", "--only", "contracts"])
        self.quiet_exit(["ingest", "--limit", "0"])
        self.quiet_exit(["ingest", "--jobs", "0"])
        self.quiet_exit(["ingest", "--prep-jobs", "0"])
        self.quiet_exit(["fetch", "--prep-jobs", "2"])
        self.quiet_exit(["fetch", "--limit", "5"])
        self.quiet_exit(["status", "--force"])

    def test_needs(self) -> None:
        self.assertEqual(set(ib.NEEDS), set(ib.ENTITIES))
        self.assertEqual(ib.datasets_for(["modificacoes"]), {"mods"})
        self.assertEqual(
            ib.datasets_for(["fornecedores", "anuncios"]),
            {"contracts", "entities", "notices"},
        )


class FetchCacheTest(unittest.TestCase):
    """``fetch``/``status``/cache lookup against fake API resources (file:// URLs)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.src = base / "src"
        self.src.mkdir()
        self.raw = base / "cache" / "raw"
        self.resources: list[ib.Resource] = []
        for year in ib.YEARS:
            self.add_resource(f"modcontrats{year}.json", year, f"[{year}]")
        self.real_resolve = ib.resolve_resources
        self.real_download = ib.download
        ib.resolve_resources = lambda ds: [r for r in self.resources if r.dataset == ds]
        self.downloads: list[str] = []

        def counting_download(res: ib.Resource, dest: Path, force: bool) -> str:
            self.downloads.append(res.name)
            return self.real_download(res, dest, force)

        ib.download = counting_download

    def tearDown(self) -> None:
        ib.resolve_resources = self.real_resolve
        ib.download = self.real_download
        self.tmp.cleanup()

    def add_resource(self, name: str, year: int, text: str) -> None:
        path = self.src / name
        path.write_text(text, encoding="utf-8")
        res = ib.Resource(
            "mods",
            year,
            name,
            path.as_uri(),
            path.stat().st_size,
            ib.sha1_of(path),
            "2026-11-15",
        )
        self.resources = [r for r in self.resources if r.name != name] + [res]

    def fetch(self, force: bool = False) -> dict[str, list[Path]]:
        with open(os.devnull, "w") as null:
            old, sys.stdout = sys.stdout, null
            try:
                return ib.fetch({"mods"}, self.raw, force, False)
            finally:
                sys.stdout = old

    def test_idempotent_stale_and_force(self) -> None:
        self.assertIsNone(ib.cached_files({"mods"}, self.raw))
        files = self.fetch()
        self.assertEqual(len(files["mods"]), 15)
        self.assertEqual(len(self.downloads), 15)
        manifest = ib.read_manifest(self.raw)
        self.assertEqual(
            manifest["modcontrats2012.json"]["last_modified"], "2026-11-15"
        )
        self.assertEqual(manifest["modcontrats2012.json"]["dataset"], "mods")
        self.fetch()
        self.assertEqual(len(self.downloads), 15)  # nothing downloaded again
        self.add_resource("modcontrats2020.json", 2020, "[2020, 1]")  # stale
        self.fetch()
        self.assertEqual(self.downloads[15:], ["modcontrats2020.json"])
        self.assertEqual((self.raw / "modcontrats2020.json").read_text(), "[2020, 1]")
        self.fetch(force=True)
        self.assertEqual(len(self.downloads), 31)

    def test_bad_checksum_fails_after_retries(self) -> None:
        res = self.resources[0]
        self.resources[0] = ib.Resource(
            res.dataset, res.year, res.name, res.url, res.size, "0" * 40
        )
        real_retries, real_sleep = ib.RETRIES, ib.time.sleep
        ib.RETRIES, ib.time.sleep = 2, lambda _s: None
        try:
            with self.assertRaises(ib.IngestError), open(os.devnull, "w") as null:
                old, sys.stderr = sys.stderr, null
                try:
                    self.fetch()
                finally:
                    sys.stderr = old
        finally:
            ib.RETRIES, ib.time.sleep = real_retries, real_sleep
        self.assertFalse((self.raw / res.name).exists())
        self.assertFalse((self.raw / (res.name + ".part")).exists())

    def test_cached_files_and_status(self) -> None:
        self.fetch()
        cached = ib.cached_files({"mods"}, self.raw)
        self.assertIsNotNone(cached)
        self.assertEqual(len(cached["mods"]) if cached else 0, 15)
        with open(os.devnull, "w") as null:
            old, sys.stdout = sys.stdout, null
            try:
                ok = ib.status(self.raw, verify_hashes=True)
                (self.raw / "modcontrats2013.json").unlink()
                missing = ib.status(self.raw, verify_hashes=False)
            finally:
                sys.stdout = old
        self.assertEqual(ok, 1)  # contracts, notices, entities not cached
        self.assertEqual(missing, 1)
        self.assertIsNone(ib.cached_files({"mods"}, self.raw))


if __name__ == "__main__":
    unittest.main()
