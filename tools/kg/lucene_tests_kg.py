#!/usr/bin/env python3
"""
Leva a infraestrutura de testes do Apache Lucene Core 10.5.0 para o grafo, e
liga-a ao que o Rucene ja tem.

O grafo sabia, ate agora, o que esta portado do lado do *codigo*: 1.196 tipos de
`lucene/core`, o seu `portScope` e o seu `portState`. Nao sabia nada sobre o
lado dos *testes* -- nem que existe um modulo `lucene/test-framework` com a
maquinaria que todos os testes do Lucene usam, nem que `lucene/core` traz 755
ficheiros de teste. Sem isso, a pergunta "que testes faltam portar" nao tem
resposta no grafo, so uma impressao.

Este script fecha essa lacuna. Modela os dois lados:

  scan      percorre `lucene/test-framework/src/java` e `lucene/core/src/test`,
            e levanta pacotes, ficheiros, tipos de topo, `extends`/`implements`
            e metodos de teste. Emite um survey JSON.
  classes   carrega os `TestClass` (a superficie de teste do Lucene).
  methods   carrega os `TestMethod` (um por metodo de teste declarado).
  edges     DECLARES (pacote->classe, classe->metodo), EXTENDS entre classes de
            teste, e TESTS (classe de teste -> o tipo de `lucene/core` que
            exercita, deduzido do nome).
  coverage  a correspondencia com o Rucene: para cada `TestClass` que exercita
            um tipo ja portado, procura os testes Rust que cobrem esse tipo e
            liga-os por COVERED_BY; marca `ruceneCoverage` em cada `TestClass`.
  decision  regista a regra de ambito como um no `Decision`, auditavel.

A regra de ambito e deliberadamente mecanica, pelas mesmas razoes que a do
codigo (ver `port_coverage_kg.py`): **todo o tipo de topo declarado por um
ficheiro de `lucene/test-framework/src/java` ou de `lucene/core/src/test` esta
no ambito do porte de testes**, porque o `CLAUDE.md` (14.3) exige testes de
portabilidade como cidadaos de primeira classe e o `lucene/core` e o alvo.

Uso:
    python3 tools/kg/lucene_tests_kg.py rucene \\
        --lucene-root /tmp/lucene1050 --survey /tmp/rucene_kg/survey.json \\
        --commit <sha> --date <YYYY-MM-DD>
"""

import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from load_rucene_kg import esc, read, rmp, run_unwind  # noqa: E402

# ---------------------------------------------------------------------------
# Extraccao
# ---------------------------------------------------------------------------

# Uma declaracao de tipo de topo: comeca na coluna 0, porque um tipo aninhado
# vem sempre indentado. E a mesma regra que `extract_lucene_kg.py` usa para o
# lado do codigo, para que os dois lados sejam comparaveis.
TYPE_RE = re.compile(
    r"^(?P<mods>(?:(?:public|final|abstract|strictfp|sealed|non-sealed)\s+)*)"
    r"(?P<kind>class|interface|enum|record|@interface)\s+"
    r"(?P<name>[A-Za-z_$][A-Za-z0-9_$]*)",
    re.M,
)

EXTENDS_RE = re.compile(r"\bextends\s+([A-Za-z_$][A-Za-z0-9_$.]*)")
IMPLEMENTS_RE = re.compile(r"\bimplements\s+([^{]+)")
PACKAGE_RE = re.compile(r"^\s*package\s+([A-Za-z0-9_.]+)\s*;", re.M)

# Um metodo de teste do JUnit 4 tal como o Lucene os escreve: `public void
# testXxx()`, com ou sem `@Test`. O Lucene ainda usa a convencao do nome, por
# isso ela e o criterio; `@Test` sozinho (num metodo com outro nome) tambem
# conta, e por isso e procurado a parte.
TEST_METHOD_RE = re.compile(
    r"^\s*(?:@[A-Za-z][A-Za-z0-9_.]*(?:\([^)]*\))?\s*)*"
    r"(?:public\s+|protected\s+|private\s+)?"
    r"(?:final\s+|static\s+|synchronized\s+)*"
    r"void\s+(test[A-Za-z0-9_$]*)\s*\(",
    re.M,
)
ANNOTATED_TEST_RE = re.compile(
    r"@Test(?:\([^)]*\))?\s*(?:@[A-Za-z][A-Za-z0-9_.]*(?:\([^)]*\))?\s*)*"
    r"(?:public\s+|protected\s+|private\s+)?"
    r"(?:final\s+|static\s+|synchronized\s+)*"
    r"void\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*\(",
)

