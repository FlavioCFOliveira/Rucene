# randomizedtesting oracle harness

Java programs that run randomizedtesting 2.8.1 and JDK 25 on fixed inputs and
write the fixtures (`../*.tsv`) the crate's parity tests (`src/tests/`) read.

* **Rebuild everything:** `./regenerate.sh` (JDK 25; jars from `~/.m2` or
  `$M2_REPO`: randomizedtesting-runner 2.8.1, junit 4.13.1,
  hamcrest-core 1.3). It compiles every `src/**/*.java`, runs each class in
  `oracles.txt` with the output directory as its argument, copies the
  `*.tsv` files to `tests/fixtures/` and the JDK data lists to `src/jdk/data/`.
* **Format:** tab-separated rows, `#` comment lines. A sequence of values is
  `OracleSupport.Rec`: an FNV-1a 64-bit digest over the little-endian bytes
  of each value (`int`/`long` as is, `float`/`double` by raw bits, `boolean`
  as one byte, `byte[]` and `String` prefixed by a 4-byte length, strings as
  UTF-16 code units) plus the first value. Strings in inputs are written as
  hex UTF-16 code units joined by `.` (`-` when empty); errors as
  `class<TAB>message`. The Rust side is `src/tests/mod.rs` (`Rec`, `rows`,
  `from_units`, `java_error`).
* **Extend:** add a class under `src/com/carrotsearch/randomizedtesting/`
  (that package reaches the package-private members), use `OracleSupport`
  (`Rec`, `units`, `err`, `seeds`, `tryRecord`), write its own `*.tsv` names
  into the directory it receives, list it in `oracles.txt`, and add the Rust
  test module that reads the new files.
