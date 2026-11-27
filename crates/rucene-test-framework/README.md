# rucene-test-framework

Rust port of Apache Lucene 10.5.0 `lucene/test-framework`.

## Java-oracle harness of the sprint-17 modules

`tests/sprint17_oracle/mod.rs` (repository root) runs Lucene 10.5.0 `memory`,
`facet`, `highlighter`, `spatial3d`, `spatial-extras` and `benchmark` side by
side with Rucene on one seed (rmp #628). It follows the harness of the query
modules (`crates/rucene-queries/tests/query_oracle/mod.rs`, rmp #475).

### Setup

The harness needs a JDK (`java` on `PATH`; the oracles run as single source
files, compiled once per change by `tests/jvm_fixture/mod.rs`) and these jars
in the local Maven repository (`~/.m2/repository`), each with the `.sha1`
file Maven Central publishes beside it:

| Artifact | Version |
|---|---|
| `org.apache.lucene:lucene-{core,codecs,analysis-common,queries,queryparser,test-framework}` | 10.5.0 |
| `org.apache.lucene:lucene-{memory,highlighter,facet,spatial3d,spatial-extras,benchmark,analysis-icu}` | 10.5.0 |
| `org.locationtech.spatial4j:spatial4j` | 0.8 |
| `io.sgr:s2-geometry-library-java` | 1.0.0 |
| `org.apache.commons:commons-compress` | 1.19 |
| `com.ibm.icu:icu4j` | 77.1 |
| `net.sourceforge.nekohtml:nekohtml` | 1.9.17 |
| `xerces:xercesImpl` | 2.12.2 |
| `com.carrotsearch.randomizedtesting:randomizedtesting-runner` | 2.8.1 |
| `junit:junit` | 4.13.1 |
| `org.hamcrest:hamcrest` | 2.2 |

Fetch a missing one from Maven Central, for example:

```bash
d=~/.m2/repository/org/apache/lucene/lucene-memory/10.5.0
mkdir -p $d
for ext in jar jar.sha1 pom pom.sha1; do
  curl -sf -o $d/lucene-memory-10.5.0.$ext \
    https://repo1.maven.org/maven2/org/apache/lucene/lucene-memory/10.5.0/lucene-memory-10.5.0.$ext
done
```

The harness checks the SHA-1 of every jar against a pinned value and against
the `.sha1` file, once per process.

### Use

A test includes the module and compares its dump with the oracle's:

```rust,ignore
#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;
use sprint17_oracle::Module;

sprint17_oracle::pin_tests_seed();
let java = sprint17_oracle::run(Module::Memory, "smoke", SEED, &[]);
sprint17_oracle::assert_same(&java, &rust_dump(SEED));
```

The crate's `[dev-dependencies]` must contain `randomizedtesting`; the harness
also depends on `rucene`.

* `run(module, scenario, seed, args)` / `run_with_input(…, input)` start
  `tests/fixtures/sprint17-oracle/<Module>Oracle.java` with
  `seed scenario input args…` (`input` is the path of the written text, or `-`)
  in a fresh directory under `TMPDIR`, with `-ea -esa`, `-Dtests.seed` and
  `-XX:-UseLibmIntrinsic` (so `Math.sin`, `cos`, `tan` and `pow` are
  fdlibm's, as `rucene::util::java_math` reproduces them), and return its
  standard output.
* `pin_tests_seed()` gives both sides one `tests.seed` (`StringHelper`
  hash seed).
* The Java side draws from `java.util.Random(seed)`. The Rust side uses
  `randomizedtesting::jdk::random::JdkRandom::new(seed)`, whose draws are equal.
  `corpus::{word, text}`, `escape`, `bits32` and `bits64` mirror the helpers
  of the Java programs.
* A new parity check adds a `case` to the program's `run` switch and a Rust
  test that prints the same dump.

Each module has a smoke test (`*_oracle_smoke.rs` in `rucene-memory`,
`rucene-facet`, `rucene-highlighter`, `rucene-spatial-extras` (which holds
both `spatial3d` and `spatial-extras`) and `rucene-benchmark`).
