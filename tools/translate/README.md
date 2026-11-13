# tools/translate

Mechanical translators and JDK-data generators for the port of
`lucene/analysis/common` (Lucene 10.5.0). Each is deterministic; its output is
the committed file, and rerunning it must leave that file unchanged.

* `ascii_folding_filter.py` — translates the `switch` of
  `ASCIIFoldingFilter.foldToASCII` into the `match` between the `GENERATED`
  markers of `crates/rucene-analysis-common/src/analysis/miscellaneous/ascii_folding_filter.rs`,
  then runs `rustfmt`. Rerun: `python3 tools/translate/ascii_folding_filter.py [/tmp/lucene1050]`.
* `JdkCharacterTypeData.java` — writes `crates/rucene-analysis-common/src/jdk/character_type_data.rs`
  (`Character.getType(int)` runs and the Latin-1 `isLowerCase`/`isUpperCase` sets) from the
  running JDK. Rerun with JDK 25: `java tools/translate/JdkCharacterTypeData.java > crates/rucene-analysis-common/src/jdk/character_type_data.rs`.
* `JdkDateFormatData.java` — writes `crates/rucene-analysis-common/src/jdk/date_format_data.rs`
  (the per-locale data `SimpleDateFormat.parse` reads) from the running JDK. Rerun with
  JDK 25: `java tools/translate/JdkDateFormatData.java > crates/rucene-analysis-common/src/jdk/date_format_data.rs`.