# Comentarios e literais fora do caminho, para que uma classe citada num
# javadoc nao seja lida como uma declaracao.
BLOCK_COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
LINE_COMMENT_RE = re.compile(r"//[^\n]*")
STRING_RE = re.compile(r'"(?:\\.|[^"\\])*"')
CHAR_RE = re.compile(r"'(?:\\.|[^'\\])'")


def strip_noise(text: str) -> str:
    """Substitui comentarios e literais por espacos, preservando as quebras de
    linha para que os numeros de linha continuem a bater certo."""

    def blank(m):
        return re.sub(r"[^\n]", " ", m.group(0))

    text = BLOCK_COMMENT_RE.sub(blank, text)
    text = LINE_COMMENT_RE.sub(blank, text)
    text = STRING_RE.sub(blank, text)
    text = CHAR_RE.sub(blank, text)
    return text


def classify_role(name: str, path: str, kind: str, is_framework: bool) -> str:
    """Diz o que a classe e, dentro da infraestrutura de testes.

    A funcao e mecanica de proposito: le o nome e o caminho, nunca o corpo. Um
    papel mal atribuido e visivel e corrigivel; um papel adivinhado a partir do
    conteudo nao seria auditavel.
    """
    if kind == "@interface":
        return "annotation"
    if name.startswith("Base") and (name.endswith("TestCase") or name.endswith("Tests")):
        return "base-test-case"
    if name.endswith("TestCase") or name.endswith("TestBase"):
        return "test-case-base"
    if name.startswith("Test") or name.endswith("Test") or name.endswith("Tests"):
        return "unit-test"
    if "Mock" in name:
        return "mock"
    if "Asserting" in name or name.startswith("Assert"):
        return "asserting"
    if "Cranky" in name:
        return "fault-injection"
    if "/codecs/" in path:
        return "test-codec"
    if "/mockfile/" in path:
        return "mock-filesystem"
    if is_framework:
        return "framework-util"
    return "test-helper"


