#!/usr/bin/env python3
"""The modules the port covers, surveyed and tracked as `lucene/core` is.

`CLAUDE.md` 6.1 requires the graph to say, at any moment, what of the port's
scope is ported and what is missing. The user has widened that scope twice:

* 2026-11-03, sprint 9 ("Port Tests Framework"): Lucene 10.5.0's
  `lucene/test-framework` and `lucene/codecs` and the runner they are built on,
  randomizedtesting 2.8.1 (`randomized-runner`), as a Cargo workspace with one
  crate per module. Before this tool the graph held the opposite decision
  ("Rucene does not port org.apache.lucene.tests"), `inScope: false` on both
  Lucene modules, the framework only as `TestClass` nodes, and nothing of
  randomizedtesting.
* 2026-11-13, sprint 12 ("Port Analysis Common Module"): Lucene 10.5.0's
  `lucene/analysis/common`, as the workspace crate `rucene-analysis-common`,
  ported 100% faithfully. Its types are tracked through a Rust skeleton
  (`SKELETON_OF`, `portState = 'skeleton'`) before each is ported.
* 2026-11-14, sprint 13 ("Port Query Modules"): Lucene 10.5.0's
  `lucene/queries` and `lucene/queryparser`, whole, as the workspace crates
  `rucene-queries` and `rucene-queryparser`, and of `lucene/sandbox` only
  `org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery` (with its nested
  types), which `queryparser.xml.builders.FuzzyLikeThisQueryBuilder` needs, as
  the crate `rucene-sandbox`. The sandbox entry carries a source-file
  allow-list (`files`), so no other sandbox type enters the scope.
* 2026-11-22, sprint 17 ("Port the benchmark module and its dependency
  modules"): Lucene 10.5.0's `lucene/benchmark` and every module it depends on,
  whole -- `lucene/memory`, `lucene/facet`, `lucene/highlighter`,
  `lucene/spatial3d`, `lucene/spatial-extras` and `lucene/spatial-test-fixtures`
  -- as the crates `rucene-benchmark`, `rucene-memory`, `rucene-facet`,
  `rucene-highlighter`, `rucene-spatial3d`, `rucene-spatial-extras` and
  `rucene-spatial-test-fixtures`; and, as randomizedtesting was, two external
  Java libraries ported as their own crates and `Project`s, each narrowed by a
  `files` allow-list: spatial4j 0.8 (`spatial4j`) and s2-geometry-library-java
  1.0.0 (`s2-geometry`), the versions Lucene's `gradle/libs.versions.toml` pins.

`PORT_MODULES` is the single statement of that scope. Every other KG tool that
needs to know which module a Java file belongs to, which crate ports it, or
which source trees to census imports it from here:

    port_coverage_kg.py   portScope / portState / Class.module, per module
    nested_port_kg.py     the nested-type measurement, which stays core-only
    audit_kg.py           the independent census, per module
    extract_rucene_kg.py  the workspace crates (`cratePath`)

Phases (`--phase`, default `all`):

  modules    the port `Module` nodes' scope properties (`scopeFiles` on a
             module scoped to listed files), the randomizedtesting
             `Project`, `Module` -> `Module` `DEPENDS_ON` from each module's
             JPMS descriptor, and one scope `Decision` per widening
             (`DECISIONS`), with its `SUPERSEDES` and `CONCERNS` to each
             module it brings in.
  structure  packages, files, top-level and nested types, members, `EXTENDS` /
             `IMPLEMENTS`, and `TestClass` -[:PORT_UNIT]-> `Class`, for every
             module beyond `lucene/core` (`ADDED_MODULES`), reusing
             `extract_lucene_kg.discover` and
             `lucene_structure_kg`'s loaders over each module's source tree;
             nodes the trees no longer declare are pruned.
  coverage   prints the per-module coverage (`COVERAGE_QUERY`).

`early` (not part of `all`) records a member ported early under the sprint's
cycle rule: a task that calls a member of a class owned by a later task ports
that member in full and records it here, as `Task -[:PORTS_EARLY]-> Method`.

Usage:
    python3 tools/kg/port_modules_kg.py rucene --lucene-root /tmp/lucene1050 \\
        --rt-root "$RANDOMIZEDTESTING_ROOT" --commit "$COMMIT" --date "$DATE"
    python3 tools/kg/port_modules_kg.py rucene --phase early --task 330 \\
        --owner-task 345 --member '<Method.qualifiedName>' \\
        --rust-item '<RustFn.qualifiedName>' --commit "$COMMIT" --date "$DATE"
"""
import argparse
import os
import re
import sys
from contextlib import contextmanager
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kgio import esc, read, rmp, unwind  # noqa: E402

LUCENE = "Apache Lucene Core 10.5.0"
RT_PROJECT = "randomizedtesting 2.8.1"
DEFAULT_LUCENE_ROOT = "/tmp/lucene1050"
# The two external libraries sprint 17 ports (`files`-narrowed modules below).
# spatial4j 0.8 and s2-geometry-library-java 1.0.0 are the versions Lucene
# 10.5.0 pins (`gradle/libs.versions.toml`: `spatial4j = "0.8"`,
# `s2-geometry = "1.0.0"`, `io.sgr:s2-geometry-library-java`). Obtain them with
#   git clone --depth 1 --branch spatial4j-0.8 \
#       https://github.com/locationtech/spatial4j.git <dir>         (8704d9ca)
#   git clone --depth 1 --branch v1.0.0 \
#       https://github.com/sgr-io/s2-geometry-library-java.git <dir> (55871d6c)
# Both are Apache-2.0. Their roots come from the environment, else the defaults.
SPATIAL4J_PROJECT = "spatial4j 0.8"
S2_PROJECT = "s2-geometry-library-java 1.0.0"
SPATIAL4J_ROOT_ENV = "SPATIAL4J_ROOT"
DEFAULT_SPATIAL4J_ROOT = "/tmp/spatial4j-0.8"
S2_ROOT_ENV = "S2_GEOMETRY_ROOT"
DEFAULT_S2_ROOT = "/tmp/s2-geometry-1.0.0"
# The external Java projects whose modules the port covers, beyond Lucene:
# `(name, version, reference URL)`, each written as a `Project` by `--phase
# modules`.
EXTERNAL_PROJECTS = (
    (RT_PROJECT, "2.8.1",
     "https://github.com/randomizedtesting/randomizedtesting/tree/release/2.8.1"),
    (SPATIAL4J_PROJECT, "0.8",
     "https://github.com/locationtech/spatial4j/tree/spatial4j-0.8"),
    (S2_PROJECT, "1.0.0",
     "https://github.com/sgr-io/s2-geometry-library-java/tree/v1.0.0"),
)
# randomizedtesting 2.8.1 is the runner Lucene 10.5.0 builds its test framework
# on (`lucene/test-framework`'s descriptor: `requires transitive
# randomizedtesting.runner`). Obtain it with
#   git clone --branch release/2.8.1 --single-branch \
#       https://github.com/randomizedtesting/randomizedtesting.git <dir>
RT_ROOT_ENV = "RANDOMIZEDTESTING_ROOT"
DEFAULT_RT_ROOT = "/tmp/randomizedtesting281"

