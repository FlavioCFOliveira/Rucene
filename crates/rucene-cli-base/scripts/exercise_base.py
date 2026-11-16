#!/usr/bin/env python3
"""Scripted exercise of the ``rucene-base`` CLI over the Portal BASE indices.

Usage::

    python3 -I exercise_base.py --binary target/release/rucene-base \\
        --root /path/to/full/root --scratch /path/to/scratch/dir \\
        [--only stats,accent,...]

Two groups of scenarios:

* **Read-only scenarios** run ``stats``, ``count``, ``search`` and ``get``
  against the full indices under ``--root`` and never write to them:
  ``stats``, ``accent``, ``phrase``, ``cpv``, ``place``, ``ranges``,
  ``sort``, ``supplier`` and ``crossref``.
* **Write scenarios** (``write``) build a separate small root under
  ``--scratch`` from the crate's test fixtures and run ``create``, ``add``
  (file and ``--set``), ``update`` (file and ``--set``), ``delete`` and
  ``get`` on ``contratos``, ``fornecedores`` and ``modificacoes``,
  re-searching after every change. The scratch root may not be, contain or
  lie inside ``--root``; when ``--root`` is given, the ``stats`` of the three
  full indices are compared before and after the writes.

The central check is the analysis contract: accented, unaccented and
upper-case spellings of the same words (``ação``/``acao``/``AÇÃO``/
``ações``/...) must return the **same** total and the **same** hit set.
On the scratch root, the hit set is also compared with the documents whose
text holds one of the spellings, computed in Python from the fixture records
(Unicode folding and lower-casing, never the CLI's analyzer).

Each scenario stops at its first failed check; the others still run. The
report lists every scenario as PASS or FAIL with its checks, and the exit
status is 1 when any scenario failed (2 on a usage error).

Only the Python standard library is used. The report prints ids, NIFs of
entities, counts and query strings; it never prints the names of suppliers or
contracting entities.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unicodedata
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass, field
from pathlib import Path

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
READ_SCENARIOS = (
    "stats",
    "accent",
    "phrase",
    "cpv",
    "place",
    "ranges",
    "sort",
    "supplier",
    "crossref",
)
WRITE_SCENARIOS = ("write",)
ALL_SCENARIOS = READ_SCENARIOS + WRITE_SCENARIOS
MARKER = ".rucene-base-exercise"
FIXTURES = Path(__file__).resolve().parent.parent / "tests" / "fixtures"
HIT_SET_LIMIT = 1000  # hit sets are compared on the first N hits by _doc
CLI_TIMEOUT_S = 1800
VALUE_SEPARATOR = " | "
TOTAL_LINE = re.compile(r"# total hits: (\d+)( \(lower bound\))?")
WORD = re.compile(r"[a-z0-9]+")

# Default text fields per index (``rucene-base <entity> help``), used by the
# Python model of the scratch root.
CONTRATOS_TEXT = (
    "objectoContrato",
    "descContrato",
    "adjudicanteNome",
    "adjudicatarioNome",
    "cpvDescricao",
)

# Variant groups of DECISION 3249 (#532) and of the README: every member of a
# group must return the same documents. ``(entity, flag, field or None,
# variants)``; ``flag`` is ``--text`` or ``--phrase`` (``-in`` when a field
# is given).
ACCENT_GROUPS: tuple[tuple[str, str, str | None, tuple[str, ...]], ...] = (
    ("contratos", "--text", None, ("ação", "acao", "AÇÃO", "ações", "acoes", "AÇÕES")),
    ("contratos", "--text", None, ("acção", "acções", "accao", "accoes", "ACÇÃO")),
    ("contratos", "--text", None, ("pão", "pães", "paes", "PÃO")),
    ("contratos", "--text", None, ("limão", "limões", "limoes", "LIMÃO")),
    ("contratos", "--text", None, ("papéis", "papeis", "papel", "PAPÉIS")),
    ("contratos", "--text", None, ("preço", "preco", "PREÇO")),
    ("contratos", "--text", None, ("português", "portugues", "PORTUGUÊS")),
    ("contratos", "--text", None, ("música", "musica", "MÚSICA")),
    (
        "contratos",
        "--text",
        None,
        ("manutenção", "manutencao", "MANUTENÇÃO", "manutenções", "manutencoes"),
    ),
    (
        "contratos",
        "--text",
        "objectoContrato",
        ("construção", "construcao", "CONSTRUÇÃO", "construções", "construcoes"),
    ),
    (
        "contratos",
        "--text",
        None,
        ("aquisição serviços", "aquisicao servicos", "AQUISIÇÃO SERVIÇOS"),
    ),
    (
        "fornecedores",
        "--text",
        "nome",
        ("construções", "construcoes", "CONSTRUÇÕES", "construção", "construcao"),
    ),
    ("fornecedores", "--text", "nome", ("comércio", "comercio", "COMÉRCIO")),
    (
        "adjudicantes",
        "--text",
        "nome",
        ("câmara municipal", "camara municipal", "CÂMARA MUNICIPAL"),
    ),
    ("locais", "--text", "distrito", ("Évora", "evora", "ÉVORA")),
    ("locais", "--text", "distrito", ("Santarém", "santarem", "SANTARÉM")),
    ("cpv", "--text", "descricao", ("reparação", "reparacao", "REPARAÇÃO")),
    (
        "modificacoes",
        "--text",
        "modifContratoFundamento",
        (
            "serviços complementares",
            "servicos complementares",
            "SERVIÇOS COMPLEMENTARES",
        ),
    ),
    ("anuncios", "--text", None, ("aquisição", "aquisicao", "AQUISIÇÃO")),
)

PHRASE_GROUPS: tuple[tuple[str, str, str | None, tuple[str, ...]], ...] = (
    (
        "contratos",
        "--phrase",
        None,
        ("prestação de serviços", "prestacao de servicos", "PRESTAÇÃO DE SERVIÇOS"),
    ),
    (
        "contratos",
        "--phrase",
        "objectoContrato",
        ("refeições escolares", "refeicoes escolares", "REFEIÇÕES ESCOLARES"),
    ),
    (
        "contratos",
        "--phrase",
        None,
        ("aquisição de papel", "aquisicao de papeis", "AQUISIÇÃO DE PAPÉIS"),
    ),
)

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


class CheckFailed(Exception):
    """A check did not hold; the message says what and where."""


class UsageError(Exception):
    """The command line of this script is not usable."""


# --------------------------------------------------------------- pure helpers


@dataclass(frozen=True)
class CliResult:
    """The outcome of one CLI process."""

    cmd: tuple[str, ...]
    code: int
    out: str
    err: str
    seconds: float


@dataclass(frozen=True)
class SearchOutput:
    """The parsed TSV of ``search``."""

    total: int
    exact: bool
    header: tuple[str, ...]
    rows: tuple[dict[str, str], ...]


def shown(cmd: Sequence[str]) -> str:
    """A shell-like rendering of a command, quoting arguments with spaces."""
    out = []
    for arg in cmd:
        if arg == "" or re.search(r"[\s\"'$|&;<>()*?]", arg):
            out.append("'" + arg.replace("'", "'\\''") + "'")
        else:
            out.append(arg)
    return " ".join(out)


def parse_tsv(text: str) -> SearchOutput:
    """Parses the output of ``search``: a total line, a header, the rows.

    Raises ValueError when the shape is not the documented one.
    """
    lines = text.rstrip("\n").split("\n")
    if len(lines) < 2:
        raise ValueError(f"search output has {len(lines)} line(s), expected at least 2")
    m = TOTAL_LINE.fullmatch(lines[0])
    if m is None:
        raise ValueError(f"unexpected first line {lines[0]!r}")
    header = tuple(lines[1].split("\t"))
    if header[-1] != "score":
        raise ValueError(f"header does not end with score: {lines[1]!r}")
    rows = []
    for number, line in enumerate(lines[2:], start=3):
        cells = line.split("\t")
        if len(cells) != len(header):
            raise ValueError(
                f"line {number} has {len(cells)} cells, header has {len(header)}"
            )
        rows.append(dict(zip(header, cells, strict=True)))
    return SearchOutput(int(m.group(1)), m.group(2) is None, header, tuple(rows))


def parse_stats(text: str) -> dict[str, str]:
    """Parses ``stats``: ``name: value`` lines."""
    out: dict[str, str] = {}
    for line in text.splitlines():
        name, sep, value = line.partition(": ")
        if not sep:
            raise ValueError(f"unexpected stats line {line!r}")
        out[name] = value
    for required in ("index", "documents", "deleted", "max_doc", "segments"):
        if required not in out:
            raise ValueError(f"stats output lacks {required!r}")
    return out


def parse_get(text: str) -> list[dict[str, object]]:
    """Parses ``get``: one JSON object per line (TypeError for another JSON value)."""
    docs = []
    for line in text.splitlines():
        if not line.strip():
            continue
        value = json.loads(line)
        if not isinstance(value, dict):
            raise TypeError(f"get line is not a JSON object: {line!r}")
        docs.append(value)
    return docs


def parse_count(text: str) -> int:
    """Parses ``count``: one integer."""
    value = text.strip()
    if not value.isdigit():
        raise ValueError(f"count output is not an integer: {text!r}")
    return int(value)


def cell_values(cell: str) -> list[str]:
    """The values of a TSV cell (multi-valued fields are joined by `` | ``)."""
    return [] if cell == "" else cell.split(VALUE_SEPARATOR)


def as_list(value: object) -> list[str]:
    """A stored JSON value (scalar, list or absent) as a list of strings."""
    if value is None:
        return []
    if isinstance(value, list):
        return [str(v) for v in value]
    return [str(value)]


def fold(text: str) -> str:
    """Lower-cases and strips combining marks (NFKD), the Python model of
    ``LowerCaseFilter`` + ``ASCIIFoldingFilter`` for Portuguese letters."""
    decomposed = unicodedata.normalize("NFKD", text.lower())
    return "".join(ch for ch in decomposed if not unicodedata.combining(ch))


def folded_words(text: str) -> set[str]:
    """The folded alphanumeric words of ``text``."""
    return set(WORD.findall(fold(text)))


def model_matches(
    records: Iterable[dict[str, object]],
    key: str,
    fields: Sequence[str],
    forms: set[str],
) -> set[str]:
    """Keys of the records whose ``fields`` hold a word in ``forms``
    (compared folded); the expectation for a one-word ``--text`` query."""
    folded_forms = {fold(f) for f in forms}
    out = set()
    for record in records:
        words: set[str] = set()
        for name in fields:
            for value in as_list(record.get(name)):
                words |= folded_words(value)
        if words & folded_forms:
            out.add(str(record[key]))
    return out


def sort_cell_key(cell: str, numeric: bool) -> float | str | None:
    """The comparable value of a sort cell; None when the value is missing."""
    if cell == "":
        return None
    return float(cell) if numeric else cell


def sort_violation(
    rows: Sequence[dict[str, str]],
    specs: Sequence[tuple[str, bool, bool]],
) -> int | None:
    """Checks the order of ``rows`` under ``specs`` = ``(column, descending,
    numeric)``: lexicographic, missing values after present ones in both
    directions. Returns the index of the first row out of order, or None."""
    for pos in range(1, len(rows)):
        prev, cur = rows[pos - 1], rows[pos]
        for column, descending, numeric in specs:
            a = sort_cell_key(prev[column], numeric)
            b = sort_cell_key(cur[column], numeric)
            if a == b:
                continue
            if a is None:
                return pos
            if b is None:
                break
            if (a > b) if not descending else (a < b):  # type: ignore[operator]
                return pos
            break
    return None


def check_scratch_location(scratch: Path, root: Path | None) -> None:
    """Refuses a scratch directory that is, contains or lies in ``root``."""
    if root is None:
        return
    s, r = scratch.resolve(), root.resolve()
    if s == r or s.is_relative_to(r) or r.is_relative_to(s):
        raise UsageError(f"--scratch {s} must not be, contain or lie inside --root {r}")


def prepare_scratch(scratch: Path) -> None:
    """Empties a scratch directory this script created before (it holds the
    marker file) or creates it; refuses any other non-empty directory."""
    if scratch.exists():
        if not scratch.is_dir():
            raise UsageError(f"--scratch {scratch} is not a directory")
        entries = list(scratch.iterdir())
        if entries and not (scratch / MARKER).exists():
            raise UsageError(
                f"--scratch {scratch} is not empty and was not created by this script"
            )
        shutil.rmtree(scratch)
    scratch.mkdir(parents=True)
    (scratch / MARKER).write_text(
        "rucene-base exercise scratch root\n", encoding="utf-8"
    )


def read_jsonl(path: Path) -> list[dict[str, object]]:
    """The records of a JSON Lines file."""
    out = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                out.append(json.loads(line))
    return out


def write_jsonl(path: Path, records: Iterable[dict[str, object]]) -> None:
    """Writes records as JSON Lines (UTF-8, no ASCII escaping)."""
    with path.open("w", encoding="utf-8") as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")


def parse_only(value: str | None) -> list[str]:
    """The scenarios selected by ``--only`` (all when absent), in run order."""
    if value is None:
        return list(ALL_SCENARIOS)
    wanted = [part.strip() for part in value.split(",") if part.strip()]
    unknown = [name for name in wanted if name not in ALL_SCENARIOS]
    if unknown or not wanted:
        raise UsageError(
            f"--only: unknown scenario(s) {unknown or [value]}; "
            f"known: {', '.join(ALL_SCENARIOS)}"
        )
    return [name for name in ALL_SCENARIOS if name in wanted]


# ------------------------------------------------------------------ the runner


@dataclass
class ScenarioResult:
    """The outcome of one scenario."""

    name: str
    passed: bool = True
    checks: int = 0
    commands: int = 0
    seconds: float = 0.0
    error: str = ""


@dataclass
class Exercise:
    """Drives the CLI and records the checks of the current scenario."""

    binary: Path
    root: Path | None
    scratch: Path | None
    results: list[ScenarioResult] = field(default_factory=list)

    # ---------------------------------------------------------------- basics

    @property
    def current(self) -> ScenarioResult:
        return self.results[-1]

    def check(self, cond: bool, what: str) -> None:
        self.current.checks += 1
        if not cond:
            raise CheckFailed(what)

    def say(self, text: str) -> None:
        print(f"    {text}", flush=True)

    def run(self, args: Sequence[str]) -> CliResult:
        """Runs the CLI with ``args``; never raises on a non-zero exit."""
        cmd = (str(self.binary), *args)
        t0 = time.perf_counter()
        proc = subprocess.run(
            cmd,
            capture_output=True,
            check=False,
            timeout=CLI_TIMEOUT_S,
        )
        elapsed = time.perf_counter() - t0
        self.current.commands += 1
        return CliResult(
            cmd,
            proc.returncode,
            proc.stdout.decode("utf-8"),
            proc.stderr.decode("utf-8"),
            elapsed,
        )

    def ok(self, args: Sequence[str]) -> str:
        """Runs the CLI and requires exit status 0; returns stdout."""
        res = self.run(args)
        self.check(
            res.code == 0,
            f"exit status {res.code}: {shown(res.cmd)}\n      stderr: {res.err.strip()}",
        )
        return res.out

    def need_root(self) -> Path:
        if self.root is None:
            raise UsageError("this scenario reads the full indices: give --root")
        return self.root

    def search(
        self,
        root: Path,
        entity: str,
        criteria: Sequence[str],
        fields: str,
        limit: int,
        sort: str | None = None,
    ) -> SearchOutput:
        args = [entity, "search", str(root), *criteria, "--fields", fields]
        args += ["--limit", str(limit), "--exact-total"]
        if sort:
            args += ["--sort", sort]
        out = self.ok(args)
        try:
            parsed = parse_tsv(out)
        except ValueError as exc:
            raise CheckFailed(f"{shown(args)}: {exc}") from exc
        self.check(
            parsed.exact, f"{shown(args)}: total is not exact with --exact-total"
        )
        return parsed

    def count(self, root: Path, entity: str, criteria: Sequence[str]) -> int:
        args = [entity, "count", str(root), *criteria]
        out = self.ok(args)
        try:
            return parse_count(out)
        except ValueError as exc:
            raise CheckFailed(f"{shown(args)}: {exc}") from exc

    def stats(self, root: Path, entity: str) -> dict[str, str]:
        args = [entity, "stats", str(root)]
        out = self.ok(args)
        try:
            return parse_stats(out)
        except ValueError as exc:
            raise CheckFailed(f"{shown(args)}: {exc}") from exc

    def get(
        self, root: Path, entity: str, keys: Sequence[str]
    ) -> list[dict[str, object]]:
        args = [entity, "get", str(root)]
        for key in keys:
            args += ["--key", key]
        out = self.ok(args)
        try:
            return parse_get(out)
        except (ValueError, TypeError) as exc:
            raise CheckFailed(f"{shown(args)}: {exc}") from exc

    def hit_keys(
        self, root: Path, entity: str, criteria: Sequence[str], key: str
    ) -> tuple[int, list[str]]:
        """Exact total and the keys of the first HIT_SET_LIMIT hits by _doc."""
        res = self.search(root, entity, criteria, key, HIT_SET_LIMIT, sort="_doc")
        return res.total, [row[key] for row in res.rows]

    # --------------------------------------------------- shared assertions

    def variant_group(
        self,
        root: Path,
        entity: str,
        flag: str,
        field_name: str | None,
        variants: Sequence[str],
        require_hits: bool = True,
    ) -> tuple[int, list[str]]:
        """Every variant must give the same count, total and hit set."""
        key = KEYS.get(entity)
        reference: tuple[int, list[str]] | None = None
        cells = []
        for variant in variants:
            if field_name is None:
                criteria = [flag, variant]
            else:
                criteria = [f"{flag}-in", f"{field_name}={variant}"]
            n = self.count(root, entity, criteria)
            if key is None:
                fields = "idcontrato,sequencia"
                res = self.search(root, entity, criteria, fields, HIT_SET_LIMIT, "_doc")
                got = (
                    res.total,
                    [f"{r['idcontrato']}/{r['sequencia']}" for r in res.rows],
                )
            else:
                got = self.hit_keys(root, entity, criteria, key)
            self.check(
                n == got[0],
                f"{entity} {shown(criteria)}: count {n} != search total {got[0]}",
            )
            cells.append(f"{variant!r}={n}")
            if reference is None:
                reference = got
                if require_hits:
                    self.check(n > 0, f"{entity} {shown(criteria)}: no hits")
                continue
            self.check(
                got[0] == reference[0],
                f"{entity} {flag} {variant!r}: total {got[0]} != {reference[0]} "
                f"of {variants[0]!r}",
            )
            self.check(
                got[1] == reference[1],
                f"{entity} {flag} {variant!r}: hit set differs from {variants[0]!r} "
                f"(first {HIT_SET_LIMIT} by _doc)",
            )
        target = f"{flag}-in {field_name}" if field_name else flag
        self.say(f"ok  {entity} {target}: " + ", ".join(cells) + " (same hit set)")
        assert reference is not None
        return reference

    # ============================================================ scenarios

    def scenario_stats(self) -> None:
        root = self.need_root()
        for entity in ENTITIES:
            st = self.stats(root, entity)
            docs = int(st["documents"])
            self.check(
                st["index"] == entity, f"{entity} stats: index is {st['index']!r}"
            )
            self.check(docs > 0, f"{entity}: the full index is empty")
            self.check(
                int(st["max_doc"]) >= docs,
                f"{entity}: max_doc {st['max_doc']} < documents {docs}",
            )
            total = self.count(root, entity, [])
            self.check(
                total == docs, f"{entity}: count of all {total} != documents {docs}"
            )
            self.say(
                f"ok  {entity}: documents={docs} deleted={st['deleted']} "
                f"max_doc={st['max_doc']} segments={st['segments']}"
            )

    def scenario_accent(self) -> None:
        root = self.need_root()
        for entity, flag, field_name, variants in ACCENT_GROUPS:
            self.variant_group(root, entity, flag, field_name, variants)
        # The hit set of the ação group mixes the source spellings.
        res = self.search(
            root,
            "contratos",
            ["--text", "acao"],
            "idcontrato,objectoContrato,descContrato",
            HIT_SET_LIMIT,
            "_doc",
        )
        spellings: dict[str, int] = {}
        pattern = re.compile(r"\b(a[cç][aã]o|a[cç][oõ]es)\b", re.IGNORECASE)
        for row in res.rows:
            for column in ("objectoContrato", "descContrato"):
                for m in pattern.finditer(row[column]):
                    spellings[m.group(0)] = spellings.get(m.group(0), 0) + 1
        self.check(
            len(spellings) >= 2,
            f"--text acao: hits hold only the spellings {sorted(spellings)}",
        )
        top = sorted(spellings.items(), key=lambda kv: (-kv[1], kv[0]))[:8]
        self.say(
            "ok  --text acao: source spellings among the first "
            f"{len(res.rows)} hits: " + ", ".join(f"{k!r}x{v}" for k, v in top)
        )

    def scenario_phrase(self) -> None:
        root = self.need_root()
        for entity, flag, field_name, variants in PHRASE_GROUPS:
            self.variant_group(root, entity, flag, field_name, variants)
        # A phrase is narrower than its words.
        words = self.count(root, "contratos", ["--text", "prestação de serviços"])
        phrase = self.count(root, "contratos", ["--phrase", "prestação de serviços"])
        self.check(
            0 < phrase <= words,
            f"phrase count {phrase} not in (0, words count {words}]",
        )
        self.say(f"ok  --phrase 'prestação de serviços' {phrase} <= --text {words}")

    def scenario_cpv(self) -> None:
        root = self.need_root()
        div = self.count(root, "contratos", ["--term", "cpvDivisao=45"])
        self.check(div > 0, "no contract in CPV division 45")
        sample = self.search(
            root, "contratos", ["--term", "cpvDivisao=45"], "idcontrato,cpv", 200
        )
        for row in sample.rows:
            self.check(
                any(c.startswith("45") for c in cell_values(row["cpv"])),
                f"contract {row['idcontrato']} in cpvDivisao=45 has cpv {row['cpv']!r}",
            )
        # A category code (digits 3 to 5 non-zero), so that the four levels
        # are four distinct prefixes rather than one padded with zeros.
        specific = [
            c
            for row in sample.rows
            for c in cell_values(row["cpv"])
            if c.startswith("45") and "0" not in c[2:5]
        ]
        self.check(
            bool(specific), "no category-level code among 200 hits of division 45"
        )
        code = min(specific)
        levels = [
            ("cpvDivisao", code[:2]),
            ("cpvGrupo", code[:3]),
            ("cpvClasse", code[:4]),
            ("cpvCategoria", code[:5]),
        ]
        counts = [
            self.count(root, "contratos", ["--term", f"{f}={v}"]) for f, v in levels
        ]
        for (f, v), n in zip(levels, counts, strict=True):
            self.say(f"    {f}={v}: {n}")
        self.check(
            all(counts[i] >= counts[i + 1] > 0 for i in range(3)),
            f"CPV hierarchy counts not non-increasing and positive: {counts}",
        )
        for i in range(3):
            (pf, pv), (cf, cv) = levels[i], levels[i + 1]
            inside = self.count(
                root, "contratos", ["--term", f"{pf}={pv}", "--term", f"{cf}={cv}"]
            )
            outside = self.count(
                root,
                "contratos",
                ["--term", f"{pf}={pv}", "--not", "--term", f"{cf}={cv}"],
            )
            self.check(
                inside == counts[i + 1] and inside + outside == counts[i],
                f"{pf}={pv}: {inside} with {cf}={cv} + {outside} without != {counts[i]}",
            )
        either = self.count(
            root,
            "contratos",
            [
                "--should",
                "--term",
                "cpvDivisao=45",
                "--should",
                "--term",
                "cpvDivisao=50",
            ],
        )
        terms = self.count(root, "contratos", ["--terms", "cpvDivisao=45,50"])
        self.check(either == terms, f"--should x2 {either} != --terms {terms}")
        self.say(
            f"ok  hierarchy {code}: {counts}; parent = child + (parent --not child) "
            f"at every level; --should 45/50 = --terms 45,50 = {terms}"
        )
        # The cpv index agrees on the division.
        cpv_div = self.count(root, "cpv", ["--term", "cpvDivisao=45"])
        rows = self.search(
            root, "cpv", ["--term", "cpvDivisao=45"], "cpv", 1000, "cpv:asc"
        )
        self.check(
            cpv_div > 0 and all(r["cpv"].startswith("45") for r in rows.rows),
            f"cpv index: division 45 holds {cpv_div} codes, or a code outside it",
        )
        self.say(f"ok  cpv index: {cpv_div} codes in division 45, all prefixed 45")

    def scenario_place(self) -> None:
        root = self.need_root()
        district = "Portugal, Lisboa"
        n = self.count(root, "contratos", ["--term", f"localNivel2={district}"])
        self.check(n > 0, f"no contract with localNivel2={district!r}")
        sample = self.search(
            root,
            "contratos",
            ["--term", f"localNivel2={district}"],
            "idcontrato,localExecucao",
            200,
        )
        for row in sample.rows:
            self.check(
                any(
                    v == district or v.startswith(district + ",")
                    for v in cell_values(row["localExecucao"])
                ),
                f"{row['idcontrato']}: localExecucao {row['localExecucao']!r} not in {district}",
            )
        places = self.search(
            root,
            "locais",
            ["--term", f"localNivel2={district}"],
            "local",
            1000,
            "local:asc",
        )
        keys = [r["local"] for r in places.rows]
        self.check(
            0 < len(keys) == places.total and len(keys) < 1000,
            f"locais under {district!r}: {places.total} places",
        )
        criteria: list[str] = []
        for key in keys:
            criteria += ["--should", "--term", f"localExecucao={key}"]
        union = self.count(root, "contratos", criteria)
        self.check(
            union == n,
            f"contracts in the {len(keys)} locais of {district!r}: {union} != localNivel2 {n}",
        )
        municipality = f"{district}, Lisboa"
        m = self.count(root, "contratos", ["--term", f"localNivel3={municipality}"])
        self.check(0 < m <= n, f"localNivel3={municipality!r} {m} not in (0, {n}]")
        self.say(
            f"ok  localNivel2={district!r}: {n} contracts = union of its {len(keys)} "
            f"locais; localNivel3={municipality!r}: {m}"
        )
        # The place hierarchy combines with text: contracts in Évora district.
        evora = self.count(root, "locais", ["--text-in", "distrito=evora"])
        evora_places = self.count(
            root, "locais", ["--term", "localNivel2=Portugal, Évora"]
        )
        self.check(
            evora == evora_places > 0,
            f"locais: --text-in distrito=evora {evora} != localNivel2 'Portugal, Évora' "
            f"{evora_places}",
        )
        self.say(
            f"ok  locais: distrito=evora (unaccented) = {evora} places of 'Portugal, Évora'"
        )

    def scenario_ranges(self) -> None:
        root = self.need_root()
        f = "dataPublicacao"
        year = self.count(root, "contratos", ["--range", f"{f}=2025-01-01..2025-12-31"])
        h1 = self.count(root, "contratos", ["--range", f"{f}=2025-01-01..2025-06-30"])
        h2 = self.count(root, "contratos", ["--range", f"{f}=2025-07-01..2025-12-31"])
        dmy = self.count(root, "contratos", ["--range", f"{f}=01/01/2025..31/12/2025"])
        self.check(year > 0 and year == h1 + h2, f"2025 {year} != H1 {h1} + H2 {h2}")
        self.check(dmy == year, f"DD/MM/YYYY range {dmy} != ISO range {year}")
        self.say(f"ok  {f} 2025: {year} = {h1} + {h2}; DD/MM/YYYY form gives {dmy}")
        res = self.search(
            root,
            "contratos",
            ["--range", f"{f}=2025-03-01..2025-03-31"],
            f"idcontrato,{f}",
            200,
            f"{f}:asc",
        )
        self.check(
            all("2025-03-01" <= r[f] <= "2025-03-31" for r in res.rows) and res.rows,
            f"{f} March 2025: a hit outside the range",
        )
        self.check(
            sort_violation(res.rows, [(f"sort:{f}:asc", False, False)]) is None,
            f"{f} March 2025: hits not ascending",
        )
        self.say(
            f"ok  {f} March 2025: {res.total} hits, first {len(res.rows)} in range, ascending"
        )

        p = "precoContratual"
        every = self.count(root, "contratos", ["--range", f"{p}=.."])
        low = self.count(root, "contratos", ["--range", f"{p}=..5000"])
        high = self.count(root, "contratos", ["--range", f"{p}=5000.."])
        at = self.count(root, "contratos", ["--range", f"{p}=5000..5000"])
        exact = self.count(root, "contratos", ["--term", f"{p}=5000"])
        self.check(
            every > 0 and low + high - at == every,
            f"{p}: ..5000 {low} + 5000.. {high} - 5000..5000 {at} != all {every}",
        )
        self.check(at == exact, f"{p}: range 5000..5000 {at} != --term 5000 {exact}")
        self.say(f"ok  {p}: {low} + {high} - {at} = {every}; --term 5000 = {exact}")
        res = self.search(
            root,
            "contratos",
            ["--range", f"{p}=1000..2000"],
            f"idcontrato,{p}",
            200,
            f"{p}:desc",
        )
        values = [float(r[p]) for r in res.rows]
        self.check(
            bool(values) and all(1000 <= v <= 2000 for v in values),
            f"{p} 1000..2000: a hit outside the range",
        )
        self.check(
            sort_violation(res.rows, [(f"sort:{p}:desc", True, True)]) is None,
            f"{p} 1000..2000: hits not descending",
        )
        self.say(
            f"ok  {p} 1000..2000: {res.total} hits, first {len(values)} in range, descending"
        )

    def scenario_sort(self) -> None:
        root = self.need_root()
        p = "precoContratual"
        top = self.search(
            root,
            "contratos",
            ["--text", "manutenção"],
            f"idcontrato,{p}",
            100,
            f"{p}:desc",
        )
        self.check(
            sort_violation(top.rows, [(f"sort:{p}:desc", True, True)]) is None,
            "--text manutenção --sort precoContratual:desc: not descending",
        )
        best = float(top.rows[0][p])
        above = self.count(
            root, "contratos", ["--text", "manutencao", "--range", f"{p}={best}.."]
        )
        self.check(above >= 1, f"no contract at or above the top price {best}")
        self.say(
            f"ok  --text manutenção sorted by {p}:desc; top {best}, {above} at or above it"
        )
        multi = self.search(
            root,
            "contratos",
            ["--term", "cpvDivisao=45"],
            "idcontrato,Ano,precoContratual",
            500,
            "Ano:desc,precoContratual:desc",
        )
        specs = [
            ("sort:Ano:desc", True, True),
            ("sort:precoContratual:desc", True, True),
        ]
        bad = sort_violation(multi.rows, specs)
        self.check(
            bad is None, f"--sort Ano:desc,precoContratual:desc: row {bad} out of order"
        )
        self.say(
            f"ok  --sort Ano:desc,precoContratual:desc: first {len(multi.rows)} rows ordered"
        )
        sup = self.search(
            root, "fornecedores", [], "nif,valorTotal", 50, "valorTotal:desc"
        )
        self.check(
            sort_violation(sup.rows, [("sort:valorTotal:desc", True, True)]) is None,
            "fornecedores --sort valorTotal:desc: not descending",
        )
        rel = self.search(
            root, "contratos", ["--text", "refeições escolares"], "idcontrato", 100
        )
        scores = [float(r["score"]) for r in rel.rows]
        self.check(
            all(scores[i] <= scores[i - 1] for i in range(1, len(scores))),
            "relevance order: scores increase",
        )
        self.say(
            f"ok  fornecedores by valorTotal:desc; relevance scores non-increasing "
            f"({len(scores)} hits)"
        )

    def scenario_supplier(self) -> None:
        root = self.need_root()
        top = self.search(
            root,
            "fornecedores",
            [],
            "nif,nome,numContratos,valorTotal",
            1,
            "valorTotal:desc",
        )
        row = top.rows[0]
        nif = row["nif"]
        one = self.search(root, "fornecedores", ["--term", f"nif={nif}"], "nif", 10)
        self.check(
            one.total == 1, f"fornecedores --term nif={nif}: {one.total} documents"
        )
        docs = self.get(root, "fornecedores", [nif])
        self.check(
            len(docs) == 1 and docs[0].get("nif") == nif, f"get {nif}: {len(docs)} docs"
        )
        names = as_list(docs[0].get("nome"))
        self.check(names == cell_values(row["nome"]), f"{nif}: get nome != search nome")
        name = names[0]
        variants = [name, fold(name), name.upper()]
        totals = []
        for variant in variants:
            res = self.search(
                root,
                "fornecedores",
                ["--phrase-in", f"nome={variant}"],
                "nif",
                1000,
                "_doc",
            )
            self.check(
                nif in [r["nif"] for r in res.rows],
                f"--phrase-in nome=<variant {len(totals)}> does not find {nif}",
            )
            totals.append(res.total)
        self.check(
            len(set(totals)) == 1, f"name variants of {nif} give totals {totals}"
        )
        contracts = self.count(root, "contratos", ["--term", f"adjudicatarioNif={nif}"])
        expected = int(row["numContratos"])
        self.check(
            contracts == expected,
            f"contratos with adjudicatarioNif={nif}: {contracts} != numContratos {expected}",
        )
        self.say(
            f"ok  top supplier {nif}: 1 document; name as stored / folded / upper "
            f"-> totals {totals}; {contracts} contracts = numContratos"
        )

    def scenario_crossref(self) -> None:
        root = self.need_root()
        fields = "idcontrato,numModificacoes,adjudicatarioNif,adjudicanteNif,idprocedimento,IDAnuncio"
        top = self.search(
            root,
            "contratos",
            ["--range", "numModificacoes=2.."],
            fields,
            1,
            "numModificacoes:desc",
        )
        self.check(top.total > 0, "no contract with two or more modifications")
        row = top.rows[0]
        cid, nmod = row["idcontrato"], int(row["numModificacoes"])
        mods = self.search(
            root,
            "modificacoes",
            ["--term", f"idcontrato={cid}"],
            "idcontrato,sequencia",
            1000,
            "sequencia:asc",
        )
        seqs = [int(r["sequencia"]) for r in mods.rows]
        self.check(
            mods.total == nmod and seqs == list(range(1, nmod + 1)),
            f"modificacoes of {cid}: total {mods.total}, sequences {seqs[:10]}..., "
            f"expected 1..{nmod}",
        )
        got = self.get(root, "modificacoes", [f"{cid}/1", f"{cid}/{nmod}"])
        self.check(
            [(d.get("idcontrato"), d.get("sequencia")) for d in got]
            == [(cid, 1), (cid, nmod)],
            f"modificacoes get {cid}/1 {cid}/{nmod}: {got!r:.200}",
        )
        self.say(f"ok  contract {cid}: numModificacoes {nmod} = modificacoes 1..{nmod}")
        for entity, column in (
            ("fornecedores", "adjudicatarioNif"),
            ("adjudicantes", "adjudicanteNif"),
        ):
            nifs = cell_values(row[column])
            self.check(bool(nifs), f"contract {cid} has no {column}")
            docs = self.get(root, entity, nifs)
            self.check(
                sorted(str(d.get("nif")) for d in docs) == sorted(nifs),
                f"{entity} get {nifs}: found {[d.get('nif') for d in docs]}",
            )
            for nif in nifs:
                n = self.count(
                    root, "contratos", ["--term", f"{column}={nif}", "--key", cid]
                )
                self.check(n == 1, f"contract {cid} not found by {column}={nif}")
            self.say(f"ok  {entity} of {cid}: {nifs} found by NIF")
        if row["idprocedimento"]:
            proc = self.get(root, "procedimentos", [row["idprocedimento"]])
            self.check(
                len(proc) == 1 and cid in as_list(proc[0].get("idcontrato")),
                f"procedimentos {row['idprocedimento']} does not list {cid}",
            )
            self.say(f"ok  procedimento {row['idprocedimento']} lists {cid}")
        if row["IDAnuncio"]:
            ann = self.get(root, "anuncios", [row["IDAnuncio"]])
            self.check(
                len(ann) == 1, f"anuncios get {row['IDAnuncio']}: {len(ann)} docs"
            )
            self.say(f"ok  anuncio {row['IDAnuncio']} found")
        # From a supplier to its contracts and back.
        nif = cell_values(row["adjudicatarioNif"])[0]
        sup = self.get(root, "fornecedores", [nif])[0]
        listed = self.count(root, "contratos", ["--term", f"adjudicatarioNif={nif}"])
        self.check(
            listed == int(str(sup.get("numContratos"))),
            f"supplier {nif}: {listed} contracts != numContratos {sup.get('numContratos')}",
        )
        self.say(f"ok  supplier {nif}: {listed} contracts = numContratos")

    # ------------------------------------------------------- write scenario

    def scenario_write(self) -> None:
        if self.scratch is None:
            raise UsageError("the write scenario needs --scratch")
        before = self.full_snapshot()
        work = self.scratch
        root = work / "root"
        self.write_contratos(root, work)
        self.write_fornecedores(root, work)
        self.write_modificacoes(root, work)
        after = self.full_snapshot()
        self.check(before == after, f"full indices changed: {before} -> {after}")
        if before:
            self.say(f"ok  full indices untouched: {before}")

    def full_snapshot(self) -> dict[str, tuple[str, str, str]]:
        if self.root is None:
            return {}
        out = {}
        for entity in ("contratos", "fornecedores", "modificacoes"):
            st = self.stats(self.root, entity)
            out[entity] = (st["documents"], st["max_doc"], st["segments"])
        return out

    def docs_of(self, root: Path, entity: str) -> int:
        return int(self.stats(root, entity)["documents"])

    def expect_hits(
        self,
        root: Path,
        entity: str,
        criteria: Sequence[str],
        key: str,
        expected: set[str],
        what: str,
    ) -> None:
        total, keys = self.hit_keys(root, entity, criteria, key)
        self.check(
            total == len(expected) and set(keys) == expected,
            f"{what}: {entity} {shown(criteria)} -> {sorted(keys)} (total {total}), "
            f"expected {sorted(expected)}",
        )

    def write_contratos(self, root: Path, work: Path) -> None:
        e, key = "contratos", "idcontrato"
        fixture = FIXTURES / f"{e}.jsonl"
        records = read_jsonl(fixture)
        self.ok([e, "create", str(root)])
        self.check(self.docs_of(root, e) == 0, "contratos: new index not empty")
        self.ok([e, "add", str(root), "--file", str(fixture)])
        self.check(
            self.docs_of(root, e) == len(records),
            f"contratos add --file: documents != {len(records)}",
        )
        self.say(f"ok  contratos create + add --file: {len(records)} documents")

        def model(forms: set[str]) -> set[str]:
            return model_matches(records, key, CONTRATOS_TEXT, forms)

        acao = model({"acao", "acoes"})
        self.check(bool(acao), "fixtures hold no ação/ações")
        for variant in ("ação", "acao", "AÇÃO", "ações", "acoes", "AÇÕES"):
            self.expect_hits(
                root, e, ["--text", variant], key, acao, "ação group vs model"
            )
        self.say(
            f"ok  contratos ação group (6 spellings) = Python model: {sorted(acao)}"
        )

        new_id = "EXERCISE-1"
        set_args = [
            "--set", f"idcontrato={new_id}",
            "--set", "objectoContrato=Fornecimento de PÃES e limões para a cantina",
            "--set", "dataPublicacao=15/06/2025",
            "--set", "precoContratual=1234.5",
            "--set", "Ano=2025",
            "--set", "cpv=15811100-7",
            "--set", "cpv=15331000-7",
            "--set", "localExecucao=Portugal, Évora, Évora",
            "--set", "CritMateriais=Não",
        ]  # fmt: skip
        self.ok([e, "add", str(root), *set_args])
        pao = model({"pao", "paes"}) | {new_id}
        for variant in ("pão", "pães", "paes", "PÃO"):
            self.expect_hits(root, e, ["--text", variant], key, pao, "after add --set")
        limao = model({"limao", "limoes"}) | {new_id}
        for variant in ("limão", "limoes", "LIMÕES"):
            self.expect_hits(
                root, e, ["--text", variant], key, limao, "after add --set"
            )
        for criteria in (
            ["--term", "cpvDivisao=15", "--key", new_id],
            ["--term", "cpvGrupo=153", "--key", new_id],
            ["--term", "localNivel2=Portugal, Évora", "--key", new_id],
            ["--term", "CritMateriais=nao", "--key", new_id],
            ["--range", "dataPublicacao=2025-06-15..2025-06-15", "--key", new_id],
            ["--range", "precoContratual=1234.5..1234.5", "--key", new_id],
        ):
            self.check(
                self.count(root, e, criteria) == 1, f"{new_id} not found by {criteria}"
            )
        doc = self.get(root, e, [new_id])
        self.check(len(doc) == 1, f"get {new_id}: {len(doc)} documents")
        stored = doc[0]
        self.check(
            stored.get("dataPublicacao") == "2025-06-15"
            and stored.get("cpv") == ["15811100-7", "15331000-7"]
            and stored.get("precoContratual") == 1234.5
            and stored.get("CritMateriais") == "nao",
            f"get {new_id}: stored values {stored}",
        )
        self.say(
            f"ok  add --set {new_id}: pão/limão groups, derived CPV/place levels, get"
        )

        self.ok([e, "add", str(root), *set_args])
        self.check(
            self.count(root, e, ["--key", new_id]) == 2, "add twice: not 2 documents"
        )
        changed = [
            "--set",
            f"idcontrato={new_id}",
            "--set",
            "objectoContrato=Aquisição de papéis",
        ]
        self.ok([e, "update", str(root), *changed])
        self.check(
            self.count(root, e, ["--key", new_id]) == 1, "update: not 1 document"
        )
        self.expect_hits(
            root, e, ["--text", "PÃO"], key, model({"pao", "paes"}), "after update"
        )
        papel = model({"papel", "papeis"}) | {new_id}
        for variant in ("papéis", "papeis", "papel", "PAPÉIS"):
            self.expect_hits(
                root, e, ["--text", variant], key, papel, "after update --set"
            )
        self.check(
            self.get(root, e, [new_id])[0].get("objectoContrato")
            == "Aquisição de papéis",
            "get after update --set: object not replaced",
        )
        self.check(
            self.docs_of(root, e) == len(records) + 1,
            "after add x2 + update: documents != fixtures + 1",
        )
        self.say(
            "ok  add twice -> 2 docs; update --set -> 1 doc, pão gone, papel group found"
        )

        target = dict(records[1])
        target_id = str(target[key])
        target["precoContratual"] = 987654.32
        target["objectoContrato"] = "Manutenção de AÇÕES de formação"
        patch = work / "contratos_update.jsonl"
        write_jsonl(patch, [target])
        self.ok([e, "update", str(root), "--file", str(patch)])
        got = self.get(root, e, [target_id])
        self.check(
            len(got) == 1 and got[0].get("precoContratual") == 987654.32,
            f"update --file {target_id}: {got!r:.300}",
        )
        self.check(
            self.count(
                root, e, ["--range", "precoContratual=987654.32..", "--key", target_id]
            )
            == 1,
            "update --file: new price not searchable",
        )
        self.check(
            self.count(root, e, ["--text", "acoes", "--key", target_id]) == 1,
            "update --file: new object not searchable",
        )
        self.say(
            f"ok  update --file {target_id}: price and object replaced and searchable"
        )

        gone = str(records[2][key])
        docs_before = self.docs_of(root, e)
        self.ok([e, "delete", str(root), "--key", new_id, "--key", gone])
        self.check(self.docs_of(root, e) == docs_before - 2, "delete 2 keys: documents")
        res = self.run([e, "get", str(root), "--key", gone, "--key", target_id])
        self.check(
            res.code == 1 and gone in res.err and len(parse_get(res.out)) == 1,
            f"get after delete: exit {res.code}, stderr {res.err.strip()!r}",
        )
        self.check(
            self.count(root, e, ["--key", new_id]) == 0, "deleted key still found"
        )
        bad = self.run(
            [e, "add", str(root), "--set", f"idcontrato={new_id}", "--set", "nope=1"]
        )
        self.check(
            bad.code == 1 and self.docs_of(root, e) == docs_before - 2,
            f"add with an unknown field: exit {bad.code}, index changed",
        )
        self.say(
            f"ok  delete {new_id} {gone}: documents -2, get reports the missing key (exit 1); "
            f"bad add exits {bad.code} with nothing committed"
        )

    def write_fornecedores(self, root: Path, work: Path) -> None:
        e, key = "fornecedores", "nif"
        fixture = FIXTURES / f"{e}.jsonl"
        records = read_jsonl(fixture)
        self.ok([e, "create", str(root)])
        self.ok([e, "add", str(root), "--file", str(fixture)])
        self.check(self.docs_of(root, e) == len(records), "fornecedores add --file")
        nif = "599999990"
        self.ok(
            [e, "add", str(root), "--set", f"nif={nif}",
             "--set", "nome=Construções Exemplo, Lda.", "--set", "pme=Sim",
             "--set", "numContratos=3", "--set", "valorTotal=1500"]
        )  # fmt: skip
        base = model_matches(records, key, ("nome",), {"construcao", "construcoes"})
        for variant in ("construções", "construcoes", "CONSTRUÇÕES", "construção"):
            self.expect_hits(
                root, e, ["--text-in", f"nome={variant}"], key, base | {nif}, "add"
            )
        self.check(
            self.count(root, e, ["--term", "nome=Construções Exemplo, Lda."]) == 1,
            "exact name copy not found",
        )
        self.ok(
            [e, "update", str(root), "--set", f"nif={nif}",
             "--set", "nome=Comércio Exemplo, Lda.", "--set", "nome=CONSTRUÇÕES EXEMPLO",
             "--set", "numContratos=4"]
        )  # fmt: skip
        self.check(self.count(root, e, ["--key", nif]) == 1, "update: not 1 document")
        self.check(
            self.count(root, e, ["--text-in", "nome=comercio", "--key", nif]) == 1
            and self.count(root, e, ["--text-in", "nome=construcoes", "--key", nif])
            == 1
            and self.count(root, e, ["--term", "numContratos=4", "--key", nif]) == 1
            and self.count(root, e, ["--term", "pme=sim", "--key", nif]) == 0,
            "update --set: replaced document not as given",
        )
        self.check(
            self.get(root, e, [nif])[0].get("nome")
            == ["Comércio Exemplo, Lda.", "CONSTRUÇÕES EXEMPLO"],
            "get after update: nome",
        )
        self.ok([e, "delete", str(root), "--key", nif])
        res = self.run([e, "get", str(root), "--key", nif])
        self.check(
            res.code == 1 and res.out == "", f"get deleted {nif}: exit {res.code}"
        )
        self.check(self.docs_of(root, e) == len(records), "fornecedores after delete")
        self.say(
            f"ok  fornecedores: add --file {len(records)}, add --set {nif} found by 4 spellings "
            "of construções, update --set (2 names, pme dropped), delete, get exit 1"
        )

    def write_modificacoes(self, root: Path, work: Path) -> None:
        e = "modificacoes"
        fixture = FIXTURES / f"{e}.jsonl"
        records = read_jsonl(fixture)
        self.ok([e, "create", str(root)])
        self.ok([e, "add", str(root), "--file", str(fixture)])
        self.check(self.docs_of(root, e) == len(records), "modificacoes add --file")
        first = records[0]
        cid, seq = str(first["idcontrato"]), int(str(first["sequencia"]))
        k = f"{cid}/{seq}"
        got = self.get(root, e, [k])
        self.check(
            len(got) == 1
            and got[0].get("idcontrato") == cid
            and got[0].get("sequencia") == seq,
            f"get {k}: {got!r:.200}",
        )
        nxt = f"{cid}/{seq + 1}"
        self.ok(
            [e, "update", str(root), "--set", f"idcontrato={cid}", "--set", f"sequencia={seq + 1}",
             "--set", "modifContratoFundamento=Prorrogação do prazo de execução",
             "--set", "modifContratoData=2025-10-01", "--set", "modifContratoPrecoAlterado=0"]
        )  # fmt: skip
        self.check(
            self.count(root, e, ["--term", f"idcontrato={cid}"])
            == sum(1 for r in records if str(r["idcontrato"]) == cid) + 1,
            f"modificacoes of {cid} after update --set (new key)",
        )
        for variant in ("prorrogação", "prorrogacao", "PRORROGAÇÃO"):
            self.check(
                self.count(root, e, ["--text", variant, "--key", nxt]) == 1,
                f"new modification not found by {variant!r}",
            )
        changed = dict(first)
        changed["modifContratoPrecoAlterado"] = 1.5
        patch = work / "modificacoes_update.jsonl"
        write_jsonl(patch, [changed])
        self.ok([e, "update", str(root), "--file", str(patch)])
        got = self.get(root, e, [k])
        self.check(
            len(got) == 1 and got[0].get("modifContratoPrecoAlterado") == 1.5,
            f"update --file {k}: {got!r:.200}",
        )
        self.ok([e, "delete", str(root), "--key", nxt, "--key", k])
        self.check(
            self.docs_of(root, e) == len(records) - 1, "modificacoes after delete"
        )
        res = self.run([e, "get", str(root), "--key", k])
        self.check(res.code == 1, f"get deleted {k}: exit {res.code}")
        self.say(
            f"ok  modificacoes: add --file {len(records)}, get {k}, update --set new key {nxt} "
            "(prorrogação x3), update --file price, delete 2 keys, get exit 1"
        )

    # ---------------------------------------------------------------- driver

    def run_scenario(self, name: str) -> ScenarioResult:
        self.results.append(ScenarioResult(name))
        print(f"== {name}", flush=True)
        method: Callable[[], None] = getattr(self, f"scenario_{name}")
        t0 = time.perf_counter()
        try:
            method()
        except (CheckFailed, UsageError, subprocess.TimeoutExpired, OSError) as exc:
            self.current.passed = False
            self.current.error = f"{type(exc).__name__}: {exc}"
            print(f"    FAIL {self.current.error}", flush=True)
        self.current.seconds = time.perf_counter() - t0
        return self.current


def report(results: Sequence[ScenarioResult]) -> str:
    """The final pass/fail table."""
    lines = ["", "scenario   result  checks  commands  seconds"]
    for r in results:
        verdict = "PASS" if r.passed else "FAIL"
        lines.append(
            f"{r.name:<10} {verdict:<7} {r.checks:>6}  {r.commands:>8}  {r.seconds:>7.1f}"
        )
    failed = [r.name for r in results if not r.passed]
    checks = sum(r.checks for r in results)
    if failed:
        lines.append(f"FAILED: {', '.join(failed)} ({checks} checks run)")
    else:
        lines.append(f"ALL SCENARIOS PASSED: {len(results)} scenarios, {checks} checks")
    return "\n".join(lines)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--binary", type=Path, required=True, help="the rucene-base binary"
    )
    parser.add_argument(
        "--root", type=Path, help="root of the full indices (read only)"
    )
    parser.add_argument(
        "--scratch",
        type=Path,
        help="directory for the write scenario (default: a new temporary directory)",
    )
    parser.add_argument(
        "--only", help=f"comma-separated scenarios: {','.join(ALL_SCENARIOS)}"
    )
    args = parser.parse_args(argv)
    try:
        scenarios = parse_only(args.only)
        if not args.binary.is_file():
            raise UsageError(f"--binary {args.binary} is not a file")
        needs_root = any(s in READ_SCENARIOS for s in scenarios)
        if needs_root and (args.root is None or not args.root.is_dir()):
            raise UsageError("read-only scenarios need --root, an existing directory")
        scratch: Path | None = None
        if "write" in scenarios:
            scratch = args.scratch or Path(
                tempfile.mkdtemp(prefix="rucene-base-exercise-")
            )
            check_scratch_location(scratch, args.root)
            prepare_scratch(scratch)
    except UsageError as exc:
        print(f"exercise_base: error: {exc}", file=sys.stderr)
        return 2
    ex = Exercise(args.binary.resolve(), args.root, scratch)
    print(f"binary: {ex.binary}\nroot: {args.root}\nscratch: {scratch}", flush=True)
    for name in scenarios:
        ex.run_scenario(name)
    print(report(ex.results), flush=True)
    return 0 if all(r.passed for r in ex.results) else 1


if __name__ == "__main__":
    sys.exit(main())
