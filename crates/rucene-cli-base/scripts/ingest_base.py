#!/usr/bin/env python3
"""Ingest every Portal BASE dataset into every index of ``rucene-base``.

Usage::

    python3 -I ingest_base.py fetch  [--cache DIR] [--only ENTITIES] [--force]
    python3 -I ingest_base.py ingest [--cache DIR] [--only ENTITIES] [--root DIR]
                                     [--binary PATH] [--offline DIR] [--limit N]
    python3 -I ingest_base.py status [--cache DIR] [--verify]

No dataset file is kept in the repository: ``fetch`` downloads the files into
the cache (default ``crates/rucene-cli-base/data/base``, git-ignored) when they
are needed, and ``ingest`` calls it by itself when a needed file is not cached.
``status`` lists what is cached. Only the small test fixture under
``scripts/tests/fixtures`` is committed.

Stages (``ingest`` runs all of them, in this order):

1. **Source files.** ``ingest`` uses the cached files when the cache holds
   every file it needs (no network); otherwise it runs ``fetch``. ``fetch``
   resolves the current resource URLs through the dados.gov.pt API (they
   change every week) for the contracts, modifications and announcements of
   2012-2026 and for the entity register, and downloads into ``<cache>/raw``
   only what is missing or stale: a file is kept when its size and SHA-1
   match the API metadata. ``<cache>/raw/manifest.json`` records each file's
   URL, size, SHA-1, publication date and download time. An interrupted
   download resumes from its ``.part`` file (HTTP ``Range``); every request
   is retried with a growing back-off; ``--force`` downloads everything
   again. ``ingest --offline DIR`` reads the files from ``DIR`` instead.
2. **Clean.** Stream-parse each JSON array (never loaded whole), normalise
   strings (HTML entities, NFC, whitespace), placeholders, dates, numbers,
   CPV and NUTS codes, and split competitor strings. Contracts are deduplicated
   by ``idcontrato`` (newest year file first; the first occurrence wins); the
   rows of one notice repeated per contracting entity are merged by
   ``IDAnuncio``.
3. **Derive.** Build the entity records with their aggregates: suppliers,
   contracting entities, invited entities, procedures, CPV codes, execution
   places, NUTS regions. Every entity is first written as normalised JSON
   Lines to ``<cache>/clean/<entity>.jsonl``.
4. **Write and load.** Convert each entity to the ``rucene-base`` bulk input
   format (flat JSON Lines, see the crate documentation) in
   ``<cache>/cli/<entity>/part-NNNN.jsonl`` and run, for every index,
   ``rucene-base <entity> create <root>`` followed by one
   ``rucene-base <entity> add <root> --file PART`` per part. Every index lives
   in its own subdirectory ``<root>/<entity>``.

``ingest`` fails (exit status 1) on any download, parse or CLI error, and when
a full run yields fewer than 2,000,000 unique contracts. ``--limit N`` reads at
most N records from each source file (a sample run: the minimum-count check is
skipped, and with ``--offline`` missing yearly files are allowed). The cache
needs about 14 GB for a full run (1.4 GB downloads, 7.1 GB clean and 4.8 GB bulk
JSON Lines).

Portal BASE data (IMPIC, published on dados.gov.pt) is in the public domain.
Only the Python standard library is used.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import io
import json
import re
import shutil
import subprocess
import sys
import time
import unicodedata
import urllib.error
import urllib.request
import zipfile
import zlib
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable, Iterator
from dataclasses import dataclass, field
from datetime import date
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any, TextIO

# --------------------------------------------------------------------------
# Constants
# --------------------------------------------------------------------------

API = "https://dados.gov.pt/api/1/datasets/{slug}/"
USER_AGENT = "rucene-base-ingest/1.0 (+python-urllib)"
SLUG_CONTRACTS = "contratos-publicos-portal-base-impic-contratos-de-2012-a-2026"
SLUG_MODS = (
    "contratos-publicos-portal-base-impic-modificacoes-contratuais-de-2012-a-2026"
)
SLUG_NOTICES = "contratos-publicos-portal-base-impic-anuncios-de-2012-a-2026"
SLUG_ENTITIES = "contratos-publicos-portal-base-impic-entidades"
YEARS = tuple(range(2012, 2027))
MIN_CONTRACTS = 2_000_000
BATCH = 250_000  # documents per bulk file (one CLI commit each)
BUCKETS = 64  # procedure aggregation buckets
CHUNK = 4 * 1024 * 1024
RETRIES = 5
INT32_MAX = 2**31 - 1

ENTITIES = (
    "contratos",
    "fornecedores",
    "adjudicantes",
    "procedimentos",
    "cpv",
    "locais",
    "nuts",
    "modificacoes",
    "anuncios",
    "convidadas",
)
# Source datasets each index needs.
NEEDS: dict[str, frozenset[str]] = {
    "contratos": frozenset({"contracts", "mods"}),
    "fornecedores": frozenset({"contracts", "entities"}),
    "adjudicantes": frozenset({"contracts", "entities"}),
    "procedimentos": frozenset({"contracts"}),
    "cpv": frozenset({"contracts"}),
    "locais": frozenset({"contracts"}),
    "nuts": frozenset({"contracts"}),
    "modificacoes": frozenset({"mods"}),
    "anuncios": frozenset({"notices"}),
    "convidadas": frozenset({"contracts"}),
}
# Resource file names per dataset; group 1 is the year.
PATTERNS: dict[str, re.Pattern[str]] = {
    "contracts": re.compile(r"contratos(\d{4})\.(?:zip|json)"),
    "mods": re.compile(r"modcontrats(\d{4})\.json"),
    "notices": re.compile(r"anuncios(\d{4})\.json"),
    "entities": re.compile(r"entidades\.json"),
}
SLUGS = {
    "contracts": SLUG_CONTRACTS,
    "mods": SLUG_MODS,
    "notices": SLUG_NOTICES,
    "entities": SLUG_ENTITIES,
}

CONTRACT_KEYS = frozenset(
    [
        "idcontrato",
        "nAnuncio",
        "TipoAnuncio",
        "idINCM",
        "tipoContrato",
        "idprocedimento",
        "tipoprocedimento",
        "objectoContrato",
        "descContrato",
        "adjudicante",
        "adjudicatarios",
        "dataPublicacao",
        "dataCelebracaoContrato",
        "precoContratual",
        "CPV",
        "prazoExecucao",
        "localExecucao",
        "fundamentacao",
        "ProcedimentoCentralizado",
        "numAcordoQuadro",
        "DescrAcordoQuadro",
        "precoBaseProcedimento",
        "dataDecisaoAdjudicacao",
        "dataFechoContrato",
        "PrecoTotalEfetivo",
        "regime",
        "justifNReducEscrContrato",
        "tipoFimContrato",
        "CritMateriais",
        "concorrentes",
        "linkPecasProc",
        "Observacoes",
        "ContratEcologico",
        "Ano",
        "fundamentAjusteDireto",
        "NUTs",
        "Lotes",
        "compraConjunta",
        "TipoMEC",
        "entidadesConvidadas",
        "CausasAlteraPrazo",
        "CausasAlteraPreco",
        "IDAnuncio",
        "estadoContrato",
        "precoEstimadoProc",
        "ocorrencias",
    ]
)
MOD_KEYS = frozenset(
    [
        "idcontrato",
        "modifContratoFundamento",
        "modifContratoTipoAto",
        "modifContratoData",
        "modifContratoPrecoAlterado",
        "modifPrazoExecucao",
        "modifDataPublicacao",
        "Ano",
        "modifEntidadesSubcontrat",
        "modifJustNContratEscrito",
    ]
)
NOTICE_KEYS = frozenset(
    [
        "nAnuncio",
        "IdIncm",
        "dataPublicacao",
        "nifEntidade",
        "designacaoEntidade",
        "descricaoAnuncio",
        "url",
        "numDR",
        "serie",
        "tipoActo",
        "tiposContrato",
        "PrecoBase",
        "CPVs",
        "modeloAnuncio",
        "Ano",
        "CriterAmbient",
        "PrazoPropostas",
        "PecasProcedimento",
        "DataLimitePropostas",
        "Lotes",
        "AcordoQuadro",
        "sistemaAquisicaoDinamico",
        "anuncioAnterior",
        "IDAnuncio",
    ]
)
ENTITY_KEYS = frozenset(
    [
        "nifEntidade",
        "desigEntidade",
        "numContratos",
        "totAdjudicatario",
        "totValorContratIni",
        "totAdjudicante",
        "totAdjudicanteValorContratIni",
        "descPais",
        "AliasPais",
    ]
)

FREE_TEXT_PLACEHOLDERS = frozenset({"n/a", ".", "-"})
PRICE_PLACEHOLDERS = frozenset({"inexistente"})
TERM_SENTINEL = 99_999_999

WS_RE = re.compile(r"\s+")
HSPACE_RE = re.compile(r"[^\S\n]+")
ISO_DATE_RE = re.compile(r"(\d{4})-(\d{2})-(\d{2})(?:[ T]\d{2}:\d{2}:\d{2})?")
PT_DATE_RE = re.compile(r"(\d{2})/(\d{2})/(\d{4})(?: \d{2}:\d{2}:\d{2})?")
CPV_RE = re.compile(r"\d{8}-\d")
NUTS_RE = re.compile(r"[A-Z]{2}[0-9A-Z]{0,3}")
INT_RE = re.compile(r"[+-]?\d+")


class IngestError(RuntimeError):
    """A failure that stops the run; the message names file, record and value."""


# --------------------------------------------------------------------------
# Stage report
# --------------------------------------------------------------------------


@dataclass
class Report:
    """Counters of every normalisation and anomaly, printed at the end."""

    events: Counter[str] = field(default_factory=Counter)
    samples: dict[str, list[str]] = field(default_factory=lambda: defaultdict(list))

    def note(self, key: str, sample: str | None = None) -> None:
        """Count one event; keep up to 5 samples of it."""
        self.events[key] += 1
        if sample is not None and len(self.samples[key]) < 5:
            self.samples[key].append(sample)

    def lines(self) -> list[str]:
        """Return the report as sorted human-readable lines."""
        out = []
        for key in sorted(self.events):
            line = f"  {key}: {self.events[key]:,}"
            if self.samples.get(key):
                line += "  e.g. " + ", ".join(repr(s) for s in self.samples[key])
            out.append(line)
        return out


# --------------------------------------------------------------------------
# Cleaning helpers (pure)
# --------------------------------------------------------------------------


def _as_text(value: Any, where: str) -> str | None:
    if value is None:
        return None
    if isinstance(value, bool):
        raise IngestError(f"{where}: expected a string, got {value!r}")
    if isinstance(value, (int, Decimal, float)):
        return str(value)
    if not isinstance(value, str):
        raise IngestError(f"{where}: expected a string, got {type(value).__name__}")
    return value


def clean_name(value: Any, where: str = "value") -> str | None:
    """Normalise a short string: HTML entities, NFC, one space between words.

    Returns ``None`` for ``None`` and for strings that are empty after trim.
    """
    text = _as_text(value, where)
    if text is None:
        return None
    text = unicodedata.normalize("NFC", html.unescape(text))
    text = WS_RE.sub(" ", text).strip()
    return text or None


def clean_text(
    value: Any, where: str = "value", placeholders: frozenset[str] = frozenset()
) -> str | None:
    """Normalise free text, keeping its line breaks.

    HTML entities are decoded, the text is NFC-normalised, CR/CRLF become LF,
    runs of horizontal whitespace become one space, every line is trimmed and
    runs of blank lines collapse to one. Values in ``placeholders``
    (compared case-insensitively after trim) become ``None``.
    """
    text = _as_text(value, where)
    if text is None:
        return None
    text = unicodedata.normalize("NFC", html.unescape(text))
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    lines = [HSPACE_RE.sub(" ", line).strip() for line in text.split("\n")]
    out: list[str] = []
    for line in lines:
        if line or (out and out[-1]):
            out.append(line)
    text = "\n".join(out).strip()
    if not text or text.casefold() in placeholders:
        return None
    return text


def clean_code(value: Any, where: str = "value") -> str | None:
    """Normalise an identifier or code: trim and collapse whitespace, no unescape."""
    text = _as_text(value, where)
    if text is None:
        return None
    text = WS_RE.sub(" ", unicodedata.normalize("NFC", text)).strip()
    return text or None


def clean_nif(value: Any, where: str = "nif") -> str | None:
    """Return a NIF, or ``None`` for the placeholders ``-`` and empty."""
    text = clean_code(value, where)
    if text is None or text == "-":
        return None
    return text


def clean_date(value: Any, report: Report, key: str) -> str | None:
    """Return ``YYYY-MM-DD`` from ``YYYY-MM-DD[ HH:MM:SS]`` or ``DD/MM/YYYY[ HH:MM:SS]``.

    Empty values give ``None``; an unparsable or impossible date gives
    ``None`` and is counted under ``key``.
    """
    text = clean_code(value, key)
    if text is None:
        return None
    m = ISO_DATE_RE.fullmatch(text)
    if m:
        y, mo, d = int(m[1]), int(m[2]), int(m[3])
    else:
        m = PT_DATE_RE.fullmatch(text)
        if not m:
            report.note(f"{key}: invalid date (dropped)", text)
            return None
        d, mo, y = int(m[1]), int(m[2]), int(m[3])
    try:
        return date(y, mo, d).isoformat()
    except ValueError:
        report.note(f"{key}: invalid date (dropped)", text)
        return None


def to_decimal(value: Any, report: Report, key: str) -> Decimal | None:
    """Parse a JSON number or numeric string into a finite ``Decimal``."""
    if value is None or isinstance(value, bool):
        if isinstance(value, bool):
            report.note(f"{key}: invalid number (dropped)", repr(value))
        return None
    if isinstance(value, Decimal):
        return value
    if isinstance(value, (int, float)):
        return Decimal(str(value))
    text = clean_code(value, key)
    if text is None:
        return None
    if text.casefold() in PRICE_PLACEHOLDERS:
        report.note(f"{key}: placeholder {text!r} -> null")
        return None
    try:
        dec = Decimal(text)
    except InvalidOperation:
        report.note(f"{key}: invalid number (dropped)", text)
        return None
    if not dec.is_finite():
        report.note(f"{key}: invalid number (dropped)", text)
        return None
    return dec


def clean_money(
    value: Any, report: Report, key: str, zero_is_null: bool = False
) -> float | None:
    """Return a price as ``float``; ``0`` becomes ``None`` when ``zero_is_null``."""
    dec = to_decimal(value, report, key)
    if dec is None:
        return None
    if zero_is_null and dec == 0:
        report.note(f"{key}: 0 -> null")
        return None
    return float(dec)


def clean_int(
    value: Any, report: Report, key: str, sentinel: int | None = None
) -> int | None:
    """Return a 32-bit integer from a JSON integer or a string of digits."""
    if value is None:
        return None
    if isinstance(value, bool):
        report.note(f"{key}: invalid integer (dropped)", repr(value))
        return None
    if isinstance(value, int):
        number = value
    elif isinstance(value, Decimal) and value == value.to_integral_value():
        number = int(value)
    else:
        text = clean_code(value, key)
        if text is None:
            return None
        if not INT_RE.fullmatch(text):
            report.note(f"{key}: invalid integer (dropped)", text)
            return None
        number = int(text)
    if sentinel is not None and number == sentinel:
        report.note(f"{key}: {sentinel} -> null")
        return None
    if not -INT32_MAX - 1 <= number <= INT32_MAX:
        report.note(f"{key}: integer out of 32-bit range (dropped)", str(number))
        return None
    return number


def clean_flag(value: Any, report: Report, key: str) -> str | None:
    """Return ``"Sim"``/``"Não"`` from the source's ``Sim``/``Não`` strings."""
    text = clean_name(value, key)
    if text is None:
        return None
    folded = text.casefold()
    if folded == "sim":
        return "Sim"
    if folded in ("não", "nao"):
        return "Não"
    report.note(f"{key}: invalid flag (dropped)", text)
    return None


