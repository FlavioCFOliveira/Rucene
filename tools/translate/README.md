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
  `HTMLStripCharFilter`): their DFA tables, in Java's packed form (the packed `String`
  constants and the ported `zzUnpack*` routines, run at first use), into the wholly generated
  `crates/rucene-analysis-common/src/analysis/<pkg>/<scanner>_tables.rs`,
  and the JFlex skeleton with its simple actions into the `// @generated begin/end` regions
  (`constants`, `fields`, `init`, `skeleton`) of the hand-written `<scanner>.rs`, which it
  then formats with `rustfmt`. It checks every skeleton member token for token against the
  JFlex 1.8.2 skeleton and fails, naming the scanner, on any other construct; the user code
  and the actions translated by hand (`fn action_<N>`, `fn eof_code`) are pinned by the
  SHA-256 of their Java tokens. Rerun: `python3 tools/translate/jflex_java_to_rust.py
  [/tmp/lucene1050]` (`--check` compares instead of writing, `--only <JavaClass>` limits the
  run to one scanner, `--hashes` prints the pins).
* `lang_families.py` — translates the per-language class families of `lucene/analysis/common`
  (packages `ca`, `da`, `de`, `es`, `fr`, `gl`, `it`, `nl`, `no`, `pt`, `sv`, `el`, `br`, `tr`,
  `ro`, `id`, `ga`, `ru`, `cz`, `bg`, `sr`, `lv`, `lt`, `fi`, `hu`, `et`, `eu`, `hy`, `ar`, `fa`,
  `ckb`, `in`, `hi`, `bn`, `te`, `ta`, `ne`) into
  `crates/rucene-analysis-common/src/analysis/<pkg>/<class>.rs`: the
  keyword-aware stem filters (`stemmer.stem(termAtt.buffer(), termAtt.length())`, and the
  `resizeBuffer(len + 1)` variant), the stem and normalization filters of the general template
  (a stemmer's `stem` or a normalizer's `normalize` on every term or every non-keyword term),
  the token filter factories without arguments, and the
  `StopwordAnalyzerBase` analyzers with a default stop set and a chain of known filters. It
  matches each Java class token for token against its family's template, captures only the
  varying parts (names, `NAME`, resources, constant sets, the filter chain, messages, javadoc),
  and reports every class that deviates, with the reason, without generating it; those are
  translated by hand. The output is formatted by `rustfmt`. Rerun:
  `python3 tools/translate/lang_families.py [/tmp/lucene1050]` (`--check` compares instead of
  writing, `--list` prints each class as generated or deviating).
* `JdkUnicodeBlockData.java` — writes `crates/rucene-analysis-common/src/jdk/unicode_block_data.rs`
  (`Character.UnicodeBlock.of(int)` for every code point, as runs over the blocks' canonical
  names) from the running JDK. Rerun with JDK 25: `java tools/translate/JdkUnicodeBlockData.java > crates/rucene-analysis-common/src/jdk/unicode_block_data.rs`.
* `JdkCharacterTypeData.java` — writes `crates/rucene-analysis-common/src/jdk/character_type_data.rs`
  (`Character.getType(int)` runs, the Latin-1 `isLowerCase`/`isUpperCase` sets, and the
  `isUpperCase`/`isLowerCase` runs of every code point) from the running JDK. Rerun with JDK 25: `java tools/translate/JdkCharacterTypeData.java > crates/rucene-analysis-common/src/jdk/character_type_data.rs`.
* `JdkCharsetData.java` — writes `crates/rucene-analysis-common/src/jdk/charset_data.rs` (the
  canonical name, the aliases and the decoder kind of every charset of
  `Charset.availableCharsets()`, and the byte-to-unit decoding table of every single-byte charset,
  each byte's unmappable or malformed input marked) and `crates/rucene-analysis-common/src/jdk/charset_multibyte.bin`
  (zlib items: the byte-sequence machine of every table-driven multi-byte charset, probed from
  each shift state of the JDK decoder, whose state it reads by reflection; the `GB18030` two-byte
  and four-byte tables; the `x-ISCII91` byte table) from the running JDK. It checks every machine
  and table against the JDK decoder (pseudo-random streams fed in random chunks into random small
  output buffers, compared call for call; every `GB18030` sequence of one, two and four bytes) and
  fails on any difference; it runs for about two minutes. Rerun with JDK 25:

  ```sh
  java -Xmx4g \
    --add-opens java.base/sun.nio.cs=ALL-UNNAMED \
    --add-opens jdk.charsets/sun.nio.cs.ext=ALL-UNNAMED \
    tools/translate/JdkCharsetData.java \
    crates/rucene-analysis-common/src/jdk/charset_data.rs \
    crates/rucene-analysis-common/src/jdk/charset_multibyte.bin
  ```
* `JdkSimpleCaseData.java` — rewrites, in place, the simple case-mapping tables `SIMPLE_UPPERCASE`
  and `SIMPLE_LOWERCASE` of `src/util/automaton/case_folding.rs`: every code point whose
  `Character.toUpperCase(int)` / `toLowerCase(int)` differs from it, on JDK 25 (Unicode 16.0; it
  refuses another JDK). Deterministic; a rerun on its output changes nothing. Rerun with JDK 25:

  ```sh
  java tools/translate/JdkSimpleCaseData.java src/util/automaton/case_folding.rs
  ```
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
* `BrkCheck.java`, `DbbiCheck.java`, `BrkBackCheck.java` — not translators: Java models of the
  JDK 25 break iterators that `crates/rucene-analysis-common/src/jdk/break_iterator.rs`,
  `break_iterator_rules.rs` and `break_dictionary.rs` implement, checked against the running JDK on
  pseudo-random strings. `BrkCheck` compares the forward tables with `handleNext` (root word, Thai
  word ranges, sentence); `DbbiCheck` the Thai dictionary splitting, run on the JDK's own `thai_dict`;
  `BrkBackCheck` random sequences of `first`, `last`, `next`, `previous`, `following`, `preceding`
  and `isBoundary` (backward tables and `handlePrevious`) on all three iterators. Each prints every
  difference and `mismatches=0` when it finds none; they validate the derivation the Rust code
  follows, not the Rust code. Run with JDK 25 (the last two read `thai_dict` by reflection):

  ```sh
  java tools/translate/BrkCheck.java [seed] [maxLength]
  java --add-opens java.base/sun.text=ALL-UNNAMED tools/translate/DbbiCheck.java [seed]
  java --add-opens java.base/sun.text=ALL-UNNAMED tools/translate/BrkBackCheck.java [seed] [count]
  ```
* `../gen/CollatorData.java` (in `tools/gen`, where the test framework's JDK generators live) —
  writes `crates/rucene-analysis-common/src/jdk/collator_data.rs`, the data of the
  `java.text.Collator` emulation: the root rules, the decompositions and combining classes, and
  the locale tailorings of `Collator.getInstance(Locale)`, with a check of the locale resolution
  on every available locale; see its header for the command.