# One entry per module the port covers. `repo` names the clone a module's
# `sources` are relative to, and every `File.path` of the module is relative to
# that clone's root. `prefix` is the Java name prefix (or prefixes) of the
# module's types, and `packageRoot` the Java package (or packages) the crate's
# `src/` mirrors (`org.apache.lucene.codecs.simpletext.X` ->
# `crates/rucene-codecs/src/simpletext/`); either is a string or a tuple.
# `lucene/analysis/common` declares packages under two roots,
# `org.apache.lucene` (`analysis.*`, `collation.*`) and `org.tartarus.snowball`
# (the Snowball runtime and its generated stemmers), so both are tuples there:
# `org.apache.lucene.analysis.ar` -> `src/analysis/ar/`, and `org` as the
# second root keeps the vendor in the path (`org.tartarus.snowball` ->
# `src/tartarus/snowball/`), clear of `analysis.snowball`.
# `files`, when present, narrows a module to the listed source files (paths
# relative to the clone, as `File.path`): only they, and the module's JPMS
# descriptor, are surveyed, scoped, censused and given a crate destination;
# every other file of the module's `sources` is outside the port
# (`file_in_scope`). Sprint 13 takes from `lucene/sandbox` only
# `FuzzyLikeThisQuery.java`, whose top-level type and nested types are all
# declared in that one file, so a file allow-list is exactly the type set the
# user scoped and needs no type-level rule. Its package root is
# `org.apache.lucene.sandbox` (`org.apache.lucene.sandbox.queries` ->
# `crates/rucene-sandbox/src/queries/`), as `lucene/queries` and
# `lucene/queryparser` mirror the packages below `org.apache.lucene.queries` and
# `org.apache.lucene.queryparser` (`...queries.function.valuesource` ->
# `crates/rucene-queries/src/function/valuesource/`,
# `...queryparser.flexible.core.nodes` ->
# `crates/rucene-queryparser/src/flexible/core/nodes/`).
# `skeletons` is set on a module whose types are tracked through a Rust
# skeleton before they are ported (`SKELETON_OF`): `"report"` makes the audit
# report the classes still lacking one, `"enforce"` fails it on any.
# Sprint 17's modules mirror the packages below each module's own package root
# (`org.apache.lucene.search.uhighlight` -> `crates/rucene-highlighter/src/
# uhighlight/`, `org.apache.lucene.benchmark.byTask.feeds` ->
# `crates/rucene-benchmark/src/by_task/feeds/`, `org.locationtech.spatial4j.
# shape.impl` -> `crates/spatial4j/src/shape/impl/`). The two external libraries
# are narrowed by `files` to the transitive closure, over each library's main
# code, of the library types `lucene/spatial-extras/src/java` and
# `lucene/benchmark/src/java` import: a file's edges are its single-type and
# static imports, its wildcard imports filtered by the simple names it uses, the
# types of its own package whose simple names it uses, and fully qualified names
# in code (comments and literals blanked). spatial4j 0.8: 15 imported types, 41
# files; s2-geometry 1.0.0: 5 imported types, 15 files. spatial4j's list also
# holds, from `src/test/java`, the test helpers `lucene/spatial-extras/src/test`
# imports and their closure (`TestLog`, `shape.RandomizedShapeTest`,
# `shape.RectIntersectionTestHelper`), ported by the spatial-extras test task.
# The two clones share the source-tree spelling `src/main/java`; the `files`
# lists keep their modules apart (`module_of_file`).
# `jpms` is the module's JPMS name:
# randomizedtesting 2.8.1 ships no descriptor, so its name is the automatic one
# the JDK derives from `randomizedtesting-runner-2.8.1.jar` (version dropped,
# `-` -> `.`), the name `lucene/test-framework`'s descriptor requires.
PORT_MODULES = (
    {"name": "lucene/core", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/core/src/java", "lucene/core/src/java21"),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene",
     "crate": "rucene", "cratePath": ".", "jpms": "org.apache.lucene.core",
     "descriptor": "lucene/core/src/java/module-info.java",
     "scopeRule": "lucene-core-is-the-port-target", "classRule": "lucene-core-top-level",
     "role": "the port target"},
    {"name": "lucene/codecs", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/codecs/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.codecs",
     "crate": "rucene-codecs", "cratePath": "crates/rucene-codecs",
     "jpms": "org.apache.lucene.codecs",
     "descriptor": "lucene/codecs/src/java/module-info.java",
     "scopeRule": "sprint-9-ports-the-module", "classRule": "sprint-9-module-top-level",
     "role": "codecs outside core (SimpleText, bloom, memory, uniform split, ...); "
             "ported by sprint 9"},
    {"name": "lucene/test-framework", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/test-framework/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.tests",
     "crate": "rucene-test-framework", "cratePath": "crates/rucene-test-framework",
     "jpms": "org.apache.lucene.test_framework",
     "descriptor": "lucene/test-framework/src/java/module-info.java",
     "scopeRule": "sprint-9-ports-the-module", "classRule": "sprint-9-module-top-level",
     "role": "the test framework every Lucene test extends; ported by sprint 9"},
    {"name": "randomizedtesting/randomized-runner", "project": RT_PROJECT,
     "repo": "randomizedtesting", "sources": ("randomized-runner/src/main/java",),
     "prefix": "com.carrotsearch.randomizedtesting",
     "packageRoot": "com.carrotsearch.randomizedtesting",
     "crate": "randomizedtesting", "cratePath": "crates/randomizedtesting",
     "jpms": "randomizedtesting.runner", "descriptor": None,
     "scopeRule": "sprint-9-ports-the-module", "classRule": "sprint-9-module-top-level",
     "role": "the randomized test runner lucene/test-framework is built on; "
             "ported by sprint 9"},
    {"name": "lucene/analysis/common", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/analysis/common/src/java",),
     "prefix": ("org.apache.lucene", "org.tartarus.snowball"),
     "packageRoot": ("org.apache.lucene", "org"),
     "crate": "rucene-analysis-common", "cratePath": "crates/rucene-analysis-common",
     "jpms": "org.apache.lucene.analysis.common",
     "descriptor": "lucene/analysis/common/src/java/module-info.java",
     "scopeRule": "sprint-12-ports-the-module", "classRule": "sprint-12-module-top-level",
     "skeletons": "enforce",
     "role": "analyzers, tokenizers, token filters and char filters for many "
             "languages, with their SPI factories (the Snowball stemmers included); "
             "ported by sprint 12"},
    {"name": "lucene/queries", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/queries/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.queries",
     "crate": "rucene-queries", "cratePath": "crates/rucene-queries",
     "jpms": "org.apache.lucene.queries",
     "descriptor": "lucene/queries/src/java/module-info.java",
     "scopeRule": "sprint-13-ports-the-module", "classRule": "sprint-13-module-top-level",
     "skeletons": "report",
     "role": "query types outside core: function queries and value sources, "
             "intervals, spans, payload queries, more-like-this; ported by sprint 13"},
    {"name": "lucene/queryparser", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/queryparser/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.queryparser",
     "crate": "rucene-queryparser", "cratePath": "crates/rucene-queryparser",
     "jpms": "org.apache.lucene.queryparser",
     "descriptor": "lucene/queryparser/src/java/module-info.java",
     "scopeRule": "sprint-13-ports-the-module", "classRule": "sprint-13-module-top-level",
     "skeletons": "report",
     "role": "query parsers: classic, flexible (standard, precedence), simple, "
             "surround, complex-phrase, ext and the XML query parser; ported by sprint 13"},
    {"name": "lucene/sandbox", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/sandbox/src/java",),
     "files": ("lucene/sandbox/src/java/org/apache/lucene/sandbox/queries/"
               "FuzzyLikeThisQuery.java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.sandbox",
     "crate": "rucene-sandbox", "cratePath": "crates/rucene-sandbox",
     "jpms": "org.apache.lucene.sandbox",
     "descriptor": "lucene/sandbox/src/java/module-info.java",
     "scopeRule": "sprint-13-ports-listed-files",
     "classRule": "sprint-13-listed-file-top-level",
     "skeletons": "report",
     "role": "experimental code; only FuzzyLikeThisQuery is ported (sprint 13), "
             "for the XML query parser's FuzzyLikeThisQueryBuilder"},
    {"name": "lucene/memory", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/memory/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.index.memory",
     "crate": "rucene-memory", "cratePath": "crates/rucene-memory",
     "jpms": "org.apache.lucene.memory",
     "descriptor": "lucene/memory/src/java/module-info.java",
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "the single-document in-memory index (MemoryIndex); ported by sprint 17"},
    {"name": "lucene/facet", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/facet/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.facet",
     "crate": "rucene-facet", "cratePath": "crates/rucene-facet",
     "jpms": "org.apache.lucene.facet",
     "descriptor": "lucene/facet/src/java/module-info.java",
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "faceted search: taxonomy and sorted-set facets, ranges, range-on-range and facet sets; ported by sprint 17"},
    {"name": "lucene/highlighter", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/highlighter/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.search",
     "crate": "rucene-highlighter", "cratePath": "crates/rucene-highlighter",
     "jpms": "org.apache.lucene.highlighter",
     "descriptor": "lucene/highlighter/src/java/module-info.java",
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "highlighters: highlight, vectorhighlight, uhighlight and matchhighlight; ported by sprint 17"},
    {"name": "lucene/spatial3d", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/spatial3d/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.spatial3d",
     "crate": "rucene-spatial3d", "cratePath": "crates/rucene-spatial3d",
     "jpms": "org.apache.lucene.spatial3d",
     "descriptor": "lucene/spatial3d/src/java/module-info.java",
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "3D planar geometry on the sphere/ellipsoid and its point field and queries; ported by sprint 17"},
    {"name": "lucene/spatial-extras", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/spatial-extras/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.spatial",
     "crate": "rucene-spatial-extras", "cratePath": "crates/rucene-spatial-extras",
     "jpms": "org.apache.lucene.spatial_extras",
     "descriptor": "lucene/spatial-extras/src/java/module-info.java",
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "spatial strategies over spatial4j shapes (prefix trees, bbox, serialized, vector, composite); ported by sprint 17"},
    {"name": "lucene/spatial-test-fixtures", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/spatial-test-fixtures/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.spatial3d.tests",
     "crate": "rucene-spatial-test-fixtures", "cratePath": "crates/rucene-spatial-test-fixtures",
     # No descriptor: the automatic name the JDK derives from the jar
     # `lucene-spatial-test-fixtures-10.5.0.jar` (`gradle/globals.gradle`
     # `archivesName`; version dropped, `-` -> `.`).
     "jpms": "lucene.spatial.test.fixtures",
     "descriptor": None,
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "test fixtures for the spatial modules (random spatial3d shapes); ported by sprint 17"},
    {"name": "lucene/benchmark", "project": LUCENE, "repo": "lucene",
     "sources": ("lucene/benchmark/src/java",),
     "prefix": "org.apache.lucene", "packageRoot": "org.apache.lucene.benchmark",
     "crate": "rucene-benchmark", "cratePath": "crates/rucene-benchmark",
     "jpms": "org.apache.lucene.benchmark",
     "descriptor": "lucene/benchmark/src/java/module-info.java",
     "scopeRule": "sprint-17-ports-the-module", "classRule": "sprint-17-module-top-level",
     "skeletons": "enforce",
     "role": "the benchmarking harness (byTask algorithms, quality/TREC evaluation); ported by sprint 17"},
    {"name": "spatial4j/spatial4j", "project": SPATIAL4J_PROJECT,
     "repo": "spatial4j", "sources": ("src/main/java", "src/test/java"),
     "files": (
               "src/main/java/org/locationtech/spatial4j/context/SpatialContext.java",
               "src/main/java/org/locationtech/spatial4j/context/SpatialContextFactory.java",
               "src/main/java/org/locationtech/spatial4j/distance/AbstractDistanceCalculator.java",
               "src/main/java/org/locationtech/spatial4j/distance/CartesianDistCalc.java",
               "src/main/java/org/locationtech/spatial4j/distance/DistanceCalculator.java",
               "src/main/java/org/locationtech/spatial4j/distance/DistanceUtils.java",
               "src/main/java/org/locationtech/spatial4j/distance/GeodesicSphereDistCalc.java",
               "src/main/java/org/locationtech/spatial4j/exception/InvalidShapeException.java",
               "src/main/java/org/locationtech/spatial4j/io/BinaryCodec.java",
               "src/main/java/org/locationtech/spatial4j/io/GeoJSONReader.java",
               "src/main/java/org/locationtech/spatial4j/io/GeoJSONWriter.java",
               "src/main/java/org/locationtech/spatial4j/io/GeohashUtils.java",
               "src/main/java/org/locationtech/spatial4j/io/LegacyShapeReader.java",
               "src/main/java/org/locationtech/spatial4j/io/LegacyShapeWriter.java",
               "src/main/java/org/locationtech/spatial4j/io/OnePointsBuilder.java",
               "src/main/java/org/locationtech/spatial4j/io/ParseUtils.java",
               "src/main/java/org/locationtech/spatial4j/io/PolyshapeReader.java",
               "src/main/java/org/locationtech/spatial4j/io/PolyshapeWriter.java",
               "src/main/java/org/locationtech/spatial4j/io/ShapeIO.java",
               "src/main/java/org/locationtech/spatial4j/io/ShapeReader.java",
               "src/main/java/org/locationtech/spatial4j/io/ShapeWriter.java",
               "src/main/java/org/locationtech/spatial4j/io/SupportedFormats.java",
               "src/main/java/org/locationtech/spatial4j/io/WKTReader.java",
               "src/main/java/org/locationtech/spatial4j/io/WKTWriter.java",
               "src/main/java/org/locationtech/spatial4j/shape/BaseShape.java",
               "src/main/java/org/locationtech/spatial4j/shape/Circle.java",
               "src/main/java/org/locationtech/spatial4j/shape/Point.java",
               "src/main/java/org/locationtech/spatial4j/shape/Rectangle.java",
               "src/main/java/org/locationtech/spatial4j/shape/Shape.java",
               "src/main/java/org/locationtech/spatial4j/shape/ShapeCollection.java",
               "src/main/java/org/locationtech/spatial4j/shape/ShapeFactory.java",
               "src/main/java/org/locationtech/spatial4j/shape/SpatialRelation.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/BBoxCalculator.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/BufferedLine.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/BufferedLineString.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/CircleImpl.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/GeoCircle.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/InfBufLine.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/PointImpl.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/RectangleImpl.java",
               "src/main/java/org/locationtech/spatial4j/shape/impl/ShapeFactoryImpl.java",
               "src/test/java/org/locationtech/spatial4j/TestLog.java",
               "src/test/java/org/locationtech/spatial4j/shape/RandomizedShapeTest.java",
               "src/test/java/org/locationtech/spatial4j/shape/RectIntersectionTestHelper.java",
     ),
     "prefix": "org.locationtech.spatial4j", "packageRoot": "org.locationtech.spatial4j",
     "crate": "spatial4j", "cratePath": "crates/spatial4j",
     "jpms": "spatial4j", "descriptor": None,
     "scopeRule": "sprint-17-ports-listed-files",
     "classRule": "sprint-17-listed-file-top-level",
     "skeletons": "enforce",
     "role": "the spatial4j 0.8 shapes, context, distance and I/O that lucene/spatial-extras "
             "and lucene/benchmark use (their closure); ported by sprint 17"},
    {"name": "s2-geometry/s2-geometry-library-java", "project": S2_PROJECT,
     "repo": "s2-geometry", "sources": ("src/main/java",),
     "files": (
               "src/main/java/com/google/common/geometry/MutableInteger.java",
               "src/main/java/com/google/common/geometry/R1Interval.java",
               "src/main/java/com/google/common/geometry/R2Vector.java",
               "src/main/java/com/google/common/geometry/S1Angle.java",
               "src/main/java/com/google/common/geometry/S1Interval.java",
               "src/main/java/com/google/common/geometry/S2.java",
               "src/main/java/com/google/common/geometry/S2Cap.java",
               "src/main/java/com/google/common/geometry/S2Cell.java",
               "src/main/java/com/google/common/geometry/S2CellId.java",
               "src/main/java/com/google/common/geometry/S2EdgeUtil.java",
               "src/main/java/com/google/common/geometry/S2LatLng.java",
               "src/main/java/com/google/common/geometry/S2LatLngRect.java",
               "src/main/java/com/google/common/geometry/S2Point.java",
               "src/main/java/com/google/common/geometry/S2Projections.java",
               "src/main/java/com/google/common/geometry/S2Region.java",
     ),
     "prefix": "com.google.common.geometry", "packageRoot": "com.google.common.geometry",
     "crate": "s2-geometry", "cratePath": "crates/s2-geometry",
     "jpms": "s2.geometry.library.java", "descriptor": None,
     "scopeRule": "sprint-17-ports-listed-files",
     "classRule": "sprint-17-listed-file-top-level",
     "skeletons": "enforce",
     "role": "the S2 cell geometry that lucene/spatial-extras' S2 prefix tree uses "
             "(its closure); ported by sprint 17"},
)
BY_NAME = {m["name"]: m for m in PORT_MODULES}
# The modules beyond `lucene/core`: surveyed by `--phase structure`, given the
# mechanical nested `portState`, and the only ones a cycle-rule early port
# (`PORTS_EARLY`) may belong to.
ADDED_MODULES = tuple(m["name"] for m in PORT_MODULES if m["name"] != "lucene/core")


def as_tuple(v):
    """A `prefix` / `packageRoot` value as a tuple of strings."""
    return (v,) if isinstance(v, str) else tuple(v)


JAVA_PREFIXES = tuple(sorted({p for m in PORT_MODULES for p in as_tuple(m["prefix"])}))
SKELETON_MODULES = tuple(m["name"] for m in PORT_MODULES if m.get("skeletons"))

# The scope decisions, one per widening of the port, each with the modules it
# brought in. `supersedes` names an earlier `Decision` it replaces, if any.
DECISIONS = (
    {"name": "Sprint 9 ports lucene/test-framework, lucene/codecs and "
             "randomizedtesting 2.8.1",
     "on": "2026-11-03",
     "modules": ("lucene/codecs", "lucene/test-framework",
                 "randomizedtesting/randomized-runner"),
     "supersedes": "Rucene tests Lucene differentially instead of porting "
                   "lucene/test-framework"},
    {"name": "Sprint 12 ports lucene/analysis/common",
     "on": "2026-11-13",
     "modules": ("lucene/analysis/common",),
     "supersedes": None},
    {"name": "Sprint 13 ports lucene/queries and lucene/queryparser, with sandbox "
             "FuzzyLikeThisQuery",
     "on": "2026-11-14",
     "modules": ("lucene/queries", "lucene/queryparser", "lucene/sandbox"),
     "supersedes": None},
    {"name": "Sprint 17 ports lucene/benchmark with its dependency modules, and the "
             "spatial4j 0.8 and s2-geometry 1.0.0 subsets they use",
     "on": "2026-11-22",
     "modules": ("lucene/memory", "lucene/facet", "lucene/highlighter", "lucene/spatial3d",
                 "lucene/spatial-extras", "lucene/spatial-test-fixtures", "lucene/benchmark",
                 "spatial4j/spatial4j", "s2-geometry/s2-geometry-library-java"),
     "supersedes": None},
)
DECISION = DECISIONS[0]["name"]
SUPERSEDED = DECISIONS[0]["supersedes"]

# The per-module coverage: every in-scope top-level type, by module and status.
# `skeleton` is a Rust skeleton with no port yet; `unported` is every other
# `portState` but `ported` (`unported`, `candidate`).
COVERAGE_QUERY = (
    "MATCH (c:Class) WHERE c.portScope = 'in' "
    "RETURN c.module AS module, "
    "CASE WHEN c.portState = 'ported' THEN 'ported' "
    "WHEN c.portState = 'skeleton' THEN 'skeleton' "
    "WHEN c.portState = 'notApplicable' THEN 'notApplicable' ELSE 'unported' END AS status, "
    "count(c) AS classes ORDER BY module, status"
)
# The nested types per module and `portState` (`ported`, `absorbed`,
# `skeleton`, `unported`): the half of a module `COVERAGE_QUERY` leaves out.
NESTED_COVERAGE_QUERY = (
    "MATCH (c:Class) WHERE c.portScope = 'nested' "
    "RETURN c.module AS module, coalesce(c.portState, 'unstated') AS status, "
    "count(c) AS classes ORDER BY module, status"
)
# The member work still open per module: the types, top-level or nested, with
# an `absent` member verdict, and how many such members.
MEMBER_ABSENT_QUERY = (
    "MATCH (c:Class) WHERE c.portScope IN ['in', 'nested'] AND c.memberAbsent > 0 "
    "RETURN c.module AS module, count(c) AS classes, sum(c.memberAbsent) AS absent "
    "ORDER BY module"
)
# The members ported early under the cycle rule, by module.
EARLY_QUERY = (
    "MATCH (t:Task)-[e:PORTS_EARLY]->(m:Method)<-[:DECLARES]-(c:Class) "
    "RETURN c.module AS module, c.qualifiedName AS class, m.qualifiedName AS member, "
    "t.id AS task, e.ownerTask AS ownerTask, e.rustItem AS rustItem "
    "ORDER BY module, class, member"
)


def coverage_query(module=None):
    """`COVERAGE_QUERY`, narrowed to one module when `module` is given."""
    if module is None:
        return COVERAGE_QUERY
    return COVERAGE_QUERY.replace(
        "WHERE c.portScope = 'in' ", f"WHERE c.portScope = 'in' AND c.module = {esc(module)} ")


def nested_coverage_query(module=None):
    """`NESTED_COVERAGE_QUERY`, narrowed to one module when `module` is given."""
    if module is None:
        return NESTED_COVERAGE_QUERY
    return NESTED_COVERAGE_QUERY.replace(
        "WHERE c.portScope = 'nested' ", f"WHERE c.portScope = 'nested' AND c.module = {esc(module)} ")


def member_absent_query(module=None):
    """`MEMBER_ABSENT_QUERY`, narrowed to one module when `module` is given."""
    if module is None:
        return MEMBER_ABSENT_QUERY
    return MEMBER_ABSENT_QUERY.replace(
        "AND c.memberAbsent > 0 ", f"AND c.memberAbsent > 0 AND c.module = {esc(module)} ")


def fully_ported(top_rows, nested_rows, absent_rows):
    """`{module: verdict}` from the three coverage answers.

    A module is fully ported when every in-scope class is `ported` or
    `notApplicable`, every nested type `ported` or `absorbed`, and no type of it
    carries an `absent` member verdict. The verdict lists what stands in the way.
    """
    out = {}
    def entry(m):
        return out.setdefault(m, {"module": m, "topLevelOpen": 0, "nestedOpen": 0,
                                  "absentMembers": 0})
    for m, status, n in top_rows:
        e = entry(m)
        if status not in ("ported", "notApplicable"):
            e["topLevelOpen"] += n
    for m, status, n in nested_rows:
        e = entry(m)
        if status not in ("ported", "absorbed"):
            e["nestedOpen"] += n
    for m, _classes, absent in absent_rows:
        entry(m)["absentMembers"] += absent or 0
    for e in out.values():
        e["fullyPorted"] = not (e["topLevelOpen"] or e["nestedOpen"] or e["absentMembers"])
    return out


def early_query(module=None):
    if module is None:
        return EARLY_QUERY
    return EARLY_QUERY.replace("RETURN ", f"WHERE c.module = {esc(module)} RETURN ", 1)


def repo_roots(lucene_root=None, rt_root=None, spatial4j_root=None, s2_root=None):
    """`{repo: Path}` for the clones the modules live in.

    The two sprint-17 libraries take their roots from the arguments, else the
    environment (`SPATIAL4J_ROOT`, `S2_GEOMETRY_ROOT`), else the defaults, so
    every caller that passes only the Lucene and randomizedtesting roots reaches
    them unchanged.
    """
    return {"lucene": Path(lucene_root or DEFAULT_LUCENE_ROOT),
            "randomizedtesting": Path(rt_root or os.environ.get(RT_ROOT_ENV)
                                      or DEFAULT_RT_ROOT),
            "spatial4j": Path(spatial4j_root or os.environ.get(SPATIAL4J_ROOT_ENV)
                              or DEFAULT_SPATIAL4J_ROOT),
            "s2-geometry": Path(s2_root or os.environ.get(S2_ROOT_ENV) or DEFAULT_S2_ROOT)}


def source_dirs(roots, modules=PORT_MODULES):
    """`[(module, repo root, source dir)]` for every source tree of `modules`."""
    return [(m, roots[m["repo"]], roots[m["repo"]] / s)
            for m in modules for s in m["sources"]]


def file_in_scope(module, path):
    """Whether the Java file `path` (relative to its clone) is in `module`'s scope.

    A file of the module's `sources` is in scope, unless the module lists
    `files`: then only those and the module's JPMS descriptor are (the
    descriptor declares no type, so it never enters a type count).
    """
    m = BY_NAME[module]
    if not path or not any(path.startswith(s + "/") for s in m["sources"]):
        return False
    files = m.get("files")
    return files is None or path in files or path == m["descriptor"]


def module_of_file(path):
    """The port module a Java `File.path` belongs to, or `None`.

    Lucene paths start `lucene/<module>/src/...` and randomizedtesting paths
    `randomized-runner/src/main/java/...`, so the source prefixes never overlap.
    A file of a module scoped to listed `files` that is not listed belongs to
    no port module.
    """
    if not path:
        return None
    for m in PORT_MODULES:
        if file_in_scope(m["name"], path):
            return m["name"]
    return None


def package_root(module, package):
    """The longest `packageRoot` of `module` that `package` lies under, or `None`."""
    hits = [r for r in as_tuple(BY_NAME[module]["packageRoot"])
            if package == r or package.startswith(r + ".")]
    return max(hits, key=len) if hits else None


def snake_case(segment):
    """A Java package segment as the crate names its module: `complexPhrase`
    -> `complex_phrase` (the crate's `src/complex_phrase/`)."""
    s = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", segment)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s).lower()


def crate_destination(module, package):
    """The crate directory a type of `package` is ported into (`src/` mirrors
    the package below the module's `packageRoot`, each segment snake-cased as
    a Rust module name: `queryparser.complexPhrase` is `src/complex_phrase/`)."""
    m = BY_NAME[module]
    root = package_root(module, package)
    rel = package[len(root):].lstrip(".") if root else package
    base = "src" if m["cratePath"] == "." else f"{m['cratePath']}/src"
    segs = [snake_case(p) for p in rel.split(".")] if rel else []
    return "/".join([base] + segs) + "/"


REQUIRES_RE = re.compile(r"(?<![\w.])requires\s+(?:(?:transitive|static)\s+)*([\w.]+)\s*;")


def descriptor_requires(text):
    """The module names a JPMS descriptor `requires`, comments ignored."""
    from javatext import strip_java
    return REQUIRES_RE.findall(strip_java(text))


def module_dependencies(roots):
    """`[(from module, to module, descriptor path)]` between port modules."""
    by_jpms = {m["jpms"]: m["name"] for m in PORT_MODULES}
    out = []
    for m in PORT_MODULES:
        if not m["descriptor"]:
            continue
        text = (roots[m["repo"]] / m["descriptor"]).read_text(encoding="utf-8")
        for req in descriptor_requires(text):
            if req in by_jpms and by_jpms[req] != m["name"]:
                out.append((m["name"], by_jpms[req], m["descriptor"]))
    return out


# ---------------------------------------------------------------------------
# Survey of one module's source tree, by the extractors lucene/core uses
# ---------------------------------------------------------------------------


@contextmanager
def lucene_roots(base, source):
    """Point `extract_lucene_kg` and `enrich_lucene_kg` at one source tree.

    Both were written for `lucene/core` and read module-level paths; the tree
    is swapped in for the duration and restored after, so their extraction runs
    unchanged on every module (one extractor, not a parallel one).
    """
    import enrich_lucene_kg as en
    import extract_lucene_kg as el
    saved = (el.ROOT, el.LUCENE_ROOT, el.CORE_PREFIX,
             en.BASE_ROOT, en.CORE_ROOT_JAVA, en.CORE_ROOT_JAVA21)
    el.ROOT, el.LUCENE_ROOT = Path(base) / source, Path(base)
    en.BASE_ROOT, en.CORE_ROOT_JAVA = Path(base), Path(base) / source
    en.CORE_ROOT_JAVA21 = Path(base) / source / "__no_java21__"
    try:
        yield el, en
    finally:
        (el.ROOT, el.LUCENE_ROOT, el.CORE_PREFIX,
         en.BASE_ROOT, en.CORE_ROOT_JAVA, en.CORE_ROOT_JAVA21) = saved


def survey_module(module, roots):
    """Packages, files, types, nested types and members of one module.

    A module scoped to listed `files` keeps only what those files declare:
    their packages, top-level and nested types, and the members of those types.
    """
    m = BY_NAME[module]
    data = {"packages": set(), "files": [], "types": [], "nested": [],
            "members": {"methods": [], "constructors": [], "fields": [],
                        "enumConstants": []}}
    for source in m["sources"]:
        base = roots[m["repo"]]
        if not (base / source).is_dir():
            raise SystemExit(f"no sources for {module} under {base / source}")
        with lucene_roots(base, source) as (el, en):
            el.CORE_PREFIX = as_tuple(m["prefix"])
            d = el.discover()
            data["packages"].update(p for p in d["packages"] if package_root(module, p))
            data["files"].extend(d["files"])
            data["types"].extend(d["types"])
            data["nested"].extend(en.extract_inner_classes())
            for k, v in en.extract_members().items():
                data["members"][k].extend(v)
    if m.get("files"):
        keep = lambda path: file_in_scope(module, path)  # noqa: E731
        data["files"] = [f for f in data["files"] if keep(f["path"])]
        data["types"] = [t for t in data["types"] if keep(t["file"])]
        data["nested"] = [n for n in data["nested"] if keep(n["file"])]
        owners = ({t["qualified"] for t in data["types"]}
                  | {n["qualifiedName"] for n in data["nested"]})
        data["members"] = {k: [x for x in v if x["parentQualifiedName"] in owners]
                           for k, v in data["members"].items()}
        data["packages"] = {f["package"] for f in data["files"]
                            if package_root(module, f["package"])}
    data["packages"] = sorted(data["packages"])
    return data


# ---------------------------------------------------------------------------
# Phases
# ---------------------------------------------------------------------------


def phase_modules(roadmap, roots, commit, date):
    print("phase: modules", file=sys.stderr)
    stamp = {"commit": commit, "date": date}
    for project, version, url in EXTERNAL_PROJECTS:
        rmp("create", roadmap, f"MERGE (p:Project {{name: {esc(project)}}})")
        rmp("update", roadmap,
            f"MATCH (p:Project {{name: {esc(project)}}}) SET p.language = 'Java', "
            f"p.version = {esc(version)}, p.referenceUrl = {esc(url)}, "
            f"p.gitCommit = {esc(commit)}, p.gitDate = {esc(date)}")
    rows = []
    for m in PORT_MODULES:
        src = roots[m["repo"]] / m["sources"][0]
        # The first source tree only, as `lucene_modules_kg.py` counts every
        # module (`lucene/*/src/java`), so the figures stay comparable.
        java_files = sum(1 for _ in src.rglob("*.java"))
        if not src.is_dir():
            raise SystemExit(f"no sources for {m['name']} under {src}")
        rows.append({"name": m["name"], "project": m["project"],
                     "shortName": m["name"].split("/", 1)[1],
                     # The module's directory in its clone: `.` when the clone
                     # is the module (spatial4j, s2-geometry: `src/main/java`).
                     "path": (m["sources"][0].rsplit("/src/", 1)[0]
                              if "/src/" in m["sources"][0] else "."),
                     "javaFiles": java_files, "scopeRule": m["scopeRule"],
                     "role": m["role"], "crate": m["crate"], "cratePath": m["cratePath"],
                     "jpms": m["jpms"],
                     # As `Class.nestedHostFiles`: one string, ", "-separated; null
                     # (no property) on a module ported whole.
                     "scopeFiles": ", ".join(m["files"]) if m.get("files") else None,
                     **stamp})
    unwind(roadmap, "create", [{"name": r["name"]} for r in rows],
           "MERGE (m:Module {name: row.name})", "port Module nodes")
    unwind(roadmap, "update", rows,
           "MATCH (m:Module {name: row.name}) SET m.kind = 'maven-module', "
           "m.project = row.project, m.shortName = row.shortName, m.path = row.path, "
           "m.javaFiles = row.javaFiles, m.inScope = true, m.scopeRule = row.scopeRule, "
           "m.role = row.role, m.crate = row.crate, m.cratePath = row.cratePath, "
           "m.jpmsName = row.jpms, m.scopeFiles = row.scopeFiles, "
           "m.gitCommit = row.commit, m.gitDate = row.date",
           "port Module properties")
    unwind(roadmap, "create", [{"name": r["name"], "project": r["project"]} for r in rows],
           "MATCH (p:Project {name: row.project}) WITH p, row "
           "MATCH (m:Module {name: row.name}) MERGE (p)-[:CONTAINS]->(m)",
           "Project CONTAINS port Module")
    unwind(roadmap, "update", [{"name": r["name"], "project": r["project"], **stamp}
                               for r in rows],
           "MATCH (:Project {name: row.project})-[e:CONTAINS]->(:Module {name: row.name}) "
           "SET e.gitCommit = row.commit, e.gitDate = row.date",
           "Project CONTAINS stamped")

    deps = [{"a": a, "b": b, "via": via, **stamp} for a, b, via in module_dependencies(roots)]
    # A full replace of the descriptor-derived edges, so a requirement the
    # descriptors drop does not survive.
    rmp("delete", roadmap,
        "MATCH (:Module)-[e:DEPENDS_ON]->(:Module) WHERE e.evidence = 'module-info' DELETE e")
    unwind(roadmap, "create", deps,
           "MATCH (a:Module {name: row.a}) MATCH (b:Module {name: row.b}) "
           "MERGE (a)-[:DEPENDS_ON]->(b)", "Module DEPENDS_ON Module")
    unwind(roadmap, "update", deps,
           "MATCH (a:Module {name: row.a})-[e:DEPENDS_ON]->(b:Module {name: row.b}) "
           "SET e.evidence = 'module-info', e.declaredAt = row.via, "
           "e.gitCommit = row.commit, e.gitDate = row.date", "Module DEPENDS_ON evidence")
    for a, b, _via in module_dependencies(roots):
        print(f"  {a} requires {b}", file=sys.stderr)
    for d in DECISIONS:
        write_decision(roadmap, d, rows, deps, commit, date)


def decision_text(decision):
    """`(summary, rationale, alternatives)` of one scope `Decision`."""
    if decision["name"] == DECISIONS[3]["name"]:
        return (
            "On 2026-11-22 the user decided that sprint 17 ('Port the benchmark module "
            "and its dependency modules') ports Lucene 10.5.0's lucene/benchmark and every "
            "module it depends on, whole and faithfully: lucene/memory, lucene/facet, "
            "lucene/highlighter, lucene/spatial3d, lucene/spatial-extras and "
            "lucene/spatial-test-fixtures, as the workspace crates rucene-benchmark, "
            "rucene-memory, rucene-facet, rucene-highlighter, rucene-spatial3d, "
            "rucene-spatial-extras and rucene-spatial-test-fixtures (crates/<crate>); and "
            "two external Java libraries as their own crates and Projects, as "
            "randomizedtesting 2.8.1 was: spatial4j 0.8 (crate spatial4j) and "
            "s2-geometry-library-java 1.0.0 (crate s2-geometry), each narrowed by "
            "Module.scopeFiles to the transitive closure, over the library's main code, of "
            "the library types lucene/spatial-extras and lucene/benchmark main code import "
            "(spatial4j: 41 files, plus the 3 test helpers lucene/spatial-extras' tests "
            "import; s2: 15 files). Rust module paths mirror the Java packages below each "
            "module's package root. The nine modules are inScope=true; the top-level types "
            "of the seven Lucene modules are portScope='in' (portScopeRule "
            "'sprint-17-module-top-level'), those of the listed library files portScope="
            "'in' (portScopeRule 'sprint-17-listed-file-top-level'), nested types "
            "portScope='nested'. All nine are tracked through Rust skeletons "
            "(skeletons: 'enforce'). commons-compress, icu4j and nekohtml, which "
            "lucene/benchmark also uses, are replaced by Rust crates (rmp #611), not ported.",
            "The user's decisions for sprint 17 (rmp sprint 17 comments, 2026-11-22): the "
            "dependency modules are ported in full, not only the parts lucene/benchmark "
            "uses; spatial4j and s2-geometry are ported (the subset used), the other three "
            "libraries replaced by crates. CLAUDE.md 6.1 requires the graph to answer what "
            "of the port's scope is ported and what is missing, against an explicit "
            "denominator, so the modules the work now covers enter the graph's scope, and "
            "the library narrowing is stated in the graph as a file list computed from the "
            "sources rather than implied.",
            "Porting only the parts of the dependency modules lucene/benchmark uses, as "
            "the sandbox precedent did (rejected by the user). Porting lucene/benchmark "
            "alone (rejected by the user: 13 of its files would stay blocked). Porting all "
            "five external libraries, or replacing all five by crates (both rejected by the "
            "user). Taking spatial4j and s2-geometry whole (rejected: the user scoped them "
            "to the subset used; types no Lucene code reaches would inflate the "
            "denominator). Folding the library types into rucene-spatial-extras (rejected: "
            "the crate layout mirrors the Java modules and projects). Out of scope by the "
            "user's decision: lucene/analysis/icu (benchmark's moduleRuntimeOnly) and jts "
            "(no import in spatial-extras' main sources).",
        )
    if decision["name"] == DECISIONS[2]["name"]:
        return (
            "On 2026-11-14 the user decided that sprint 13 ('Port Query Modules') ports "
            "Lucene 10.5.0's lucene/queries and lucene/queryparser completely and "
            "faithfully, as the Cargo workspace crates rucene-queries "
            "(crates/rucene-queries) and rucene-queryparser (crates/rucene-queryparser), "
            "and of lucene/sandbox only org.apache.lucene.sandbox.queries."
            "FuzzyLikeThisQuery and its nested types, as the crate rucene-sandbox "
            "(crates/rucene-sandbox). Rust module paths mirror the Java packages below "
            "org.apache.lucene.queries, org.apache.lucene.queryparser and "
            "org.apache.lucene.sandbox. The three modules are inScope=true; the top-level "
            "types of lucene/queries and lucene/queryparser are portScope='in' "
            "(portScopeRule 'sprint-13-module-top-level'). lucene/sandbox is scoped to "
            "the source files its Module.scopeFiles lists, FuzzyLikeThisQuery.java alone: "
            "its top-level type is portScope='in' (portScopeRule "
            "'sprint-13-listed-file-top-level'), its nested types are portScope='nested', "
            "and no other sandbox type is surveyed, scoped or counted. The three modules "
            "are tracked through Rust skeletons (skeletons: 'report' until the skeletons "
            "land).",
            "The user's decisions for sprint 13 (rmp sprint 13 comments, 2026-11-14): "
            "both query modules, whole; from lucene/sandbox only FuzzyLikeThisQuery, "
            "because lucene/queryparser's xml.builders.FuzzyLikeThisQueryBuilder needs it "
            "and it depends only on lucene/core. CLAUDE.md 6.1 requires the graph to "
            "answer what of the port's scope is ported and what is missing, against an "
            "explicit denominator, so the modules the work now covers enter the graph's "
            "scope, and the sandbox narrowing is stated in the graph rather than implied.",
            "Bringing lucene/sandbox in whole (rejected: the user scoped it to one type; "
            "every other sandbox type would inflate the denominator with work nobody "
            "asked for). Leaving FuzzyLikeThisQuery outside the port modules (rejected: "
            "the queryparser port needs it, and its coverage would have no denominator). "
            "A type-level allow-list (rejected: FuzzyLikeThisQuery and its nested types "
            "are exactly what FuzzyLikeThisQuery.java declares, so the file is the "
            "narrowest unit that every survey, scope and census tool already keys on). "
            "Folding FuzzyLikeThisQuery into the queryparser crate (rejected: the crate "
            "layout mirrors Lucene's modules).",
        )
    if decision["name"] == DECISIONS[1]["name"]:
        return (
            "On 2026-11-13 the user decided that sprint 12 ('Port Analysis Common "
            "Module') ports Lucene 10.5.0's lucene/analysis/common completely and "
            "100% faithfully, as the Cargo workspace crate rucene-analysis-common at "
            "crates/rucene-analysis-common; Rust module paths mirror the Java "
            "packages (org.apache.lucene.analysis.*, org.apache.lucene.collation.*, "
            "org.tartarus.snowball.*). The module is inScope=true; its top-level "
            "types are portScope='in' (portScopeRule 'sprint-12-module-top-level'), "
            "every Class carries its module, and portState is derived from the same "
            "declared PORTS evidence as every other module's, read from the module's "
            "crate. Before a type is ported it is tracked through its Rust skeleton: "
            "an item documented 'Skeleton of `<Java type>`; ported by rmp task #N.' "
            "is recorded as SKELETON_OF the Class, the task as Task-[:PLANS_PORT]->"
            "Class, and the Class is portState='skeleton' until a PORTS claim makes "
            "it 'ported'.",
            "The user's decision for sprint 12 (2026-11-13). CLAUDE.md 6.1 requires "
            "the graph to answer what of the port's scope is ported and what is "
            "missing, so the module the work now covers has to be in the graph's "
            "scope; the skeleton relation makes the state of each type exact "
            "between its skeleton and its port, as the user asked.",
            "Keeping the module outside the port modules (rejected: the sprint ports "
            "it, and coverage would have no denominator for it). Counting a skeleton "
            "as a port (rejected: a skeleton carries no behaviour; only a PORTS "
            "claim makes a type ported). A separate portScope value for the module "
            "(rejected: its types are port targets exactly as lucene/core's are; "
            "Class.module tells the modules apart).",
        )
    return (
        "On 2026-11-03 the user decided that sprint 9 ('Port Tests Framework') ports "
        "Lucene 10.5.0's lucene/test-framework and lucene/codecs and the runner they "
        "are built on, randomizedtesting 2.8.1 (randomized-runner), completely, as a "
        "Cargo workspace with one crate per module: crates/rucene-test-framework, "
        "crates/rucene-codecs, crates/randomizedtesting; Rust module paths mirror the "
        "Java packages. The three modules are inScope=true; their top-level types are "
        "portScope='in' (portScopeRule 'sprint-9-module-top-level'), every Class "
        "carries its module, and portState is derived from the same declared PORTS "
        "evidence as lucene/core's, read from each module's crate. A member a task "
        "ports early under the sprint's cycle rule is recorded as Task-[:PORTS_EARLY]->"
        "Method, and a member-level claim alone does not make a sprint-9 type ported.",
        "The user's decision for sprint 9 (rmp sprint 9, tasks #323-#360). CLAUDE.md "
        "6.1 requires the graph to answer what of the port's scope is ported and what "
        "is missing, so the scope the work now covers has to be the graph's scope. It "
        "supersedes the earlier observation that Rucene tests Lucene differentially "
        "instead of porting lucene/test-framework, which recorded the code as it stood "
        "and left porting the framework as an open question for the maintainer.",
        "Keeping the framework as TestClass nodes only (rejected: TestClass carries no "
        "port state, so 'what of the framework is ported' had no answer). A separate "
        "portScope value for the sprint-9 modules (rejected: they are port targets "
        "exactly as lucene/core is; Class.module already tells the modules apart). "
        "Deleting the superseded decision (rejected: it is history; it is kept, marked "
        "status='superseded', and linked by SUPERSEDES).",
    )


def write_decision(roadmap, decision, rows, deps, commit, date):
    name = decision["name"]
    files = {r["name"]: r["javaFiles"] for r in rows if r["name"] in decision["modules"]}
    scoped = {k: BY_NAME[k]["files"] for k in files if BY_NAME[k].get("files")}
    summary, rationale, alternatives = decision_text(decision)
    evidence = (
        "Java files at the surveyed tags: " +
        ", ".join(f"{k} {v}" + (f" (in scope: {', '.join(p.rsplit('/', 1)[1] for p in scoped[k])})"
                                if k in scoped else "")
                  for k, v in files.items()) +
        ". JPMS requirements of these modules on port modules: " +
        ", ".join(f"{d['a']} -> {d['b']}" for d in deps if d["a"] in decision["modules"]) +
        "."
    )
    rmp("create", roadmap, f"MERGE (d:Decision {{name: {esc(name)}}})")
    rmp("update", roadmap,
        f"MATCH (d:Decision {{name: {esc(name)}}}) SET d.kind = 'principle', "
        f"d.summary = {esc(summary)}, d.rationale = {esc(rationale)}, "
        f"d.alternatives = {esc(alternatives)}, d.evidence = {esc(evidence)}, "
        f"d.status = 'current', d.gitCommit = {esc(commit)}, d.gitDate = {esc(date)}")
    link_decision(roadmap, name, decision["modules"], commit, date)
    old_name = decision["supersedes"]
    if not old_name:
        return
    old = read(roadmap, f"MATCH (d:Decision {{name: {esc(old_name)}}}) RETURN count(d)")
    if old and old[0][0]:
        rmp("create", roadmap,
            f"MATCH (n:Decision {{name: {esc(name)}}}) "
            f"MATCH (o:Decision {{name: {esc(old_name)}}}) MERGE (n)-[:SUPERSEDES]->(o)")
        rmp("update", roadmap,
            f"MATCH (n:Decision {{name: {esc(name)}}})-[e:SUPERSEDES]->"
            f"(o:Decision {{name: {esc(old_name)}}}) "
            f"SET e.on = {esc(decision['on'])}, e.gitCommit = {esc(commit)}, "
            f"e.gitDate = {esc(date)}, o.status = 'superseded'")
        print(f"  '{old_name}' marked superseded", file=sys.stderr)


def link_decision(roadmap, name, modules, commit, date):
    """Scope `Decision` -[:CONCERNS]-> each port `Module` whose scope it sets.

    Shared with `port_coverage_kg.phase_decision`. An edge to a module no
    longer in `modules` is dropped. A listed module left without the edge (no
    `Module` node of that name) stops the run: an isolated scope `Decision` is
    a defect, not a state the sync accepts.
    """
    decision = f"MATCH (d:Decision {{name: {esc(name)}}}) "
    listed = "[" + ", ".join(esc(m) for m in modules) + "]"
    rmp("delete", roadmap,
        f"{decision}MATCH (d)-[e:CONCERNS]->(m:Module) WHERE NOT m.name IN {listed} DELETE e")
    unwind(roadmap, "create", [{"m": m} for m in modules],
           f"{decision}MATCH (m:Module {{name: row.m}}) MERGE (d)-[:CONCERNS]->(m)",
           "Decision CONCERNS Module")
    rmp("update", roadmap,
        f"{decision}MATCH (d)-[e:CONCERNS]->(m:Module) WHERE m.name IN {listed} "
        f"SET e.gitCommit = {esc(commit)}, e.gitDate = {esc(date)}")
    linked = {r[0] for r in read(roadmap,
                                 f"{decision}MATCH (d)-[:CONCERNS]->(m:Module) RETURN m.name")}
    missing = sorted(set(modules) - linked)
    if missing:
        sys.exit(f"Decision {name!r}: no CONCERNS edge to Module {', '.join(missing)}")


def phase_structure(roadmap, roots, commit, date):
    print("phase: structure", file=sys.stderr)
    import lucene_structure_kg as ls
    graph_classes = {r[0] for r in read(roadmap, "MATCH (c:Class) RETURN c.qualifiedName")}
    for module in ADDED_MODULES:
        m = BY_NAME[module]
        data = survey_module(module, roots)
        print(f"  {module}: packages={len(data['packages'])} files={len(data['files'])} "
              f"types={len(data['types'])} nested={len(data['nested'])} members="
              f"{sum(len(v) for v in data['members'].values())}", file=sys.stderr)
        st = {"commit": commit, "date": date}
        pkgs = [{"name": p, "short": p.rsplit(".", 1)[-1],
                 "path": f"{m['sources'][0]}/{p.replace('.', '/')}", "module": module, **st}
                for p in data["packages"]]
        unwind(roadmap, "create", [{"name": p["name"]} for p in pkgs],
               "MERGE (p:Package {name: row.name})", "Package nodes")
        unwind(roadmap, "update", pkgs,
               "MATCH (p:Package {name: row.name}) SET p.shortName = row.short, "
               "p.path = row.path, p.module = row.module, "
               "p.gitCommit = row.commit, p.gitDate = row.date", "Package properties")
        unwind(roadmap, "create", [{"name": p["name"], "module": module} for p in pkgs],
               "MATCH (m:Module {name: row.module}) MATCH (p:Package {name: row.name}) "
               "MERGE (m)-[:CONTAINS]->(p)", "Module CONTAINS Package")
        unwind(roadmap, "update", [{"name": p["name"], "module": module, **st} for p in pkgs],
               "MATCH (:Module {name: row.module})-[e:CONTAINS]->(:Package {name: row.name}) "
               "SET e.gitCommit = row.commit, e.gitDate = row.date", "Module CONTAINS stamped")

        files = [{"path": f["path"], "name": f["name"], "pkg": f["package"],
                  "module": module, **st} for f in data["files"]]
        unwind(roadmap, "create", [{"path": f["path"]} for f in files],
               "MERGE (f:File {path: row.path})", "File nodes")
        unwind(roadmap, "update", files,
               "MATCH (f:File {path: row.path}) SET f.name = row.name, f.kind = 'source', "
               "f.language = 'Java', f.package = row.pkg, f.module = row.module, "
               "f.gitCommit = row.commit, f.gitDate = row.date", "File properties")
        unwind(roadmap, "create", [{"path": f["path"], "pkg": f["pkg"]} for f in files],
               "MATCH (p:Package {name: row.pkg}) MATCH (f:File {path: row.path}) "
               "MERGE (p)-[:CONTAINS]->(f)", "Package CONTAINS File")
        unwind(roadmap, "update", [{"path": f["path"], "pkg": f["pkg"], **st} for f in files],
               "MATCH (:Package {name: row.pkg})-[e:CONTAINS]->(:File {path: row.path}) "
               "SET e.gitCommit = row.commit, e.gitDate = row.date", "Package CONTAINS stamped")

        # The JPMS descriptor declares no package, so `discover` skips it; it
        # is the module's own file, and the evidence of `Module` `DEPENDS_ON`.
        if m["descriptor"]:
            desc = [{"path": m["descriptor"], "module": module, **st}]
            unwind(roadmap, "create", [{"path": m["descriptor"]}],
                   "MERGE (f:File {path: row.path})", "descriptor File node")
            unwind(roadmap, "update", desc,
                   "MATCH (f:File {path: row.path}) SET f.name = 'module-info.java', "
                   "f.kind = 'module-descriptor', f.language = 'Java', f.module = row.module, "
                   "f.gitCommit = row.commit, f.gitDate = row.date", "descriptor File properties")
            unwind(roadmap, "create", [{"path": m["descriptor"], "module": module}],
                   "MATCH (m:Module {name: row.module}) MATCH (f:File {path: row.path}) "
                   "MERGE (m)-[:CONTAINS]->(f)", "Module CONTAINS descriptor")
            unwind(roadmap, "update", desc,
                   "MATCH (:Module {name: row.module})-[e:CONTAINS]->(:File {path: row.path}) "
                   "SET e.gitCommit = row.commit, e.gitDate = row.date", "descriptor stamped")
            data["files"].append({"path": m["descriptor"]})

        types = [{"qn": t["qualified"], "name": t["name"], "kind": t["kind"],
                  "file": t["file"], "pkg": t["package"], "module": module, **st}
                 for t in data["types"]]
        unwind(roadmap, "create", [{"qn": t["qn"]} for t in types],
               "MERGE (c:Class {qualifiedName: row.qn})", "top-level Class nodes")
        unwind(roadmap, "update", types,
               "MATCH (c:Class {qualifiedName: row.qn}) SET c.name = row.name, "
               "c.kind = row.kind, c.file = row.file, c.package = row.pkg, "
               "c.module = row.module, c.gitCommit = row.commit, c.gitDate = row.date",
               "top-level Class properties")
        unwind(roadmap, "create", [{"qn": t["qn"], "file": t["file"]} for t in types],
               "MATCH (f:File {path: row.file}) MATCH (c:Class {qualifiedName: row.qn}) "
               "MERGE (f)-[:DECLARES]->(c)", "File DECLARES Class")
        unwind(roadmap, "update", [{"qn": t["qn"], "file": t["file"], **st} for t in types],
               "MATCH (:File {path: row.file})-[e:DECLARES]->(c:Class {qualifiedName: row.qn}) "
               "SET e.gitCommit = row.commit, e.gitDate = row.date", "File DECLARES stamped")

        # Nested types and members, by `lucene_structure_kg`'s own loaders, fed
        # the survey's extraction of this module's tree (narrowed to a scoped
        # module's listed files). Its prune is never asked for: it compares
        # against the whole graph, and would read lucene/core's nested types as
        # stale.
        ls.load_nested(roadmap, commit, date, prune=False, inners=data["nested"])
        ls.load_members(roadmap, commit, date, members=data["members"])
        unwind(roadmap, "update", [{"qn": n["qualifiedName"], "module": module}
                                   for n in data["nested"]],
               "MATCH (c:Class {qualifiedName: row.qn}) SET c.module = row.module",
               "nested Class module")

        known = graph_classes | {t["qn"] for t in types} | {
            n["qualifiedName"] for n in data["nested"]}
        sup = []
        for t in data["types"]:
            for kind, targets in (("EXTENDS", [t["extends"]] if t["extends"] else []),
                                  ("IMPLEMENTS", t["implements"])):
                for b in targets:
                    if b in known and b != t["qualified"]:
                        sup.append({"kind": kind, "a": t["qualified"], "b": b, **st})
        for kind in ("EXTENDS", "IMPLEMENTS"):
            sel = [r for r in sup if r["kind"] == kind]
            unwind(roadmap, "create", [{"a": r["a"], "b": r["b"]} for r in sel],
                   "MATCH (a:Class {qualifiedName: row.a}) MATCH (b:Class {qualifiedName: row.b}) "
                   f"MERGE (a)-[:{kind}]->(b)", f"Class {kind} Class")
            unwind(roadmap, "update", sel,
                   f"MATCH (a:Class {{qualifiedName: row.a}})-[e:{kind}]->"
                   "(b:Class {qualifiedName: row.b}) "
                   "SET e.gitCommit = row.commit, e.gitDate = row.date", f"{kind} stamped")
        prune_module(roadmap, module, data, types)

    # The framework's TestClass nodes stay -- lucene/core's tests reach
    # LuceneTestCase through their EXTENDS chain -- and each points at the Class
    # node that now carries its port state.
    pairs = [{"qn": r[0], "commit": commit, "date": date} for r in read(
        roadmap, "MATCH (t:TestClass) WHERE t.module = 'lucene/test-framework' "
        "RETURN t.qualifiedName")]
    unwind(roadmap, "create", [{"qn": p["qn"]} for p in pairs],
           "MATCH (t:TestClass {qualifiedName: row.qn}) MATCH (c:Class {qualifiedName: row.qn}) "
           "MERGE (t)-[:PORT_UNIT]->(c)", "TestClass PORT_UNIT Class")
    unwind(roadmap, "update", pairs,
           "MATCH (:TestClass {qualifiedName: row.qn})-[e:PORT_UNIT]->"
           "(:Class {qualifiedName: row.qn}) SET e.gitCommit = row.commit, e.gitDate = row.date",
           "PORT_UNIT stamped")


def prune_module(roadmap, module, data, types):
    """Delete what the module's tree no longer declares; never a port's target."""
    if not data["files"]:
        return  # an empty survey proves nothing
    files = {f["path"] for f in data["files"]}
    classes = {t["qn"] for t in types} | {n["qualifiedName"] for n in data["nested"]}
    members = {x["qualifiedName"] for v in data["members"].values() for x in v}
    m = esc(module)
    protected = {r[0] for r in read(
        roadmap, f"MATCH ()-[:PORTS]->(c:Class) WHERE c.module = {m} RETURN c.qualifiedName")}
    stale_c = [{"qn": r[0]} for r in read(
        roadmap, f"MATCH (c:Class) WHERE c.module = {m} RETURN c.qualifiedName")
        if r[0] not in classes and r[0] not in protected]
    early = {r[0] for r in read(roadmap, "MATCH (:Task)-[:PORTS_EARLY]->(x:Method) "
                                         "RETURN x.qualifiedName")}
    stale_m = [{"qn": r[0]} for r in read(
        roadmap, f"MATCH (c:Class)-[:DECLARES]->(x:Method) WHERE c.module = {m} "
        "RETURN x.qualifiedName") if r[0] not in members and r[0] not in early]
    stale_f = [{"p": r[0]} for r in read(
        roadmap, f"MATCH (f:File) WHERE f.module = {m} RETURN f.path") if r[0] not in files]
    unwind(roadmap, "delete", stale_m,
           "MATCH (x:Method {qualifiedName: row.qn}) DETACH DELETE x", "stale Method pruned")
    unwind(roadmap, "delete", stale_c,
           "MATCH (c:Class {qualifiedName: row.qn}) DETACH DELETE c", "stale Class pruned")
    unwind(roadmap, "delete", stale_f,
           "MATCH (f:File {path: row.p}) DETACH DELETE f", "stale File pruned")


def phase_coverage(roadmap):
    print("phase: coverage", file=sys.stderr)
    for module, status, n in read(roadmap, COVERAGE_QUERY):
        print(f"  {module:<38} {status:<8} {n}", file=sys.stderr)


def phase_early(roadmap, a):
    """Record one member ported early under the cycle rule, and read it back."""
    for need in ("task", "owner_task", "member", "rust_item"):
        if getattr(a, need) in (None, ""):
            raise SystemExit(f"--phase early needs --{need.replace('_', '-')}")
    hit = read(roadmap,
               f"MATCH (c:Class)-[:DECLARES]->(m:Method {{qualifiedName: {esc(a.member)}}}) "
               "RETURN c.module")
    if not hit:
        raise SystemExit(f"no Method {a.member!r} in the graph")
    if hit[0][0] not in ADDED_MODULES:
        raise SystemExit(f"{a.member!r} belongs to {hit[0][0]}, not to a module beyond "
                         "lucene/core")
    for tid in (a.task, a.owner_task):
        if not read(roadmap, f"MATCH (t:Task {{id: {int(tid)}}}) RETURN t.id"):
            raise SystemExit(f"no Task #{tid} in the graph")
    rmp("create", roadmap,
        f"MATCH (t:Task {{id: {int(a.task)}}}) "
        f"MATCH (m:Method {{qualifiedName: {esc(a.member)}}}) MERGE (t)-[:PORTS_EARLY]->(m)")
    rmp("update", roadmap,
        f"MATCH (t:Task {{id: {int(a.task)}}})-[e:PORTS_EARLY]->"
        f"(m:Method {{qualifiedName: {esc(a.member)}}}) "
        f"SET e.ownerTask = {int(a.owner_task)}, e.rustItem = {esc(a.rust_item)}, "
        f"e.evidence = 'cycle-rule', e.gitCommit = {esc(a.commit)}, e.gitDate = {esc(a.date)}")
    back = read(roadmap,
                f"MATCH (t:Task {{id: {int(a.task)}}})-[e:PORTS_EARLY]->"
                f"(m:Method {{qualifiedName: {esc(a.member)}}}) RETURN e.ownerTask, e.rustItem")
    if back != [[int(a.owner_task), a.rust_item]]:
        raise SystemExit(f"PORTS_EARLY read-back mismatch: {back}")
    print(f"  PORTS_EARLY #{a.task} -> {a.member} (owner #{a.owner_task})", file=sys.stderr)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roadmap")
    ap.add_argument("--lucene-root", default=DEFAULT_LUCENE_ROOT)
    ap.add_argument("--rt-root", help=f"randomizedtesting 2.8.1 clone (else ${RT_ROOT_ENV}, "
                                      f"else {DEFAULT_RT_ROOT})")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--phase", default="all",
                    choices=["all", "modules", "structure", "coverage", "early"])
    ap.add_argument("--task", type=int)
    ap.add_argument("--owner-task", type=int)
    ap.add_argument("--member")
    ap.add_argument("--rust-item")
    a = ap.parse_args(argv)
    roots = repo_roots(a.lucene_root, a.rt_root)
    if a.phase == "early":
        return phase_early(a.roadmap, a)
    for repo, path in roots.items():
        if not path.is_dir():
            raise SystemExit(f"no {repo} clone at {path}")
    if a.phase in ("all", "modules"):
        phase_modules(a.roadmap, roots, a.commit, a.date)
    if a.phase in ("all", "structure"):
        phase_structure(a.roadmap, roots, a.commit, a.date)
    if a.phase in ("all", "coverage"):
        phase_coverage(a.roadmap)


if __name__ == "__main__":
    main()