def clean_list(value: Any, where: str) -> list[Any]:
    """Return a source list, ``[]`` for ``None``; any other type is an error."""
    if value is None:
        return []
    if not isinstance(value, list):
        raise IngestError(f"{where}: expected a list, got {type(value).__name__}")
    return value


def split_competitor(
    value: Any, where: str = "concorrentes[]"
) -> dict[str, str | None] | None:
    """Split a ``"NIF-Name"`` string at its first ``-`` into ``{"nif", "nome"}``.

    A keyless competitor is written with the NIF placeholder ``-``, so its
    string starts with ``--`` (``"--Name"``); its NIF is ``None``. A string
    without ``-`` is taken as a name without NIF.
    """
    text = clean_name(value, where)
    if text is None:
        return None
    if text.startswith("--"):
        return {"nif": None, "nome": clean_name(text[2:])}
    head, sep, tail = text.partition("-")
    if not sep:
        return {"nif": None, "nome": text}
    return {"nif": clean_nif(head), "nome": clean_name(tail)}


def split_code_desc(
    value: Any, where: str, code_re: re.Pattern[str]
) -> dict[str, str | None] | None:
    """Split ``"CODE - Description"`` into ``{"codigo", "descricao"}``.

    A value without `` - `` is accepted as a bare code when it matches
    ``code_re``; otherwise the code is ``None`` and the text is the description.
    """
    text = clean_name(value, where)
    if text is None:
        return None
    head, sep, tail = text.partition(" - ")
    code = head.strip().upper()
    if sep and code_re.fullmatch(code):
        return {"codigo": code, "descricao": clean_name(tail)}
    if not sep and code_re.fullmatch(code):
        return {"codigo": code, "descricao": None}
    return {"codigo": None, "descricao": text}


def clean_nuts_code(value: Any, report: Report, key: str) -> str | None:
    """Return an upper-case NUTS code (``PT``, ``PT170``, ``PTZZZ``) or ``None``."""
    text = clean_code(value, key)
    if text is None:
        return None
    code = text.upper()
    if not NUTS_RE.fullmatch(code):
        report.note(f"{key}: invalid NUTS code (dropped)", text)
        return None
    return code


def clean_cpv_code(value: Any, report: Report, key: str) -> str | None:
    """Return a CPV code ``NNNNNNNN-N`` or ``None``."""
    text = clean_code(value, key)
    if text is None:
        return None
    if not CPV_RE.fullmatch(text):
        report.note(f"{key}: invalid CPV code (dropped)", text)
        return None
    return text


def distinct(values: Iterable[Any]) -> list[Any]:
    """Return the non-null values in first-seen order, without repeats."""
    seen: set[Any] = set()
    out = []
    for v in values:
        if v is None or v in seen:
            continue
        seen.add(v)
        out.append(v)
    return out


def check_keys(
    record: dict[str, Any], known: frozenset[str], dataset: str, report: Report
) -> None:
    """Count keys the profile (#531) did not list; they are not ingested."""
    for key in record.keys() - known:
        report.note(f"{dataset}: unknown key ignored", key)


# --------------------------------------------------------------------------
# Record cleaners (pure; output = normalised intermediate records)
# --------------------------------------------------------------------------


def _party(e: Any, where: str, report: Report) -> dict[str, Any]:
    if not isinstance(e, dict):
        raise IngestError(f"{where}: expected an object")
    return {
        "nif": clean_nif(e.get("nifEntContrato"), where),
        "nome": clean_name(e.get("descEntContrato"), where),
        "codPais": clean_code(e.get("codPaisEntContrato"), where),
        "nutsCod": clean_nuts_code(
            e.get("NUTsCod"), report, f"contratos.{where}.NUTsCod"
        ),
        "nutsDesc": clean_name(e.get("NUTsDesc"), where),
        "tipoEntidade": clean_name(e.get("tipoEntidade"), where),
        "atividadeEntidade": clean_name(e.get("atividadeEntidade"), where),
        "pme": clean_flag(
            e.get("adjudicatarioPME"), report, f"contratos.{where}.adjudicatarioPME"
        ),
    }