def scan_tree(root: str, repo_root: str, module: str, kind: str):
    """Levanta um dos dois troncos de teste."""
    files = []
    classes = []
    methods = []
    packages = set()

    for dirpath, _dirnames, filenames in os.walk(root):
        for fn in sorted(filenames):
            if not fn.endswith(".java"):
                continue
            abs_path = os.path.join(dirpath, fn)
            rel_path = os.path.relpath(abs_path, repo_root)
            try:
                raw = open(abs_path, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            text = strip_noise(raw)

            pkg_m = PACKAGE_RE.search(text)
            package = pkg_m.group(1) if pkg_m else ""
            if package:
                packages.add(package)

            loc = raw.count("\n") + 1
            is_framework = kind == "framework"

            # Metodos de teste do ficheiro. O Lucene declara-os quase sempre no
            # tipo de topo; atribui-los ao tipo de topo e por isso exacto na
            # esmagadora maioria dos casos, e conservador nos restantes.
            names = []
            seen = set()
            for m in TEST_METHOD_RE.finditer(text):
                if m.group(1) not in seen:
                    seen.add(m.group(1))
                    names.append(m.group(1))
            for m in ANNOTATED_TEST_RE.finditer(text):
                if m.group(1) not in seen:
                    seen.add(m.group(1))
                    names.append(m.group(1))

            declared = []
            for m in TYPE_RE.finditer(text):
                name = m.group("name")
                java_kind = m.group("kind")
                if java_kind == "@interface":
                    java_kind_name = "annotation"
                else:
                    java_kind_name = java_kind
                tail = text[m.end() : m.end() + 400]
                ext = EXTENDS_RE.search(tail)
                impl = IMPLEMENTS_RE.search(tail)
                implements = []
                if impl:
                    implements = [
                        p.strip().split("<")[0]
                        for p in impl.group(1).split(",")
                        if p.strip()
                    ]
                declared.append(
                    {
                        "name": name,
                        "qualifiedName": f"{package}.{name}" if package else name,
                        "kind": java_kind_name,
                        "package": package,
                        "file": rel_path,
                        "module": module,
                        "testKind": kind,
                        "role": classify_role(name, rel_path, java_kind, is_framework),
                        "abstract": "abstract" in m.group("mods"),
                        "extends": ext.group(1).split("<")[0] if ext else "",
                        "implements": implements,
                    }
                )

            # O primeiro tipo de topo e o publico do ficheiro; os metodos de
            # teste do ficheiro pertencem-lhe.
            owner = declared[0] if declared else None
            if owner is not None:
                owner["testMethodCount"] = len(names)
                for n in names:
                    methods.append(
                        {
                            "name": n,
                            "qualifiedName": f"{owner['qualifiedName']}#{n}",
                            "parentQualifiedName": owner["qualifiedName"],
                            "file": rel_path,
                            "module": module,
                        }
                    )
            for d in declared[1:]:
                d.setdefault("testMethodCount", 0)

            files.append(
                {
                    "path": rel_path,
                    "name": fn,
                    "kind": "test",
                    "package": package,
                    "module": module,
                    "testKind": kind,
                    "loc": loc,
                    "types": len(declared),
                    "testMethods": len(names),
                }
            )
            classes.extend(declared)

    return files, classes, methods, sorted(packages)


def phase_scan(lucene_root: str, out_path: str):
    fw_root = os.path.join(
        lucene_root, "lucene/test-framework/src/java/org/apache/lucene"
    )
    core_root = os.path.join(lucene_root, "lucene/core/src/test/org/apache/lucene")
    for p in (fw_root, core_root):
        if not os.path.isdir(p):
            print(f"  nao encontrado: {p}", file=sys.stderr)
            sys.exit(2)

    fw_files, fw_classes, fw_methods, fw_pkgs = scan_tree(
        fw_root, lucene_root, "lucene/test-framework", "framework"
    )
    core_files, core_classes, core_methods, core_pkgs = scan_tree(
        core_root, lucene_root, "lucene/core", "unit-test"
    )

    survey = {
        "files": fw_files + core_files,
        "classes": fw_classes + core_classes,
        "methods": fw_methods + core_methods,
        "packages": sorted(set(fw_pkgs) | set(core_pkgs)),
    }
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    json.dump(survey, open(out_path, "w", encoding="utf-8"))
    print(
        f"  framework: files={len(fw_files)} types={len(fw_classes)} "
        f"testMethods={len(fw_methods)}",
        file=sys.stderr,
    )
    print(
        f"  core tests: files={len(core_files)} types={len(core_classes)} "
        f"testMethods={len(core_methods)}",
        file=sys.stderr,
    )
    return survey


# ---------------------------------------------------------------------------
# Carga
# ---------------------------------------------------------------------------


def stamp_of(commit, date):
    return f"c.gitCommit = {esc(commit)}, c.gitDate = {esc(date)}"


def phase_classes(roadmap, survey, commit, date, batch_size):
    print("phase: classes", file=sys.stderr)
    rows = [
        {
            "qn": c["qualifiedName"],
            "n": c["name"],
            "k": c["kind"],
            "p": c["package"],
            "f": c["file"],
            "m": c["module"],
            "tk": c["testKind"],
            "r": c["role"],
            "ab": bool(c["abstract"]),
            "tm": int(c.get("testMethodCount", 0)),
        }
        for c in survey["classes"]
    ]
    # `rmp graph create` so aceita CREATE/MERGE puros, e `graph update` so
    # SET/REMOVE, por isso cada no entra em duas passagens: primeiro a
    # identidade, depois as propriedades.
    run_unwind(
        "create",
        roadmap,
        rows,
        "MERGE (c:TestClass {qualifiedName: row.qn})",
        batch_size,
        "TestClass create",
    )
    run_unwind(
        "update",
        roadmap,
        rows,
        "MATCH (c:TestClass {qualifiedName: row.qn}) "
        "SET c.name = row.n, c.kind = row.k, c.package = row.p, c.file = row.f, "
        "c.module = row.m, c.testKind = row.tk, c.role = row.r, "
        "c.isAbstract = row.ab, c.testMethodCount = row.tm, "
        f"{stamp_of(commit, date)}",
        batch_size,
        "TestClass update",
    )


def phase_methods(roadmap, survey, commit, date, batch_size):
    print("phase: methods", file=sys.stderr)
    rows = [
        {
            "qn": m["qualifiedName"],
            "n": m["name"],
            "pq": m["parentQualifiedName"],
            "f": m["file"],
            "mo": m["module"],
        }
        for m in survey["methods"]
    ]
    run_unwind(
        "create",
        roadmap,
        rows,
        "MERGE (c:TestMethod {qualifiedName: row.qn})",
        batch_size,
        "TestMethod create",
    )
    run_unwind(
        "update",
        roadmap,
        rows,
        "MATCH (c:TestMethod {qualifiedName: row.qn}) "
        "SET c.name = row.n, c.parentQualifiedName = row.pq, c.file = row.f, "
        f"c.module = row.mo, {stamp_of(commit, date)}",
        batch_size,
        "TestMethod update",
    )


def phase_edges(roadmap, survey, commit, date, batch_size):
    print("phase: edges", file=sys.stderr)

    # TestClass -> TestMethod
    run_unwind(
        "create",
        roadmap,
        [
            {"c": m["parentQualifiedName"], "m": m["qualifiedName"]}
            for m in survey["methods"]
        ],
        "MATCH (c:TestClass {qualifiedName: row.c}), "
        "(m:TestMethod {qualifiedName: row.m}) MERGE (c)-[:DECLARES]->(m)",
        batch_size,
        "TestClass DECLARES TestMethod",
    )

    # TestClass -> TestClass, por `extends`.
    by_simple = {}
    for c in survey["classes"]:
        by_simple.setdefault(c["name"], []).append(c["qualifiedName"])
    ext_rows = []
    for c in survey["classes"]:
        base = c["extends"].split(".")[-1] if c["extends"] else ""
        if not base:
            continue
        targets = by_simple.get(base, [])
        if len(targets) == 1:
            ext_rows.append({"a": c["qualifiedName"], "b": targets[0]})
    run_unwind(
        "create",
        roadmap,
        ext_rows,
        "MATCH (a:TestClass {qualifiedName: row.a}), "
        "(b:TestClass {qualifiedName: row.b}) MERGE (a)-[:EXTENDS]->(b)",
        batch_size,
        "TestClass EXTENDS TestClass",
    )

    # TestClass -> Class: o tipo de `lucene/core` que a classe de teste
    # exercita, deduzido do nome. Uma deducao, nao um facto: fica registada na
    # propriedade `evidence` da aresta, para que quem a leia saiba disso.
    core = {
        r["name"]: r["qn"]
        for r in read(
            roadmap,
            "MATCH (c:Class) WHERE c.portScope = 'in' "
            "RETURN c.name AS name, c.qualifiedName AS qn",
        )
    }
    dup = {}
    for r in read(
        roadmap,
        "MATCH (c:Class) WHERE c.portScope = 'in' RETURN c.name AS name, count(*) AS n",
    ):
        dup[r["name"]] = r["n"]

    tests_rows = []
    for c in survey["classes"]:
        subject = subject_of(c["name"])
        if not subject:
            continue
        if dup.get(subject, 0) != 1:
            continue
        tests_rows.append({"t": c["qualifiedName"], "s": core[subject]})
    run_unwind(
        "create",
        roadmap,
        tests_rows,
        "MATCH (t:TestClass {qualifiedName: row.t}), "
        "(c:Class {qualifiedName: row.s}) MERGE (t)-[:TESTS]->(c)",
        batch_size,
        "TestClass TESTS Class",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (:TestClass)-[r:TESTS]->(:Class) SET r.evidence = 'name-convention'",
        "TESTS provenance",
    )
    print(f"  subjects resolved: {len(tests_rows)}", file=sys.stderr)
    return {r["t"] for r in tests_rows}


def subject_of(name: str):
    """O tipo que uma classe de teste exercita, pela convencao de nomes do
    Lucene: `TestFoo` -> `Foo`, `FooTest` -> `Foo`, `BaseFooTestCase` -> `Foo`.

    Devolve `None` quando o nome nao segue nenhuma delas -- e melhor nao ligar
    do que ligar ao tipo errado.
    """
    if name.startswith("Base") and name.endswith("TestCase"):
        return name[len("Base") : -len("TestCase")] or None
    if name.startswith("Test") and len(name) > 4:
        return name[4:]
    if name.endswith("Test") and len(name) > 4:
        return name[:-4]
    if name.endswith("Tests") and len(name) > 5:
        return name[:-5]
    return None


def phase_coverage(roadmap, commit, date, batch_size):
    """A correspondencia com o Rucene.

    Para cada `TestClass` que exercita um tipo de `lucene/core`, pergunta ao
    grafo se esse tipo esta portado e, se estiver, se ha testes Rust no ficheiro
    que o declara. E a unica ligacao defensavel: um teste Rust nao nomeia o tipo
    Lucene que cobre, mas vive no ficheiro que declara o porte desse tipo.
    """
    print("phase: coverage", file=sys.stderr)

    pairs = read(
        roadmap,
        "MATCH (t:TestClass)-[:TESTS]->(c:Class) "
        "RETURN t.qualifiedName AS t, c.qualifiedName AS c, c.portState AS st",
    )

    # ficheiro Rust de cada tipo de `lucene/core` portado
    rust_files = {}
    for r in read(
        roadmap,
        "MATCH (x)-[:PORTS|PORTS_CANDIDATE]->(c:Class) WHERE c.portScope = 'in' "
        "RETURN c.qualifiedName AS c, x.file AS f",
    ):
        if r["f"]:
            rust_files.setdefault(r["c"], set()).add(r["f"])

    # ficheiros Rust que declaram pelo menos um teste
    tested_files = {
        r["f"]
        for r in read(
            roadmap,
            "MATCH (f:RustFn) WHERE f.kind = 'test' RETURN DISTINCT f.file AS f",
        )
        if r["f"]
    }

    covered, uncovered, unported = [], [], []
    cover_edges = []
    for p in pairs:
        files = rust_files.get(p["c"], set())
        if not files:
            unported.append(p["t"])
            continue
        hit = sorted(files & tested_files)
        if hit:
            covered.append(p["t"])
            for f in hit:
                cover_edges.append({"t": p["t"], "f": f})
        else:
            uncovered.append(p["t"])

    for state, names in (
        ("covered", covered),
        ("uncovered", uncovered),
        ("subject-unported", unported),
    ):
        run_unwind(
            "update",
            roadmap,
            [{"t": n} for n in names],
            "MATCH (c:TestClass {qualifiedName: row.t}) "
            f"SET c.ruceneCoverage = {esc(state)}, {stamp_of(commit, date)}",
            batch_size,
            f"ruceneCoverage = {state}",
        )

    run_unwind(
        "create",
        roadmap,
        cover_edges,
        "MATCH (t:TestClass {qualifiedName: row.t}), (f:File {path: row.f}) "
        "MERGE (t)-[:COVERED_BY]->(f)",
        batch_size,
        "TestClass COVERED_BY File",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (:TestClass)-[r:COVERED_BY]->(:File) "
        "SET r.evidence = 'ported-type-file-has-tests'",
        "COVERED_BY provenance",
    )

    # Tudo o que nao tem sujeito resolvido fica explicitamente sem cobertura
    # conhecida, em vez de ficar sem propriedade nenhuma.
    rmp(
        "update",
        roadmap,
        "MATCH (c:TestClass) WHERE c.ruceneCoverage IS NULL "
        "SET c.ruceneCoverage = 'no-subject-resolved'",
        "ruceneCoverage = no-subject-resolved",
    )
    print(
        f"  covered={len(covered)} uncovered={len(uncovered)} "
        f"subject-unported={len(unported)}",
        file=sys.stderr,
    )


def scan_rucene_tests(repo_root: str):
    """Levanta a infraestrutura de teste do lado do Rucene.

    O Rucene **nao portou** o `lucene/test-framework`. Construiu outra coisa: um
    harness Java que conduz o Lucene 10.5.0 real e imprime os bytes e os valores
    de referencia, e testes Rust que os confrontam nos dois sentidos de leitura e
    escrita. Registar isso e o que torna a correspondencia honesta -- sem ele o
    grafo diria "0% do framework portado" e omitiria que existe uma infra
    equivalente em proposito, diferente em forma.
    """
    fixtures = []
    fixture_root = os.path.join(repo_root, "tests/fixtures")
    for dirpath, dirnames, filenames in os.walk(fixture_root):
        # `target/` e saida do Maven, nao fonte.
        dirnames[:] = [d for d in dirnames if d != "target"]
        for fn in sorted(filenames):
            if not fn.endswith(".java"):
                continue
            abs_path = os.path.join(dirpath, fn)
            rel = os.path.relpath(abs_path, repo_root)
            try:
                raw = open(abs_path, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            fixtures.append(
                {
                    "path": rel,
                    "name": fn,
                    "stem": fn[: -len(".java")],
                    "loc": raw.count("\n") + 1,
                }
            )
    return fixtures


def phase_rucene(roadmap, repo_root, commit, date, batch_size):
    """Carrega o harness de fixtures e liga cada teste de portabilidade ao seu."""
    print("phase: rucene", file=sys.stderr)
    fixtures = scan_rucene_tests(repo_root)
    rows = [
        {"p": f["path"], "n": f["name"], "l": f["loc"]}
        for f in fixtures
    ]
    run_unwind(
        "create",
        roadmap,
        rows,
        "MERGE (f:File {path: row.p})",
        batch_size,
        "fixture File create",
    )
    run_unwind(
        "update",
        roadmap,
        rows,
        "MATCH (f:File {path: row.p}) SET f.name = row.n, f.kind = 'test-fixture', "
        "f.language = 'Java', f.crate = 'java-codec-harness', f.loc = row.l, "
        f"f.gitCommit = {esc(commit)}, f.gitDate = {esc(date)}",
        batch_size,
        "fixture File update",
    )

    # Liga cada ficheiro de teste de portabilidade ao fixture que o alimenta,
    # pela raiz do nome: tests/portability/norms.rs <-> NormsFixture.java e
    # NormsReaderFixture.java.
    port_files = [
        r["p"]
        for r in read(
            roadmap,
            "MATCH (f:File) WHERE f.path STARTS WITH 'tests/portability/' "
            "RETURN f.path AS p",
        )
    ]
    pairs = []
    for pf in port_files:
        stem = os.path.basename(pf)[: -len(".rs")].replace("_", "")
        for fx in fixtures:
            if fx["stem"].lower().startswith(stem.lower()) and stem:
                pairs.append({"a": pf, "b": fx["path"]})
    run_unwind(
        "create",
        roadmap,
        pairs,
        "MATCH (a:File {path: row.a}), (b:File {path: row.b}) "
        "MERGE (a)-[:DEPENDS_ON]->(b)",
        batch_size,
        "portability test DEPENDS_ON fixture",
    )
    print(
        f"  fixtures={len(fixtures)} portability-files={len(port_files)} "
        f"links={len(pairs)}",
        file=sys.stderr,
    )
    return fixtures


def phase_rucene_decision(roadmap, fixtures, commit, date):
    """Regista, como observacao auditavel, a forma que a infra de teste do
    Rucene tomou -- e que ela nao e um porte do `lucene/test-framework`."""
    print("phase: rucene-decision", file=sys.stderr)
    loc = sum(f["loc"] for f in fixtures)
    summary = (
        "Rucene does not port org.apache.lucene.tests (the lucene/test-framework "
        f"module, 212 types). Its test infrastructure is a different construction: "
        f"{len(fixtures)} Java fixture programs ({loc} lines under tests/fixtures/) "
        "that drive real Lucene 10.5.0 and emit reference bytes and decoded values, "
        "plus Rust tests that check three directions per format -- Rucene writes "
        "bytes identical to Lucene's, Rucene reads what Lucene wrote, and Lucene "
        "reads what Rucene wrote."
    )
    rationale = (
        "This is recorded as an observation of the code as it stands, not as a "
        "decision anyone signed off. It is what the repository does today, and it "
        "serves CLAUDE.md 14.3 directly: a differential harness against the real "
        "reference implementation proves index compatibility, which is the stated "
        "goal, whereas LuceneTestCase and the Base*TestCase suites prove internal "
        "consistency against Lucene's own expectations."
    )
    alternatives = (
        "Porting the framework itself is unexplored. It would bring randomised "
        "testing with reproducible seeds, MockDirectoryWrapper's fault injection "
        "and unclosed-file detection, the assertion codecs that check codec "
        "contracts on every call, and the 41 Base*TestCase suites that a codec "
        "implementation can simply extend to inherit hundreds of conformance "
        "tests. Those have no counterpart in the crate today. Whether to port them "
        "is an open question for the maintainer, not something this survey decides."
    )
    evidence = (
        "grep for LuceneTestCase, MockDirectoryWrapper, MockAnalyzer, "
        "RandomIndexWriter, BaseTokenStreamTestCase, TestUtil, AssertingCodec, "
        "MockRandomPostingsFormat, LineFileDocs and BaseDirectoryTestCase across "
        "src/ and tests/ returns nothing. tests/fixtures/ holds the 22-file Java "
        "harness; tests/portability/ holds 12 Rust suites that consume it."
    )
    name = "Rucene tests Lucene differentially instead of porting lucene/test-framework"
    rmp("create", roadmap, f"MERGE (d:Decision {{name: {esc(name)}}})", "Decision create")
    rmp(
        "update",
        roadmap,
        f"MATCH (d:Decision {{name: {esc(name)}}}) "
        f"SET d.kind = 'adaptation', d.summary = {esc(summary)}, "
        f"d.rationale = {esc(rationale)}, d.alternatives = {esc(alternatives)}, "
        f"d.evidence = {esc(evidence)}, d.gitCommit = {esc(commit)}, "
        f"d.gitDate = {esc(date)}",
        "Decision update",
    )


def phase_decision(roadmap, survey, commit, date):
    print("phase: decision", file=sys.stderr)
    n_fw = sum(1 for c in survey["classes"] if c["module"] == "lucene/test-framework")
    n_core = len(survey["classes"]) - n_fw
    n_m = len(survey["methods"])
    summary = (
        "Every top-level type declared by a file of lucene/test-framework/src/java "
        f"({n_fw} types) or of lucene/core/src/test ({n_core} types) is in scope for "
        f"the test port, and so are the {n_m} test methods they declare. The two "
        "trees are modelled separately because they are different obligations: the "
        "framework is machinery every test needs, the core tree is the test corpus "
        "itself."
    )
    rationale = (
        "CLAUDE.md 14.3 makes portability tests first-class and requires them to pass "
        "before a task is complete, and 16.1 names lucene/core as the reference tree. "
        "A test port therefore has the same denominator problem the code port had: "
        "without the Lucene test surface in the graph, 'which tests are missing' is an "
        "impression rather than a query. This node records the rule that makes it a "
        "query."
    )
    alternatives = (
        "Scoping only lucene/core/src/test was rejected: those 755 files nearly all "
        "extend LuceneTestCase and use MockDirectoryWrapper, the assertion codecs and "
        "the Base*TestCase suites, so the corpus cannot be ported without the "
        "framework and the framework is the larger engineering task of the two. "
        "Scoping by hand-picked 'important' tests was rejected for the same reason "
        "the code scope was: a denominator chosen by taste is not defensible."
    )
    evidence = (
        f"Counted from the 10.5.0 sources: {n_fw + n_core} top-level types across "
        f"{len(survey['files'])} files, declaring {n_m} test methods."
    )
    rmp(
        "create",
        roadmap,
        "MERGE (d:Decision {name: 'Test port scope is the Lucene test-framework plus "
        "every lucene/core test'})",
        "Decision create",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (d:Decision {name: 'Test port scope is the Lucene test-framework plus "
        "every lucene/core test'}) "
        f"SET d.kind = 'principle', d.summary = {esc(summary)}, "
        f"d.rationale = {esc(rationale)}, d.alternatives = {esc(alternatives)}, "
        f"d.evidence = {esc(evidence)}, d.gitCommit = {esc(commit)}, "
        f"d.gitDate = {esc(date)}",
        "Decision",
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--out", default="/tmp/lucene_tests_kg/survey.json")
    ap.add_argument("--repo-root", default=".")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--batch-size", type=int, default=200)
    ap.add_argument(
        "--phase",
        default="all",
        choices=[
            "all",
            "scan",
            "classes",
            "methods",
            "edges",
            "coverage",
            "rucene",
            "decision",
        ],
    )
    args = ap.parse_args()

    if args.phase in ("all", "scan") or not os.path.exists(args.out):
        print("phase: scan", file=sys.stderr)
        survey = phase_scan(args.lucene_root, args.out)
    else:
        survey = json.load(open(args.out, encoding="utf-8"))

    if args.phase == "scan":
        return
    if args.phase in ("all", "classes"):
        phase_classes(args.roadmap, survey, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "methods"):
        phase_methods(args.roadmap, survey, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "edges"):
        phase_edges(args.roadmap, survey, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "coverage"):
        phase_coverage(args.roadmap, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "rucene"):
        fixtures = phase_rucene(
            args.roadmap, args.repo_root, args.commit, args.date, args.batch_size
        )
        phase_rucene_decision(args.roadmap, fixtures, args.commit, args.date)
    if args.phase in ("all", "decision"):
        phase_decision(args.roadmap, survey, args.commit, args.date)


if __name__ == "__main__":
    main()
