# tools/translate

Mechanical translators and JDK-data generators for the port of
`lucene/analysis/common` (Lucene 10.5.0). Each is deterministic; its output is
the committed file, and rerunning it must leave that file unchanged.

* `ascii_folding_filter.py` — translates the `switch` of
  `ASCIIFoldingFilter.foldToASCII` into the `match` between the `GENERATED`
  markers of `crates/rucene-analysis-common/src/analysis/miscellaneous/ascii_folding_filter.rs`,
  then runs `rustfmt`. Rerun: `python3 tools/translate/ascii_folding_filter.py [/tmp/lucene1050]`.
* `kstem_data.py` — translates `KStemData1.java`…`KStemData8.java` into
  `crates/rucene-analysis-common/src/analysis/en/kstem_data{1..8}.rs` (Java layout, the modules
  are `#[rustfmt::skip]`) and the string tables of `KStemmer.java` into the region between the
  `GENERATED` markers of `crates/rucene-analysis-common/src/analysis/en/kstemmer.rs`, then runs
  `rustfmt` on that file. Rerun: `python3 tools/translate/kstem_data.py [/tmp/lucene1050]`.
* `snowball_java_to_rust.py` — translates the 30 generated Snowball stemmers of Lucene
  (`lucene/analysis/common/src/java/org/tartarus/snowball/ext/*Stemmer.java`, Snowball 2.2.0
  output with Lucene's edits) into `crates/rucene-analysis-common/src/tartarus/snowball/ext/*_stemmer.rs`,
  statement for statement, then runs `rustfmt`. It accepts only the constructs those files use
  and fails, naming the file and line, on any other. Rerun:
  `python3 tools/translate/snowball_java_to_rust.py [/tmp/lucene1050]` (`--coverage` prints the
  count of each construct translated).
* `jflex_java_to_rust.py` — translates the four JFlex 1.8.2 scanners of Lucene
  (`ClassicTokenizerImpl`, `WikipediaTokenizerImpl`, `UAX29URLEmailTokenizerImpl`,
  `HTMLStripCharFilter`): their DFA tables, unpacked as the Java `zzUnpack*` routines do, into
  the wholly generated `crates/rucene-analysis-common/src/analysis/<pkg>/<scanner>_tables.rs`,
  and the JFlex skeleton with its simple actions into the `// @generated begin/end` regions
  (`constants`, `fields`, `init`, `skeleton`) of the hand-written `<scanner>.rs`, which it
  then formats with `rustfmt`. It checks every skeleton member token for token against the
  JFlex 1.8.2 skeleton and fails, naming the scanner, on any other construct; the user code
  and the actions translated by hand (`fn action_<N>`, `fn eof_code`) are pinned by the
  SHA-256 of their Java tokens. Rerun: `python3 tools/translate/jflex_java_to_rust.py
  [/tmp/lucene1050]` (`--check` compares instead of writing, `--only <JavaClass>` limits the
  run to one scanner, `--hashes` prints the pins).
* `JdkCharacterTypeData.java` — writes `crates/rucene-analysis-common/src/jdk/character_type_data.rs`
  (`Character.getType(int)` runs and the Latin-1 `isLowerCase`/`isUpperCase` sets) from the
  running JDK. Rerun with JDK 25: `java tools/translate/JdkCharacterTypeData.java > crates/rucene-analysis-common/src/jdk/character_type_data.rs`.
* `JdkDateParseData.java` — writes `crates/rucene-analysis-common/src/jdk/date_parse_data.bin`
  (zlib), the JDK locale data `SimpleDateFormat` parsing reads: the CLDR resolution metadata
  (language tags, aliases, parent locales, likely scripts), the display names, `DEFAULT` date
  patterns, integer number formats and time-zone names of every locale class, the time-zone
  IDs and the BCP 47 `tz` IDs. It validates its data against the JDK's own formats on a
  sample of locales and fails on a difference. It reads JDK internals, so it needs the
  `--add-exports`/`--add-opens` flags below; it runs for about seven minutes. Rerun with JDK 25:

  ```sh
  java -Xmx6g \
    --add-exports java.base/sun.util.locale.provider=ALL-UNNAMED \
    --add-exports java.base/sun.util.resources=ALL-UNNAMED \
    --add-opens java.base/sun.util.cldr=ALL-UNNAMED \
    --add-opens java.base/sun.util.locale.provider=ALL-UNNAMED \
    --add-opens java.base/sun.util.calendar=ALL-UNNAMED \
    --add-opens java.base/java.text=ALL-UNNAMED \
    tools/translate/JdkDateParseData.java crates/rucene-analysis-common/src/jdk/date_parse_data.bin
  ```

  Two parts of the data depend on the moment and the JVM, as the JDK's own
  answers do: the GMT-format fallback names of the CLDR time-zone names are
  computed from the current instant, and the CLDR provider caches derived
  zone names across locales. The generator clears those caches before each
  locale, so the data holds the names a JVM gives when that locale is the
  first it formats, as of the generation date; a rerun after a time-zone
  rule change can differ there.