def clean_contract(raw: dict[str, Any], report: Report) -> dict[str, Any]:
    """Return the normalised intermediate record of one contract.

    Scalars keep their source names; nested lists become lists of objects
    with short field names. Raises ``IngestError`` when ``idcontrato`` is
    missing.
    """
    check_keys(raw, CONTRACT_KEYS, "contratos", report)
    cid = clean_code(raw.get("idcontrato"), "idcontrato")
    if cid is None:
        raise IngestError("contract without idcontrato")
    r = report
    p = "contratos."
    c: dict[str, Any] = {
        "idcontrato": cid,
        "idprocedimento": clean_code(raw.get("idprocedimento"), "idprocedimento"),
        "tipoprocedimento": clean_name(raw.get("tipoprocedimento")),
        "tipoContrato": distinct(
            clean_name(v) for v in clean_list(raw.get("tipoContrato"), "tipoContrato")
        ),
        "objectoContrato": clean_text(raw.get("objectoContrato")),
        "descContrato": clean_text(raw.get("descContrato")),
        "dataPublicacao": clean_date(
            raw.get("dataPublicacao"), r, p + "dataPublicacao"
        ),
        "dataCelebracaoContrato": clean_date(
            raw.get("dataCelebracaoContrato"), r, p + "dataCelebracaoContrato"
        ),
        "dataDecisaoAdjudicacao": clean_date(
            raw.get("dataDecisaoAdjudicacao"), r, p + "dataDecisaoAdjudicacao"
        ),
        "dataFechoContrato": clean_date(
            raw.get("dataFechoContrato"), r, p + "dataFechoContrato"
        ),
        "precoContratual": clean_money(
            raw.get("precoContratual"), r, p + "precoContratual"
        ),
        "precoBaseProcedimento": clean_money(
            raw.get("precoBaseProcedimento"), r, p + "precoBaseProcedimento", True
        ),
        "PrecoTotalEfetivo": clean_money(
            raw.get("PrecoTotalEfetivo"), r, p + "PrecoTotalEfetivo", True
        ),
        "precoEstimadoProc": clean_money(
            raw.get("precoEstimadoProc"), r, p + "precoEstimadoProc", True
        ),
        "prazoExecucao": clean_int(
            raw.get("prazoExecucao"), r, p + "prazoExecucao", TERM_SENTINEL
        ),
        "Ano": clean_int(raw.get("Ano"), r, p + "Ano"),
        "regime": clean_name(raw.get("regime")),
        "fundamentacao": clean_name(raw.get("fundamentacao")),
        "fundamentAjusteDireto": clean_name(raw.get("fundamentAjusteDireto")),
        "tipoFimContrato": clean_name(raw.get("tipoFimContrato")),
        "TipoMEC": clean_name(raw.get("TipoMEC")),
        "estadoContrato": clean_name(raw.get("estadoContrato")),
        "ProcedimentoCentralizado": clean_flag(
            raw.get("ProcedimentoCentralizado"), r, p + "ProcedimentoCentralizado"
        ),
        "CritMateriais": clean_flag(raw.get("CritMateriais"), r, p + "CritMateriais"),
        "ContratEcologico": clean_flag(
            raw.get("ContratEcologico"), r, p + "ContratEcologico"
        ),
        "compraConjunta": clean_flag(
            raw.get("compraConjunta"), r, p + "compraConjunta"
        ),
        "ocorrencias": raw.get("ocorrencias")
        if isinstance(raw.get("ocorrencias"), bool)
        else None,
        "numAcordoQuadro": clean_code(raw.get("numAcordoQuadro")),
        "DescrAcordoQuadro": clean_text(raw.get("DescrAcordoQuadro")),
        "nAnuncio": clean_code(raw.get("nAnuncio")),
        "IDAnuncio": clean_code(raw.get("IDAnuncio")),
        "idINCM": clean_code(raw.get("idINCM")),
        "TipoAnuncio": clean_name(raw.get("TipoAnuncio")),
        "linkPecasProc": clean_code(raw.get("linkPecasProc")),
        "Observacoes": clean_text(
            raw.get("Observacoes"), placeholders=FREE_TEXT_PLACEHOLDERS
        ),
        "CausasAlteraPrazo": clean_text(
            raw.get("CausasAlteraPrazo"), placeholders=FREE_TEXT_PLACEHOLDERS
        ),
        "CausasAlteraPreco": clean_text(
            raw.get("CausasAlteraPreco"), placeholders=FREE_TEXT_PLACEHOLDERS
        ),
    }
    if c["numAcordoQuadro"] == "1":
        r.note(p + "numAcordoQuadro: '1' -> null")
        c["numAcordoQuadro"] = None
    if c["idINCM"] == "-1":
        r.note(p + "idINCM: '-1' -> null")
        c["idINCM"] = None
    c["adjudicante"] = [
        _party(e, "adjudicante[]", r)
        for e in clean_list(raw.get("adjudicante"), "adjudicante")
    ]
    c["adjudicatarios"] = [
        _party(e, "adjudicatarios[]", r)
        for e in clean_list(raw.get("adjudicatarios"), "adjudicatarios")
    ]
    cpvs = []
    for e in clean_list(raw.get("CPV"), "CPV"):
        if not isinstance(e, dict):
            raise IngestError("CPV[]: expected an object")
        cpvs.append(
            {
                "cpv": clean_cpv_code(e.get("cpvID"), r, p + "CPV[].cpvID"),
                "descricao": clean_name(e.get("cpvdescricao")),
                "valor": clean_money(e.get("cpvValor"), r, p + "CPV[].cpvValor"),
                "objeto": clean_name(e.get("CPVObjeto")),
            }
        )
    c["CPV"] = cpvs
    c["localExecucao"] = distinct(
        clean_name(v) for v in clean_list(raw.get("localExecucao"), "localExecucao")
    )
    nuts = []
    for v in clean_list(raw.get("NUTs"), "NUTs"):
        entry = split_code_desc(v, "NUTs[]", NUTS_RE)
        if entry is None:
            continue
        if entry["codigo"] is None:
            r.note(p + "NUTs[]: no code (dropped)", entry["descricao"])
            continue
        nuts.append(entry)
    c["NUTs"] = nuts
    c["justifNReducEscrContrato"] = distinct(
        clean_text(v)
        for v in clean_list(
            raw.get("justifNReducEscrContrato"), "justifNReducEscrContrato"
        )
    )
    comps = []
    for v in clean_list(raw.get("concorrentes"), "concorrentes"):
        entry = split_competitor(v)
        if entry is not None:
            comps.append(entry)
    c["concorrentes"] = comps
    invited = []
    for e in clean_list(raw.get("entidadesConvidadas"), "entidadesConvidadas"):
        if not isinstance(e, dict):
            raise IngestError("entidadesConvidadas[]: expected an object")
        invited.append(
            {
                "nif": clean_nif(e.get("NIPCEntidade")),
                "nome": clean_name(e.get("Entidade")),
                "aliasPais": clean_code(e.get("AliasPais")),
                "pais": clean_name(e.get("Pais")),
            }
        )
    c["entidadesConvidadas"] = invited
    lots = []
    for e in clean_list(raw.get("Lotes"), "Lotes"):
        if not isinstance(e, dict):
            raise IngestError("Lotes[]: expected an object")
        lots.append(
            {
                "numLot": clean_code(e.get("numLot")),
                "designacaoLote": clean_text(e.get("designacaoLote")),
                "descricaoLote": clean_text(e.get("descricaoLote")),
                "criterioLote": clean_name(e.get("criterioLote")),
                "criterioAdjudicacaoTipo": clean_name(e.get("criterioAdjudicacaoTipo")),
                "criterioAdjudicacaoTipoFator": clean_name(
                    e.get("criterioAdjudicacaoTipoFator")
                ),
                "criterioAdjudicacaoFator": clean_name(
                    e.get("criterioAdjudicacaoFator")
                ),
                "criterioAdjudicacaoPonderacao": clean_money(
                    e.get("criterioAdjudicacaoPonderacao"),
                    r,
                    p + "Lotes[].criterioAdjudicacaoPonderacao",
                ),
                "NumPropostas": clean_int(
                    e.get("NumPropostas"), r, p + "Lotes[].NumPropostas"
                ),
            }
        )
    c["Lotes"] = lots
    return c


def clean_modification(raw: dict[str, Any], report: Report) -> dict[str, Any]:
    """Return the normalised record of one contract modification (no sequence yet)."""
    check_keys(raw, MOD_KEYS, "modificacoes", report)
    cid = clean_code(raw.get("idcontrato"), "idcontrato")
    if cid is None:
        raise IngestError("modification without idcontrato")
    r = report
    p = "modificacoes."
    subs = []
    for e in clean_list(
        raw.get("modifEntidadesSubcontrat"), "modifEntidadesSubcontrat"
    ):
        if not isinstance(e, dict):
            raise IngestError("modifEntidadesSubcontrat[]: expected an object")
        subs.append(
            {
                "nif": clean_nif(e.get("NIPCEntidade")),
                "nome": clean_name(e.get("Entidade")),
            }
        )
    return {
        "idcontrato": cid,
        "modifContratoFundamento": clean_name(raw.get("modifContratoFundamento")),
        "modifContratoTipoAto": clean_name(raw.get("modifContratoTipoAto")),
        "modifContratoData": clean_date(
            raw.get("modifContratoData"), r, p + "modifContratoData"
        ),
        "modifDataPublicacao": clean_date(
            raw.get("modifDataPublicacao"), r, p + "modifDataPublicacao"
        ),
        "modifContratoPrecoAlterado": clean_money(
            raw.get("modifContratoPrecoAlterado"), r, p + "modifContratoPrecoAlterado"
        ),
        "modifPrazoExecucao": clean_int(
            raw.get("modifPrazoExecucao"), r, p + "modifPrazoExecucao", TERM_SENTINEL
        ),
        "Ano": clean_int(raw.get("Ano"), r, p + "Ano"),
        "modifEntidadesSubcontrat": subs,
        "modifJustNContratEscrito": clean_name(raw.get("modifJustNContratEscrito")),
    }


def sequence_modifications(mods: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Assign ``sequencia`` 1..n per ``idcontrato``.

    Order within a contract: ``modifContratoData``, ``modifDataPublicacao``
    (missing dates last), then the order of ``mods``. Returns the records
    sorted by ``(idcontrato, sequencia)``.
    """
    groups: dict[str, list[tuple[int, dict[str, Any]]]] = defaultdict(list)
    for i, m in enumerate(mods):
        groups[m["idcontrato"]].append((i, m))
    out = []
    for cid in sorted(groups, key=_id_sort_key):
        rows = sorted(
            groups[cid],
            key=lambda t: (
                t[1]["modifContratoData"] is None,
                t[1]["modifContratoData"] or "",
                t[1]["modifDataPublicacao"] is None,
                t[1]["modifDataPublicacao"] or "",
                t[0],
            ),
        )
        for seq, (_, m) in enumerate(rows, start=1):
            out.append({**m, "sequencia": seq})
    return out


def clean_notice(raw: dict[str, Any], report: Report) -> dict[str, Any]:
    """Return the normalised record of one announcement."""
    check_keys(raw, NOTICE_KEYS, "anuncios", report)
    nid = clean_code(raw.get("IDAnuncio"), "IDAnuncio")
    if nid is None:
        raise IngestError("announcement without IDAnuncio")
    r = report
    p = "anuncios."
    cpvs = []
    for v in clean_list(raw.get("CPVs"), "CPVs"):
        entry = split_code_desc(v, "CPVs[]", CPV_RE)
        if entry is None:
            continue
        if entry["codigo"] is None:
            r.note(p + "CPVs[]: no code", entry["descricao"])
        cpvs.append({"cpv": entry["codigo"], "descricao": entry["descricao"]})
    lots = []
    for e in clean_list(raw.get("Lotes"), "Lotes"):
        if not isinstance(e, dict):
            raise IngestError("Lotes[]: expected an object")
        lots.append(
            {
                "numLot": clean_code(e.get("numLot")),
                "designacaoLote": clean_text(e.get("designacaoLote")),
                "descricaoLote": clean_text(e.get("descricaoLote")),
                "cpv": clean_cpv_code(e.get("cpvAliasId"), r, p + "Lotes[].cpvAliasId"),
                "cpvDescricao": clean_name(e.get("cpvDescription")),
                "valorBaseLote": clean_money(
                    e.get("valorBaseLote"), r, p + "Lotes[].valorBaseLote"
                ),
                "criterioLote": clean_name(e.get("criterioLote")),
                "criterioLotePercentagem": clean_money(
                    e.get("criterioLotePercentagem"),
                    r,
                    p + "Lotes[].criterioLotePercentagem",
                ),
                "criterioAquisicaoTipo": clean_name(e.get("criterioAquisicaoTipo")),
                "criterioAquisicaoTipoFator": clean_name(
                    e.get("criterioAquisicaoTipoFator")
                ),
                "criterioAquisicaoPonderacao": clean_name(
                    e.get("criterioAquisicaoPonderacao")
                ),
            }
        )
    previous = []
    for e in clean_list(raw.get("anuncioAnterior"), "anuncioAnterior"):
        if not isinstance(e, dict):
            raise IngestError("anuncioAnterior[]: expected an object")
        previous.append(
            {
                "IDAnuncio": clean_code(e.get("antIDAnuncio")),
                "nAnuncio": clean_code(e.get("antNAnuncio")),
                "IdIncm": clean_code(e.get("antIdIncm")),
                "dataPublicacao": clean_date(
                    e.get("antDataPublicacao"),
                    r,
                    p + "anuncioAnterior[].antDataPublicacao",
                ),
                "tipoActo": clean_name(e.get("antTipoActo")),
            }
        )
    incm = clean_code(raw.get("IdIncm"))
    if incm == "-1":
        r.note(p + "IdIncm: '-1' -> null")
        incm = None
    return {
        "IDAnuncio": nid,
        "nAnuncio": clean_code(raw.get("nAnuncio")),
        "IdIncm": incm,
        "dataPublicacao": clean_date(
            raw.get("dataPublicacao"), r, p + "dataPublicacao"
        ),
        "entidades": [
            {
                "nif": clean_nif(raw.get("nifEntidade")),
                "nome": clean_name(raw.get("designacaoEntidade")),
            }
        ],
        "descricaoAnuncio": clean_text(raw.get("descricaoAnuncio")),
        "url": clean_code(raw.get("url")),
        "numDR": clean_code(raw.get("numDR")),
        "serie": clean_code(raw.get("serie")),
        "tipoActo": clean_name(raw.get("tipoActo")),
        "tiposContrato": distinct(
            clean_name(v) for v in clean_list(raw.get("tiposContrato"), "tiposContrato")
        ),
        "PrecoBase": clean_money(raw.get("PrecoBase"), r, p + "PrecoBase", True),
        "CPVs": cpvs,
        "modeloAnuncio": clean_name(raw.get("modeloAnuncio")),
        "Ano": clean_int(raw.get("Ano"), r, p + "Ano"),
        "CriterAmbient": clean_flag(raw.get("CriterAmbient"), r, p + "CriterAmbient"),
        "PrazoPropostas": clean_int(raw.get("PrazoPropostas"), r, p + "PrazoPropostas"),
        "PecasProcedimento": clean_code(raw.get("PecasProcedimento")),
        "DataLimitePropostas": clean_date(
            raw.get("DataLimitePropostas"), r, p + "DataLimitePropostas"
        ),
        "Lotes": lots,
        "AcordoQuadro": clean_flag(raw.get("AcordoQuadro"), r, p + "AcordoQuadro"),
        "sistemaAquisicaoDinamico": clean_flag(
            raw.get("sistemaAquisicaoDinamico"), r, p + "sistemaAquisicaoDinamico"
        ),
        "anuncioAnterior": previous,
    }


# Fields of duplicate announcement rows that are merged (union, first-seen
# order); any other differing field keeps the first row's value.
NOTICE_UNION_FIELDS = ("entidades", "tiposContrato", "CPVs", "Lotes", "anuncioAnterior")


def merge_notices(
    notices: Iterable[dict[str, Any]], report: Report
) -> list[dict[str, Any]]:
    """Merge the rows of one year file that share an ``IDAnuncio``.

    The source repeats a notice once per contracting entity (joint
    procurement). The merged record keeps the first row and appends, in
    first-seen order, the distinct entries of ``NOTICE_UNION_FIELDS`` from
    the later rows; other differing fields keep the first value and are
    counted. Records are returned in first-seen order.
    """
    merged: dict[str, dict[str, Any]] = {}
    for n in notices:
        base = merged.get(n["IDAnuncio"])
        if base is None:
            merged[n["IDAnuncio"]] = n
            continue
        report.note("anuncios: duplicate IDAnuncio rows merged", n["IDAnuncio"])
        for key, value in n.items():
            if key in NOTICE_UNION_FIELDS:
                for item in value:
                    if item not in base[key]:
                        base[key].append(item)
            elif base[key] is None and value is not None:
                base[key] = value
            elif value is not None and value != base[key]:
                report.note(
                    f"anuncios: duplicate rows differ in {key} (first kept)",
                    n["IDAnuncio"],
                )
    return list(merged.values())


def clean_entity_row(raw: dict[str, Any], report: Report) -> dict[str, Any]:
    """Return the normalised record of one row of the ``entidades`` register."""
    check_keys(raw, ENTITY_KEYS, "entidades", report)
    r = report
    p = "entidades."
    return {
        "nif": clean_nif(raw.get("nifEntidade")),
        "nome": clean_name(raw.get("desigEntidade")),
        "numContratos": clean_int(raw.get("numContratos"), r, p + "numContratos"),
        "totAdjudicatario": clean_int(
            raw.get("totAdjudicatario"), r, p + "totAdjudicatario"
        ),
        "totValorContratIni": clean_money(
            raw.get("totValorContratIni"), r, p + "totValorContratIni"
        ),
        "totAdjudicante": clean_int(raw.get("totAdjudicante"), r, p + "totAdjudicante"),
        "totAdjudicanteValorContratIni": clean_money(
            raw.get("totAdjudicanteValorContratIni"),
            r,
            p + "totAdjudicanteValorContratIni",
        ),
        "descPais": clean_name(raw.get("descPais")),
        "AliasPais": clean_code(raw.get("AliasPais")),
    }


# --------------------------------------------------------------------------
# Conversion to the CLI bulk format (pure)
# --------------------------------------------------------------------------


def _flags_out(value: bool | None) -> str | None:
    if value is None:
        return None
    return "Sim" if value else "Não"


def contract_to_cli(c: dict[str, Any], num_mods: int) -> dict[str, Any]:
    """Flatten a normalised contract into the ``contratos`` bulk record."""
    cpv_principal = next(
        (
            e["cpv"]
            for e in c["CPV"]
            if e["cpv"] is not None and e["objeto"] == "Principal"
        ),
        None,
    )
    lots = c["Lotes"]
    proposals = [e["NumPropostas"] for e in lots if e["NumPropostas"] is not None]
    out = {
        k: c[k]
        for k in [
            "idcontrato",
            "idprocedimento",
            "tipoprocedimento",
            "tipoContrato",
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
            "numAcordoQuadro",
            "DescrAcordoQuadro",
            "nAnuncio",
            "IDAnuncio",
            "idINCM",
            "TipoAnuncio",
            "linkPecasProc",
            "Observacoes",
            "CausasAlteraPrazo",
            "CausasAlteraPreco",
            "justifNReducEscrContrato",
            "localExecucao",
        ]
    }
    out["ocorrencias"] = _flags_out(c["ocorrencias"])
    out["adjudicanteNif"] = distinct(e["nif"] for e in c["adjudicante"])
    out["adjudicanteNome"] = distinct(e["nome"] for e in c["adjudicante"])
    out["adjudicatarioNif"] = distinct(e["nif"] for e in c["adjudicatarios"])
    out["adjudicatarioNome"] = distinct(e["nome"] for e in c["adjudicatarios"])
    out["cpv"] = distinct(e["cpv"] for e in c["CPV"])
    out["cpvDescricao"] = distinct(e["descricao"] for e in c["CPV"])
    out["cpvPrincipal"] = cpv_principal
    out["nuts"] = distinct(e["codigo"] for e in c["NUTs"])
    out["concorrenteNif"] = distinct(e["nif"] for e in c["concorrentes"])
    out["concorrenteNome"] = distinct(e["nome"] for e in c["concorrentes"])
    out["convidadaNif"] = distinct(e["nif"] for e in c["entidadesConvidadas"])
    out["convidadaNome"] = distinct(e["nome"] for e in c["entidadesConvidadas"])
    out["loteDesignacao"] = distinct(
        v for e in lots for v in (e["designacaoLote"], e["descricaoLote"])
    )
    out["criterioLote"] = distinct(e["criterioLote"] for e in lots)
    out["criterioAdjudicacaoTipo"] = distinct(
        e["criterioAdjudicacaoTipo"] for e in lots
    )
    out["criterioAdjudicacaoTipoFator"] = distinct(
        e["criterioAdjudicacaoTipoFator"] for e in lots
    )
    out["criterioAdjudicacaoFator"] = distinct(
        e["criterioAdjudicacaoFator"] for e in lots
    )
    out["numPropostas"] = max(proposals) if proposals else None
    out["numModificacoes"] = num_mods
    return compact(out)


def modification_to_cli(m: dict[str, Any]) -> dict[str, Any]:
    """Flatten a sequenced modification into the ``modificacoes`` bulk record."""
    out = {
        k: m[k]
        for k in [
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
        ]
    }
    out["subcontratadaNif"] = distinct(e["nif"] for e in m["modifEntidadesSubcontrat"])
    out["subcontratadaNome"] = distinct(
        e["nome"] for e in m["modifEntidadesSubcontrat"]
    )
    return compact(out)


def notice_to_cli(n: dict[str, Any]) -> dict[str, Any]:
    """Flatten an announcement into the ``anuncios`` bulk record."""
    out = {
        k: n[k]
        for k in [
            "IDAnuncio",
            "nAnuncio",
            "IdIncm",
            "dataPublicacao",
            "descricaoAnuncio",
            "url",
            "numDR",
            "serie",
            "tipoActo",
            "tiposContrato",
            "PrecoBase",
            "modeloAnuncio",
            "Ano",
            "CriterAmbient",
            "PrazoPropostas",
            "PecasProcedimento",
            "DataLimitePropostas",
            "AcordoQuadro",
            "sistemaAquisicaoDinamico",
        ]
    }
    # The bulk format v1 has single-valued nifEntidade/designacaoEntidade: the
    # first contracting entity of a merged notice is sent.
    first = n["entidades"][0] if n["entidades"] else {"nif": None, "nome": None}
    out["nifEntidade"] = first["nif"]
    out["designacaoEntidade"] = first["nome"]
    lots = n["Lotes"]
    out["cpv"] = distinct([e["cpv"] for e in n["CPVs"]] + [e["cpv"] for e in lots])
    out["cpvDescricao"] = distinct(
        [e["descricao"] for e in n["CPVs"]] + [e["cpvDescricao"] for e in lots]
    )
    out["loteDesignacao"] = distinct(
        v for e in lots for v in (e["designacaoLote"], e["descricaoLote"])
    )
    out["loteValorBase"] = distinct(e["valorBaseLote"] for e in lots)
    out["criterioLote"] = distinct(e["criterioLote"] for e in lots)
    out["criterioAquisicaoTipo"] = distinct(e["criterioAquisicaoTipo"] for e in lots)
    out["criterioAquisicaoTipoFator"] = distinct(
        e["criterioAquisicaoTipoFator"] for e in lots
    )
    out["anteriorIDAnuncio"] = distinct(e["IDAnuncio"] for e in n["anuncioAnterior"])
    out["anteriorNAnuncio"] = distinct(e["nAnuncio"] for e in n["anuncioAnterior"])
    return compact(out)


def compact(record: dict[str, Any]) -> dict[str, Any]:
    """Drop absent values (``None`` and empty lists) from a bulk record."""
    return {k: v for k, v in record.items() if v is not None and v != []}


# --------------------------------------------------------------------------
# Derivation of entity records with aggregates
# --------------------------------------------------------------------------


def _id_sort_key(key: str) -> tuple[int, int, str]:
    """Sort digit-only ids numerically, anything else after them as text."""
    return (0, int(key), "") if key.isdigit() else (1, 0, key)


def _ranked(counter: Counter[str]) -> list[str]:
    """Values by descending frequency, then ascending text (deterministic)."""
    return [v for v, _ in sorted(counter.items(), key=lambda kv: (-kv[1], kv[0]))]


def _dec(value: float | None) -> Decimal:
    return Decimal(repr(value)) if value is not None else Decimal(0)


@dataclass
class PartyAgg:
    """Aggregate of one party key (supplier, contracting entity)."""

    names: Counter[str] = field(default_factory=Counter)
    countries: Counter[str] = field(default_factory=Counter)
    nuts_codes: Counter[str] = field(default_factory=Counter)
    nuts_descs: Counter[str] = field(default_factory=Counter)
    types: Counter[str] = field(default_factory=Counter)
    activities: Counter[str] = field(default_factory=Counter)
    sme: bool = False
    contracts: int = 0
    value: Decimal = Decimal(0)
    first_year: int | None = None
    last_year: int | None = None

    def years(self, year: int | None) -> None:
        """Widen the first/last year range with ``year``."""
        if year is None:
            return
        self.first_year = (
            year if self.first_year is None else min(self.first_year, year)
        )
        self.last_year = year if self.last_year is None else max(self.last_year, year)


@dataclass
class CodeAgg:
    """Aggregate of one code (CPV, place, NUTS)."""

    descs: Counter[str] = field(default_factory=Counter)
    contracts: int = 0
    principal: int = 0
    complementary: int = 0
    value: Decimal = Decimal(0)
    entities: set[str] = field(default_factory=set)
    suppliers: set[str] = field(default_factory=set)


class Deriver:
    """Accumulates the derived entities over unique, cleaned contracts.

    Procedures are written to ``BUCKETS`` spill files under ``spill_dir`` and
    aggregated bucket by bucket, so memory does not grow with the 1.9 M
    procedure keys.
    """

    def __init__(self, spill_dir: Path, report: Report) -> None:
        self.report = report
        self.suppliers: dict[str, PartyAgg] = defaultdict(PartyAgg)
        self.entities: dict[str, PartyAgg] = defaultdict(PartyAgg)
        self.invited: dict[str, PartyAgg] = defaultdict(PartyAgg)
        self.cpv: dict[str, CodeAgg] = defaultdict(CodeAgg)
        self.places: dict[str, CodeAgg] = defaultdict(CodeAgg)
        self.nuts: dict[str, CodeAgg] = defaultdict(CodeAgg)
        self.keyless: Counter[str] = Counter()
        self.spill_dir = spill_dir
        spill_dir.mkdir(parents=True, exist_ok=True)
        self.spills = [
            (spill_dir / f"proc-{i:02d}.jsonl").open("w", encoding="utf-8")
            for i in range(BUCKETS)
        ]

    def close(self) -> None:
        """Close the spill files."""
        for fh in self.spills:
            if not fh.closed:
                fh.close()

    def _parties(
        self,
        role: str,
        lst: list[dict[str, Any]],
        aggs: dict[str, PartyAgg],
        c: dict[str, Any],
    ) -> None:
        if not lst:
            return
        price = _dec(c["precoContratual"])
        share = price / len(lst)
        seen: set[str] = set()
        for e in lst:
            nif = e["nif"]
            if nif is None:
                self.keyless[f"{role} references without NIF"] += 1
                continue
            a = aggs[nif]
            if e["nome"]:
                a.names[e["nome"]] += 1
            if e["codPais"]:
                a.countries[e["codPais"]] += 1
            if e["nutsCod"]:
                a.nuts_codes[e["nutsCod"]] += 1
                nuts = self.nuts[e["nutsCod"]]
                (nuts.entities if role == "adjudicante" else nuts.suppliers).add(nif)
                if e["nutsDesc"]:
                    nuts.descs[e["nutsDesc"]] += 1
            if e["nutsDesc"]:
                a.nuts_descs[e["nutsDesc"]] += 1
            if e["tipoEntidade"]:
                a.types[e["tipoEntidade"]] += 1
            if e["atividadeEntidade"]:
                a.activities[e["atividadeEntidade"]] += 1
            if e["pme"] == "Sim":
                a.sme = True
            a.value += share
            if nif not in seen:
                seen.add(nif)
                a.contracts += 1
                a.years(c["Ano"])

    def add(self, c: dict[str, Any]) -> None:
        """Account one unique cleaned contract."""
        self._parties("adjudicante", c["adjudicante"], self.entities, c)
        self._parties("adjudicatarios", c["adjudicatarios"], self.suppliers, c)
        price = _dec(c["precoContratual"])
        seen_inv: set[str] = set()
        for e in c["entidadesConvidadas"]:
            nif = e["nif"]
            if nif is None:
                self.keyless["entidadesConvidadas references without NIF"] += 1
                continue
            a = self.invited[nif]
            if e["nome"]:
                a.names[e["nome"]] += 1
            if e["aliasPais"]:
                a.countries[e["aliasPais"]] += 1
            if e["pais"]:
                a.nuts_descs[e["pais"]] += 1  # country names
            if nif not in seen_inv:
                seen_inv.add(nif)
                a.contracts += 1
                a.years(c["Ano"])
        seen_cpv: set[str] = set()
        for e in c["CPV"]:
            code = e["cpv"]
            if code is None:
                continue
            agg = self.cpv[code]
            if e["descricao"]:
                agg.descs[e["descricao"]] += 1
            agg.value += _dec(e["valor"])
            if code not in seen_cpv:
                seen_cpv.add(code)
                agg.contracts += 1
                objs = {x["objeto"] for x in c["CPV"] if x["cpv"] == code}
                agg.principal += "Principal" in objs
                agg.complementary += "Complementar" in objs
        for place in c["localExecucao"]:
            agg = self.places[place]
            agg.contracts += 1
            agg.value += price
        for code in distinct(e["codigo"] for e in c["NUTs"]):
            agg = self.nuts[code]
            agg.contracts += 1
            agg.value += price
        for e in c["NUTs"]:
            if e["descricao"]:
                self.nuts[e["codigo"]].descs[e["descricao"]] += 1
        pid = c["idprocedimento"]
        if pid is None:
            self.report.note(
                "contratos.idprocedimento: missing (no procedure record)",
                c["idcontrato"],
            )
            return
        bucket = zlib.crc32(pid.encode("utf-8")) % BUCKETS
        frag = [
            pid,
            c["idcontrato"],
            c["tipoprocedimento"],
            c["precoBaseProcedimento"],
            c["precoEstimadoProc"],
            c["objectoContrato"],
            distinct(e["nif"] for e in c["adjudicante"]),
            c["IDAnuncio"],
            c["nAnuncio"],
            c["precoContratual"],
            c["dataPublicacao"],
        ]
        self.spills[bucket].write(json.dumps(frag, ensure_ascii=False) + "\n")

    # ---- finalisation (each yields bulk records sorted by key) ----

    def suppliers_records(
        self, register: dict[str, dict[str, Any]]
    ) -> Iterator[dict[str, Any]]:
        """Yield the ``fornecedores`` records."""
        for nif in sorted(self.suppliers, key=_id_sort_key):
            a = self.suppliers[nif]
            reg = register.get(nif, {})
            yield compact(
                {
                    "nif": nif,
                    "nome": _ranked(a.names),
                    "codPais": _ranked(a.countries),
                    "nutsCod": _ranked(a.nuts_codes),
                    "nutsDesc": _ranked(a.nuts_descs),
                    "pme": "Sim" if a.sme else None,
                    "numContratos": a.contracts,
                    "valorTotal": float(a.value),
                    "primeiroAno": a.first_year,
                    "ultimoAno": a.last_year,
                    "entNumContratos": reg.get("numContratos"),
                    "entTotAdjudicatario": reg.get("totAdjudicatario"),
                    "entTotValorContratIni": reg.get("totValorContratIni"),
                    "descPais": reg.get("descPais"),
                }
            )

    def entities_records(
        self, register: dict[str, dict[str, Any]]
    ) -> Iterator[dict[str, Any]]:
        """Yield the ``adjudicantes`` records."""
        for nif in sorted(self.entities, key=_id_sort_key):
            a = self.entities[nif]
            reg = register.get(nif, {})
            yield compact(
                {
                    "nif": nif,
                    "nome": _ranked(a.names),
                    "codPais": _ranked(a.countries),
                    "nutsCod": _ranked(a.nuts_codes),
                    "nutsDesc": _ranked(a.nuts_descs),
                    "tipoEntidade": _ranked(a.types),
                    "atividadeEntidade": _ranked(a.activities),
                    "numContratos": a.contracts,
                    "valorTotal": float(a.value),
                    "primeiroAno": a.first_year,
                    "ultimoAno": a.last_year,
                    "entNumContratos": reg.get("numContratos"),
                    "entTotAdjudicante": reg.get("totAdjudicante"),
                    "entTotAdjudicanteValorContratIni": reg.get(
                        "totAdjudicanteValorContratIni"
                    ),
                    "descPais": reg.get("descPais"),
                }
            )

    def invited_records(self) -> Iterator[dict[str, Any]]:
        """Yield the ``convidadas`` records."""
        for nif in sorted(self.invited, key=_id_sort_key):
            a = self.invited[nif]
            yield compact(
                {
                    "nif": nif,
                    "nome": _ranked(a.names),
                    "aliasPais": _ranked(a.countries),
                    "pais": _ranked(a.nuts_descs),
                    "numConvites": a.contracts,
                    "primeiroAno": a.first_year,
                    "ultimoAno": a.last_year,
                }
            )

    def cpv_records(self) -> Iterator[dict[str, Any]]:
        """Yield the ``cpv`` records."""
        for code in sorted(self.cpv):
            a = self.cpv[code]
            descs = _ranked(a.descs)
            if len(descs) > 1:
                self.report.note(
                    "cpv: code with several descriptions (most frequent kept)", code
                )
            yield compact(
                {
                    "cpv": code,
                    "descricao": descs[0] if descs else None,
                    "numContratos": a.contracts,
                    "numPrincipal": a.principal,
                    "numComplementar": a.complementary,
                    "valorTotal": float(a.value),
                }
            )

    def places_records(self) -> Iterator[dict[str, Any]]:
        """Yield the ``locais`` records; the place string is split at commas."""
        for place in sorted(self.places):
            a = self.places[place]
            parts = [p.strip() or None for p in place.split(",")]
            if len(parts) > 3:
                self.report.note(
                    "locais: place with more than 3 parts (rest joined to municipality)",
                    place,
                )
                parts = parts[:2] + [", ".join(p for p in parts[2:] if p)]
            parts += [None] * (3 - len(parts))
            yield compact(
                {
                    "local": place,
                    "pais": parts[0],
                    "distrito": parts[1],
                    "concelho": parts[2],
                    "numContratos": a.contracts,
                    "valorTotal": float(a.value),
                }
            )

    def nuts_records(self) -> Iterator[dict[str, Any]]:
        """Yield the ``nuts`` records (codes from ``NUTs[]`` and party ``NUTsCod``)."""
        for code in sorted(self.nuts):
            a = self.nuts[code]
            descs = _ranked(a.descs)
            yield compact(
                {
                    "nuts": code,
                    "descricao": descs[0] if descs else None,
                    "numContratos": a.contracts,
                    "numAdjudicantes": len(a.entities),
                    "numFornecedores": len(a.suppliers),
                    "valorTotal": float(a.value),
                }
            )

    def procedure_records(self) -> Iterator[dict[str, Any]]:
        """Yield the ``procedimentos`` records, bucket by bucket, sorted by key."""
        self.close()
        for i in range(BUCKETS):
            path = self.spill_dir / f"proc-{i:02d}.jsonl"
            groups: dict[str, list[list[Any]]] = defaultdict(list)
            with path.open(encoding="utf-8") as fh:
                for line in fh:
                    frag = json.loads(line)
                    groups[frag[0]].append(frag)
            for pid in sorted(groups, key=_id_sort_key):
                yield self._procedure(pid, groups[pid])
            path.unlink()

    def _procedure(self, pid: str, frags: list[list[Any]]) -> dict[str, Any]:
        frags.sort(key=lambda f: _id_sort_key(f[1]))
        types = Counter(f[2] for f in frags if f[2])
        if len(types) > 1:
            self.report.note(
                "procedimentos: several tipoprocedimento (most frequent kept)", pid
            )
        bases = [f[3] for f in frags if f[3] is not None]
        ests = [f[4] for f in frags if f[4] is not None]
        dates = [f[10] for f in frags if f[10]]
        return compact(
            {
                "idprocedimento": pid,
                "tipoprocedimento": _ranked(types)[0] if types else None,
                "precoBaseProcedimento": max(bases) if bases else None,
                "precoEstimadoProc": max(ests) if ests else None,
                "idcontrato": [f[1] for f in frags],
                "objectoContrato": distinct(f[5] for f in frags),
                "adjudicanteNif": distinct(n for f in frags for n in f[6]),
                "IDAnuncio": distinct(f[7] for f in frags),
                "nAnuncio": distinct(f[8] for f in frags),
                "numContratos": len(frags),
                "valorTotal": float(sum((_dec(f[9]) for f in frags), Decimal(0))),
                "primeiraPublicacao": min(dates) if dates else None,
                "ultimaPublicacao": max(dates) if dates else None,
            }
        )


# --------------------------------------------------------------------------
# Streaming JSON
# --------------------------------------------------------------------------


def open_source(path: Path) -> TextIO:
    """Open a source file as UTF-8 text; a ``.zip`` must hold exactly one member."""
    if path.suffix == ".zip":
        zf = zipfile.ZipFile(path)
        names = zf.namelist()
        if len(names) != 1:
            zf.close()
            raise IngestError(f"{path}: expected one member in the zip, found {names}")
        return io.TextIOWrapper(zf.open(names[0]), encoding="utf-8-sig")
    return path.open(encoding="utf-8-sig")


def iter_json_array(fh: TextIO, name: str) -> Iterator[dict[str, Any]]:
    """Yield the objects of a top-level JSON array without loading it whole.

    Numbers with a fraction or exponent are parsed as ``Decimal``. The
    separators are checked strictly; any syntax error raises ``IngestError``
    naming the file and the element index.
    """
    dec = json.JSONDecoder(parse_float=Decimal)
    buf = fh.read(CHUNK)
    pos = 0
    eof = not buf
    index = 0

    def fill() -> None:
        nonlocal buf, pos, eof
        more = fh.read(CHUNK)
        eof = not more
        buf, pos = buf[pos:] + more, 0

    def skip_ws() -> None:
        nonlocal pos
        while True:
            while pos < len(buf) and buf[pos] in " \t\r\n":
                pos += 1
            if pos < len(buf) or eof:
                return
            fill()

    skip_ws()
    if pos >= len(buf) or buf[pos] != "[":
        raise IngestError(f"{name}: not a JSON array")
    pos += 1
    skip_ws()
    if pos < len(buf) and buf[pos] == "]":
        return
    while True:
        skip_ws()
        while True:
            try:
                obj, end = dec.raw_decode(buf, pos)
                break
            except json.JSONDecodeError as exc:
                if eof:
                    raise IngestError(f"{name}: element {index}: {exc}") from exc
                fill()
        if not isinstance(obj, dict):
            raise IngestError(f"{name}: element {index} is not an object")
        yield obj
        index += 1
        pos = end
        skip_ws()
        if pos >= len(buf):
            raise IngestError(f"{name}: unterminated array after element {index}")
        if buf[pos] == "]":
            pos += 1
            skip_ws()
            if pos < len(buf):
                raise IngestError(f"{name}: data after the closing bracket")
            return
        if buf[pos] != ",":
            raise IngestError(
                f"{name}: expected ',' or ']' after element {index}, got {buf[pos]!r}"
            )
        pos += 1
        if pos > CHUNK:
            buf, pos = buf[pos:], 0


def read_records(path: Path, limit: int | None) -> Iterator[dict[str, Any]]:
    """Yield at most ``limit`` records of one source file."""
    with open_source(path) as fh:
        for i, rec in enumerate(iter_json_array(fh, path.name)):
            if limit is not None and i >= limit:
                return
            yield rec


# --------------------------------------------------------------------------
# Stage 1: fetch
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Resource:
    """One downloadable file from the dados.gov.pt API."""

    dataset: str
    year: int | None
    name: str
    url: str
    size: int | None
    sha1: str | None
    last_modified: str | None = None


def _http_get(
    url: str, headers: dict[str, str] | None = None
) -> urllib.request.Request:
    h = {"User-Agent": USER_AGENT}
    h.update(headers or {})
    return urllib.request.Request(url, headers=h)


def _retry(what: str, action: Callable[[], Any]) -> Any:
    for attempt in range(1, RETRIES + 1):
        try:
            return action()
        except (
            urllib.error.URLError,
            TimeoutError,
            ConnectionError,
            IngestError,
        ) as exc:
            if attempt == RETRIES:
                raise IngestError(f"giving up on {what}: {exc}") from exc
            wait = 10 * attempt
            print(
                f"  attempt {attempt} failed for {what}: {exc}; retrying in {wait} s",
                file=sys.stderr,
            )
            time.sleep(wait)
    raise AssertionError("unreachable")


def resolve_resources(dataset: str) -> list[Resource]:
    """Return the current JSON/zip resources of ``dataset`` from the API."""
    url = API.format(slug=SLUGS[dataset])

    def load() -> Any:
        with urllib.request.urlopen(_http_get(url), timeout=60) as resp:
            return json.load(resp)

    meta = _retry(url, load)
    pattern = PATTERNS[dataset]
    found: dict[str, Resource] = {}
    for res in meta.get("resources", []):
        title = str(res.get("title") or "")
        res_url = str(res.get("url") or "")
        name = res_url.rsplit("/", 1)[-1]
        m = pattern.fullmatch(name) or pattern.fullmatch(title)
        if not m or not res_url.startswith("https://"):
            continue
        checksum = res.get("checksum") or {}
        sha1 = checksum.get("value") if checksum.get("type") == "sha1" else None
        year = int(m.group(1)) if m.groups() else None
        resource = Resource(
            dataset,
            year,
            name,
            res_url,
            res.get("filesize"),
            sha1,
            res.get("last_modified"),
        )
        if name in found:
            raise IngestError(f"{dataset}: resource {name} listed twice by the API")
        found[name] = resource
    return sorted(found.values(), key=lambda r: (r.year or 0, r.name))


def sha1_of(path: Path) -> str:
    """Return the hex SHA-1 of a file."""
    h = hashlib.sha1()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def verify(path: Path, res: Resource) -> str | None:
    """Return ``None`` when ``path`` matches the resource size and SHA-1, else why not."""
    size = path.stat().st_size
    if res.size is not None and size != res.size:
        return f"size {size} != {res.size}"
    if res.sha1 is not None and sha1_of(path) != res.sha1.lower():
        return "sha1 mismatch"
    return None


def download(res: Resource, dest: Path, force: bool) -> str:
    """Download one resource to ``dest`` (resumable); return what was done."""
    if dest.exists() and not force:
        if verify(dest, res) is None:
            return "kept"
        dest.unlink()
    part = dest.with_name(dest.name + ".part")
    if force and part.exists():
        part.unlink()

    def attempt() -> None:
        have = part.stat().st_size if part.exists() else 0
        headers = {"Range": f"bytes={have}-"} if have else {}
        try:
            resp = urllib.request.urlopen(_http_get(res.url, headers), timeout=120)
        except urllib.error.HTTPError as exc:
            if exc.code == 416 and have:  # range past the end: restart
                part.unlink()
            raise
        with resp:
            mode = "ab" if have and resp.status == 206 else "wb"
            with part.open(mode) as out:
                shutil.copyfileobj(resp, out, 1 << 20)
        problem = verify(part, res)
        if problem is not None:
            part.unlink()
            raise IngestError(f"{res.name}: {problem}")
        part.replace(dest)

    _retry(res.url, attempt)
    return "downloaded"


MANIFEST = "manifest.json"


def read_manifest(raw_dir: Path) -> dict[str, dict[str, Any]]:
    """Return ``<raw_dir>/manifest.json`` (file name -> metadata), ``{}`` if absent."""
    path = raw_dir / MANIFEST
    if not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise IngestError(f"{path}: not a JSON object")
    return data


def write_manifest(raw_dir: Path, manifest: dict[str, dict[str, Any]]) -> None:
    """Write the manifest atomically."""
    tmp = raw_dir / (MANIFEST + ".tmp")
    tmp.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    tmp.replace(raw_dir / MANIFEST)


def fetch(
    datasets: Iterable[str], raw_dir: Path, force: bool, allow_missing_years: bool
) -> dict[str, list[Path]]:
    """Download the current files of each dataset into ``raw_dir``.

    The resource list, sizes, SHA-1 checksums and modification dates come
    from the dados.gov.pt API. A cached file is kept when its size matches and
    its SHA-1 equals the API value (the hash is not recomputed when the
    manifest already records that same SHA-1 for a file of that size); a
    missing, partial or stale file is (re)downloaded. ``force`` downloads every
    file again. The manifest records what is cached and is updated after each
    file, so an interrupted run keeps what it finished.
    """
    raw_dir.mkdir(parents=True, exist_ok=True)
    manifest = read_manifest(raw_dir)
    out: dict[str, list[Path]] = {}
    for ds in sorted(datasets):
        resources = resolve_resources(ds)
        check_years(ds, [r.year for r in resources], allow_missing_years)
        paths = []
        for res in resources:
            dest = raw_dir / res.name
            known = manifest.get(res.name, {})
            if (
                not force
                and dest.exists()
                and res.sha1 is not None
                and known.get("sha1") == res.sha1
                and dest.stat().st_size == known.get("size") == res.size
            ):
                status = "up to date"
            else:
                status = download(res, dest, force)
                manifest[res.name] = {
                    "dataset": ds,
                    "year": res.year,
                    "url": res.url,
                    "size": res.size if res.size is not None else dest.stat().st_size,
                    "sha1": res.sha1,
                    "last_modified": res.last_modified,
                    "fetched_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                }
                write_manifest(raw_dir, manifest)
            print(f"  {res.name}: {status} ({dest.stat().st_size:,} bytes)")
            paths.append(dest)
        out[ds] = paths
    return out


def cached_files(
    datasets: Iterable[str], raw_dir: Path
) -> dict[str, list[Path]] | None:
    """Return the cached files of ``datasets`` without network access.

    Returns ``None`` when any dataset is incomplete: a year of 2012-2026 (or
    ``entidades.json``) absent from the manifest, or a listed file missing or
    of another size than recorded.
    """
    manifest = read_manifest(raw_dir)
    out: dict[str, list[Path]] = {}
    for ds in sorted(datasets):
        entries = sorted(
            (name, meta) for name, meta in manifest.items() if meta.get("dataset") == ds
        )
        try:
            check_years(ds, [meta.get("year") for _, meta in entries], False)
        except IngestError:
            return None
        paths = []
        for name, meta in entries:
            path = raw_dir / name
            if not path.is_file() or path.stat().st_size != meta.get("size"):
                return None
            paths.append(path)
        out[ds] = paths
    return out


def status(raw_dir: Path, verify_hashes: bool) -> int:
    """Print what is cached per dataset; return 1 when anything is missing or bad."""
    manifest = read_manifest(raw_dir)
    print(f"cache: {raw_dir}")
    problems = 0
    total = 0
    for ds in sorted(SLUGS):
        entries = sorted(
            (name, meta) for name, meta in manifest.items() if meta.get("dataset") == ds
        )
        print(f"{ds} ({SLUGS[ds]}):")
        for name, meta in entries:
            path = raw_dir / name
            if not path.is_file():
                state = "MISSING"
            elif path.stat().st_size != meta.get("size"):
                state = f"BAD SIZE {path.stat().st_size:,}"
            elif verify_hashes and meta.get("sha1") and sha1_of(path) != meta["sha1"]:
                state = "BAD SHA-1"
            else:
                state = "ok (sha1 verified)" if verify_hashes else "ok"
                total += path.stat().st_size
            problems += not state.startswith("ok")
            print(
                f"  {name:24} {meta.get('size', 0):>13,} B  published "
                f"{meta.get('last_modified') or '?':32}  fetched "
                f"{meta.get('fetched_at') or '?'}  {state}"
            )
        try:
            check_years(ds, [meta.get("year") for _, meta in entries], False)
        except IngestError as exc:
            problems += 1
            print(f"  INCOMPLETE: {exc}")
    print(f"total cached: {total:,} bytes in {len(manifest)} file(s)")
    if problems:
        print(f"{problems} problem(s); run: ingest_base.py fetch")
    return 1 if problems else 0


def check_years(dataset: str, years: list[int | None], allow_missing: bool) -> None:
    """Fail unless a yearly dataset covers 2012-2026 (and has no duplicate year)."""
    if dataset == "entities":
        if len(years) != 1:
            raise IngestError(
                f"entities: expected one entidades.json, found {len(years)}"
            )
        return
    dup = [y for y, n in Counter(years).items() if n > 1]
    if dup:
        raise IngestError(f"{dataset}: several files for year(s) {dup}")
    missing = sorted(set(YEARS) - {y for y in years if y is not None})
    if missing and not allow_missing:
        raise IngestError(f"{dataset}: no file for year(s) {missing}")
    if not years:
        raise IngestError(f"{dataset}: no file found")


def offline_files(
    datasets: Iterable[str], src: Path, allow_missing_years: bool
) -> dict[str, list[Path]]:
    """Stage 1 (offline): find the dataset files already present in ``src``."""
    out: dict[str, list[Path]] = {}
    for ds in sorted(datasets):
        pattern = PATTERNS[ds]
        found = sorted(
            (p for p in src.iterdir() if p.is_file() and pattern.fullmatch(p.name)),
            key=lambda p: p.name,
        )
        years = [
            int(m.group(1)) if (m := pattern.fullmatch(p.name)) and m.groups() else None
            for p in found
        ]
        check_years(ds, years, allow_missing_years)
        out[ds] = found
        for p in found:
            print(f"  {p.name}: offline ({p.stat().st_size:,} bytes)")
    return out


def file_year(path: Path) -> int:
    """Return the year in a yearly file name (0 when none)."""
    m = re.search(r"(\d{4})", path.name)
    return int(m.group(1)) if m else 0


# --------------------------------------------------------------------------
# Writers
# --------------------------------------------------------------------------


class JsonlWriter:
    """Write records as UTF-8 JSON Lines, atomically (``.tmp`` then rename)."""

    def __init__(self, path: Path) -> None:
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self.tmp = path.with_name(path.name + ".tmp")
        self.fh = self.tmp.open("w", encoding="utf-8", newline="\n")
        self.count = 0

    def write(self, record: dict[str, Any]) -> None:
        """Append one record."""
        self.fh.write(
            json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n"
        )
        self.count += 1

    def close(self) -> None:
        """Close and publish the file."""
        self.fh.close()
        self.tmp.replace(self.path)


class PartWriter:
    """Write bulk records into ``part-NNNN.jsonl`` files of at most ``batch`` lines."""

    def __init__(self, directory: Path, batch: int = BATCH) -> None:
        if directory.exists():
            shutil.rmtree(directory)
        directory.mkdir(parents=True)
        self.directory = directory
        self.batch = batch
        self.parts: list[Path] = []
        self.count = 0
        self._writer: JsonlWriter | None = None

    def write(self, record: dict[str, Any]) -> None:
        """Append one record, opening a new part when the current one is full."""
        if self._writer is None or self._writer.count >= self.batch:
            self._close_part()
            path = self.directory / f"part-{len(self.parts) + 1:04d}.jsonl"
            self._writer = JsonlWriter(path)
            self.parts.append(path)
        self._writer.write(record)
        self.count += 1

    def _close_part(self) -> None:
        if self._writer is not None:
            self._writer.close()
            self._writer = None

    def close(self) -> None:
        """Close the last part."""
        self._close_part()


# --------------------------------------------------------------------------
# Pipeline (stages 2-4)
# --------------------------------------------------------------------------


@dataclass
class Outputs:
    """Bulk parts and counts per index produced by the pipeline."""

    parts: dict[str, list[Path]] = field(default_factory=dict)
    counts: dict[str, int] = field(default_factory=dict)


def build(
    files: dict[str, list[Path]],
    wanted: tuple[str, ...],
    cache: Path,
    limit: int | None,
    report: Report,
) -> Outputs:
    """Stages 2-4 (without loading): clean, derive and write the bulk files."""
    clean_dir = cache / "clean"
    cli_dir = cache / "cli"
    clean_dir.mkdir(parents=True, exist_ok=True)
    outs = Outputs()

    def emit(
        entity: str,
        clean_records: Iterable[dict[str, Any]],
        to_cli: Callable[[dict[str, Any]], dict[str, Any]],
    ) -> None:
        cw = JsonlWriter(clean_dir / f"{entity}.jsonl")
        pw = PartWriter(cli_dir / entity)
        for rec in clean_records:
            cw.write(rec)
            pw.write(to_cli(rec))
        cw.close()
        pw.close()
        outs.parts[entity] = pw.parts
        outs.counts[entity] = pw.count
        print(f"  {entity}: {pw.count:,} records in {len(pw.parts)} bulk file(s)")

    # Entity register (only for fornecedores/adjudicantes).
    register: dict[str, dict[str, Any]] = {}
    if "entities" in files:
        rows = keyless = 0
        for path in files["entities"]:
            for raw in read_records(path, limit):
                rows += 1
                row = clean_entity_row(raw, report)
                if row["nif"] is None:
                    keyless += 1
                    continue
                if row["nif"] in register:
                    report.note("entidades: duplicate NIF (first row kept)", row["nif"])
                    continue
                register[row["nif"]] = row
        print(
            f"  entidades: {rows:,} rows, {len(register):,} NIFs, {keyless:,} rows without NIF"
        )

    # Modifications (small: held in memory to assign sequences).
    mods_per_contract: Counter[str] = Counter()
    if "mods" in files:
        mods = []
        for path in sorted(files["mods"], key=file_year):
            for raw in read_records(path, limit):
                mods.append(clean_modification(raw, report))
        sequenced = sequence_modifications(mods)
        mods_per_contract = Counter(m["idcontrato"] for m in sequenced)
        if "modificacoes" in wanted:
            emit("modificacoes", sequenced, modification_to_cli)

    # Announcements (deduplicated by IDAnuncio, newest year first).
    if "notices" in files and "anuncios" in wanted:

        def notices() -> Iterator[dict[str, Any]]:
            seen: set[str] = set()
            for path in sorted(files["notices"], key=file_year, reverse=True):
                merged = merge_notices(
                    (clean_notice(raw, report) for raw in read_records(path, limit)),
                    report,
                )
                for n in merged:
                    if n["IDAnuncio"] in seen:
                        report.note(
                            "anuncios: IDAnuncio in an older year file (newest kept)",
                            n["IDAnuncio"],
                        )
                        continue
                    seen.add(n["IDAnuncio"])
                    yield n

        emit("anuncios", notices(), notice_to_cli)

    # Contracts: clean, dedupe, write, derive in one pass.
    if "contracts" in files:
        derive = any(
            e in wanted
            for e in ENTITIES
            if e not in ("contratos", "modificacoes", "anuncios")
        )
        deriver = Deriver(cache / "spill", report) if derive else None
        try:

            def contracts() -> Iterator[dict[str, Any]]:
                seen: dict[str, bytes] = {}
                for path in sorted(files["contracts"], key=file_year, reverse=True):
                    n_file = 0
                    for raw in read_records(path, limit):
                        n_file += 1
                        c = clean_contract(raw, report)
                        digest = _digest(c)
                        old = seen.get(c["idcontrato"])
                        if old is not None:
                            kind = "identical" if old == digest else "non-identical"
                            report.note(
                                f"contratos: {kind} duplicate idcontrato (newest kept)",
                                c["idcontrato"],
                            )
                            continue
                        seen[c["idcontrato"]] = digest
                        if deriver is not None:
                            deriver.add(c)
                        yield c
                    print(f"    {path.name}: {n_file:,} records read")
                unresolved = set(mods_per_contract) - seen.keys()
                for cid in sorted(unresolved, key=_id_sort_key):
                    report.note("modificacoes: idcontrato not found in contracts", cid)

            if "contratos" in wanted:
                emit(
                    "contratos",
                    contracts(),
                    lambda c: contract_to_cli(
                        c, mods_per_contract.get(c["idcontrato"], 0)
                    ),
                )
            else:
                for _ in contracts():
                    pass
            if deriver is not None:
                for kind, total in sorted(deriver.keyless.items()):
                    report.events[f"derive: {kind}"] = total
                derived: list[tuple[str, Callable[[], Iterator[dict[str, Any]]]]] = [
                    ("fornecedores", lambda: deriver.suppliers_records(register)),
                    ("adjudicantes", lambda: deriver.entities_records(register)),
                    ("procedimentos", deriver.procedure_records),
                    ("cpv", deriver.cpv_records),
                    ("locais", deriver.places_records),
                    ("nuts", deriver.nuts_records),
                    ("convidadas", deriver.invited_records),
                ]
                for entity, make in derived:
                    if entity in wanted:
                        emit(entity, make(), lambda r: r)
                if register:
                    unmatched = sum(
                        1
                        for nif in register
                        if nif not in deriver.suppliers and nif not in deriver.entities
                    )
                    report.events[
                        "entidades: NIFs not seen as supplier or contracting entity"
                    ] = unmatched
        finally:
            if deriver is not None:
                deriver.close()
                shutil.rmtree(deriver.spill_dir, ignore_errors=True)
    return outs


def _digest(record: dict[str, Any]) -> bytes:
    data = json.dumps(
        record, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.blake2b(data, digest_size=12).digest()


# --------------------------------------------------------------------------
# Stage 4: load through the CLI
# --------------------------------------------------------------------------


def run_cli(binary: Path, args: list[str]) -> str:
    """Run ``rucene-base`` with ``args``; return stdout or raise with stderr."""
    proc = subprocess.run(
        [str(binary), *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    if proc.returncode != 0:
        raise IngestError(
            f"rucene-base {' '.join(args)} exited {proc.returncode}\n--- stderr ---\n{proc.stderr.strip()}"
        )
    return proc.stdout


def load(
    binary: Path, root: Path, outs: Outputs, wanted: tuple[str, ...]
) -> dict[str, str]:
    """Create every wanted index under ``root`` and add its bulk files; return ``count`` output."""
    if not binary.is_file():
        raise IngestError(
            f"binary not found: {binary} (build it with: cargo build --release -p rucene-cli-base)"
        )
    root.mkdir(parents=True, exist_ok=True)
    counted: dict[str, str] = {}
    for entity in wanted:
        if entity not in outs.parts:
            raise IngestError(f"{entity}: no bulk files were produced")
        started = time.monotonic()
        run_cli(binary, [entity, "create", str(root)])
        for part in outs.parts[entity]:
            run_cli(binary, [entity, "add", str(root), "--file", str(part)])
        counted[entity] = run_cli(binary, [entity, "count", str(root)]).strip()
        print(
            f"  {entity}: loaded {outs.counts[entity]:,} documents in {time.monotonic() - started:.1f} s; count: {counted[entity]}"
        )
        number = re.search(r"\d+", counted[entity].replace(",", ""))
        if number and int(number.group()) != outs.counts[entity]:
            raise IngestError(
                f"{entity}: index count {number.group()} != {outs.counts[entity]} documents written"
            )
    return counted


# --------------------------------------------------------------------------
# Command line
# --------------------------------------------------------------------------


def _only(ap: argparse.ArgumentParser, value: str) -> tuple[str, ...]:
    only = tuple(e.strip() for e in value.split(",") if e.strip())
    unknown = [e for e in only if e not in ENTITIES]
    if unknown or not only:
        ap.error(
            f"--only: unknown or empty index list {unknown}; "
            f"choose from {', '.join(ENTITIES)}"
        )
    return tuple(e for e in ENTITIES if e in only)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    """Parse the command line (subcommands ``fetch``, ``ingest``, ``status``)."""
    crate = Path(__file__).resolve().parent.parent
    repo = crate.parent.parent
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    sub = ap.add_subparsers(dest="command", required=True, metavar="COMMAND")
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--cache",
        type=Path,
        default=crate / "data" / "base",
        help="cache directory: raw/ downloads, clean/ and cli/ JSON Lines "
        "(default: %(default)s, git-ignored)",
    )
    only = argparse.ArgumentParser(add_help=False)
    only.add_argument(
        "--only",
        default=",".join(ENTITIES),
        help="comma-separated indices (default: all ten); fetch takes only the "
        "datasets they need",
    )
    f = sub.add_parser(
        "fetch",
        parents=[common, only],
        help="download what is missing or stale (dados.gov.pt API size/SHA-1)",
    )
    f.add_argument("--force", action="store_true", help="download every file again")
    i = sub.add_parser(
        "ingest",
        parents=[common, only],
        help="clean, derive and load every index; fetches first if the cache "
        "lacks a needed file",
    )
    i.add_argument(
        "--root",
        type=Path,
        default=crate / "data" / "base" / "index",
        help="index root directory, one subdirectory per index (default: %(default)s)",
    )
    i.add_argument(
        "--binary",
        type=Path,
        default=repo / "target" / "release" / "rucene-base",
        help="rucene-base executable (default: %(default)s)",
    )
    i.add_argument(
        "--offline",
        type=Path,
        help="read the dataset files from this directory instead of the cache; "
        "no network",
    )
    i.add_argument(
        "--limit",
        type=int,
        help="read at most N records per source file (sample run: no minimum "
        "count, yearly files may be missing with --offline)",
    )
    s_ = sub.add_parser(
        "status", parents=[common], help="show the cached files (no network)"
    )
    s_.add_argument("--verify", action="store_true", help="also recompute every SHA-1")
    args = ap.parse_args(argv)
    if hasattr(args, "only"):
        args.only = _only(ap, args.only)
    if getattr(args, "limit", None) is not None and args.limit < 1:
        ap.error("--limit must be >= 1")
    if getattr(args, "offline", None) is not None and not args.offline.is_dir():
        ap.error(f"--offline: {args.offline} is not a directory")
    return args


def datasets_for(entities: Iterable[str]) -> set[str]:
    """Return the source datasets the given indices need."""
    return set().union(*(NEEDS[e] for e in entities))


def cmd_fetch(args: argparse.Namespace) -> int:
    """``fetch``: download what is missing or stale."""
    fetch(datasets_for(args.only), args.cache / "raw", args.force, False)
    return 0


def cmd_ingest(args: argparse.Namespace) -> int:
    """``ingest``: obtain the files, then run stages 2-4."""
    report = Report()
    datasets = datasets_for(args.only)
    started = time.monotonic()
    raw_dir = args.cache / "raw"
    print("stage 1: source files")
    if args.offline is not None:
        files = offline_files(datasets, args.offline, args.limit is not None)
    else:
        cached = cached_files(datasets, raw_dir)
        if cached is None:
            print("  cache incomplete: fetching")
            files = fetch(datasets, raw_dir, False, False)
        else:
            files = cached
            n = sum(len(v) for v in files.values())
            print(f"  using {n} cached file(s) in {raw_dir} (run 'fetch' to refresh)")
    try:
        print("stages 2-4: clean, derive, write bulk files")
        outs = build(files, args.only, args.cache, args.limit, report)
        for entity in args.only:
            if outs.counts.get(entity, 0) == 0:
                raise IngestError(f"{entity}: no documents produced")
        if (
            "contratos" in args.only
            and args.limit is None
            and outs.counts["contratos"] < MIN_CONTRACTS
        ):
            raise IngestError(
                f"contratos: {outs.counts['contratos']:,} unique contracts "
                f"< {MIN_CONTRACTS:,}"
            )
        print("stage 4: load")
        load(args.binary, args.root, outs, args.only)
    finally:
        _print_report(report)
    print("documents per index:")
    for entity in args.only:
        print(f"  {entity}: {outs.counts[entity]:,}")
    print(f"done in {time.monotonic() - started:.0f} s; indices under {args.root}")
    return 0


def main(argv: list[str] | None = None) -> int:
    """Run the selected subcommand; return the exit status."""
    args = parse_args(argv)
    try:
        if args.command == "fetch":
            return cmd_fetch(args)
        if args.command == "status":
            return status(args.cache / "raw", args.verify)
        return cmd_ingest(args)
    except IngestError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


def _print_report(report: Report) -> None:
    if report.events:
        print("normalisation report:")
        for line in report.lines():
            print(line)


if __name__ == "__main__":
    sys.exit(main())
