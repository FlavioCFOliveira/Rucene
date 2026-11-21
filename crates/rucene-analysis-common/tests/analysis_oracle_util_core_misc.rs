/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Java-oracle portability tests of the packages `analysis.util` (rmp
//! #448), `analysis.core` (#449) and `analysis.miscellaneous` (#450,
//! #451): every analyzer, tokenizer, filter and factory of each task, with
//! representative parameterisations of each factory's documented
//! parameters, analyses the same corpus on Lucene 10.5.0 and Rucene through
//! the harness of #446 ([`analysis_oracle`]), and every token attribute is
//! compared. The corpus is the harness's standard one (its fixed edge cases
//! and seeded random inputs) plus inputs that reach each task's filters
//! (dates, `|` frequencies, hyphenated line ends, Scandinavian spellings,
//! word-delimiter shapes).
//!
//! Types no analyzer or factory reaches are exercised by the ported Lucene
//! tests instead: `EmptyTokenStream` and `ConcatenatingTokenStream`
//! (`analysis_miscellaneous*_tests`), `CharArrayIterator`, `CSVUtil`,
//! `RollingCharBuffer`, `OpenStringBuilder`, `FilesystemResourceLoader` and
//! the abstract `SegmentingTokenizerBase`/`CharTokenizer`
//! (`analysis_util_tests`, `analysis_core_tests`); `StemmerUtil` and
//! `UnicodeProps` are reached through the filters that call them.

mod analysis_oracle;
mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use analysis_oracle::{corpus, Arg, Chain, Oracle, Spec};
use rucene::analysis::{Analyzer, CharArraySet};
use rucene::util::java_collections::JavaHashMap;
use rucene_analysis_common::analysis::core::keyword_analyzer::KeywordAnalyzer;
use rucene_analysis_common::analysis::core::simple_analyzer::SimpleAnalyzer;
use rucene_analysis_common::analysis::core::stop_analyzer::StopAnalyzer;
use rucene_analysis_common::analysis::core::unicode_whitespace_analyzer::UnicodeWhitespaceAnalyzer;
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::limit_token_count_analyzer::LimitTokenCountAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::per_field_analyzer_wrapper::PerFieldAnalyzerWrapper;

/// The seed of the random half of the corpus.
const SEED: i64 = 0x0448_0451;

/// The number of random inputs.
const RANDOM_INPUTS: usize = 200;

/// Inputs that reach the filters of the four tasks beyond the standard
/// corpus.
const TASK_INPUTS: &[&str] = &[
    "7/11/2006",
    "07/11/2006 7/12/2006 13/45/2006 7/11/06",
    "Jul 11, 2006",
    "July 11, 2006",
    "11.07.2006",
    "2006-07-11",
    "11 juil. 2006",
    "11. Juli 2006",
    "quick|40 red|4 fox|06 jumped|1 dogs|1024 plain",
    "bad|x",
    "neg|-20",
    "float|1.2",
    "ecologi-\r\ncal devel-\r\n\r\nop compre-\u{0009}hensive-hands-on and ecologi-\ncal",
    "abc- def geh 1234- 5678-",
    "bl\u{e5}b\u{e6}rsyltet\u{f8}j r\u{e4}ksm\u{f6}rg\u{e5}s raeksmoergaas BLAABAERSYLTETOEJ \u{c6}\u{d8}\u{c5} aa ao ae oe oo",
    "PowerShot500-42 wi-fi O'Neil's j2se Super-Duper-XL500 ABCDEfgh 42a43 sd-500 a..b",
    "Hello thEre my Name is Ryan McKinley 1st 2nd third and the it BIG kiTTEN",
    "  trim me  ",
    "l'avion L'avion d'Artagnan qu'il m'appelle j'ai",
    "B A B E A1 B2 A1 D4 C3",
    "121 is palindrome, while 123 is not; e-mail me at a@b.org or http://x.org",
    "foo bar the Quick caf\u{e9} fox's the THE The dogs cats Cats birds",
    "Des mot cl\u{e9}s \u{c0} LA CHA\u{ce}NE \u{c6} \u{132} \u{153} \u{df} \u{fb01} \u{fb02} \u{a7} \u{2049} \u{ff03}",
    "1234\u{1f603}5 \u{1f603}12345 \u{1f603}\u{1f603}\u{1f603}\u{1f603}\u{1f603}\u{1f603}",
];

/// The corpus: the harness's standard corpus and [`TASK_INPUTS`].
fn task_corpus() -> Vec<Vec<u16>> {
    let mut c = corpus::standard(SEED, RANDOM_INPUTS);
    c.extend(TASK_INPUTS.iter().map(|s| s.encode_utf16().collect()));
    c
}

/// A directory holding the resource files the specs name: the Lucene test
/// resources of the packages (byte for byte) and files written here whose
/// words the corpus holds.
fn config_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rucene-oracle-t2-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the config dir");
    for (resource, bytes) in support::TEST_RESOURCES {
        let file = resource.rsplit('/').next().expect("a file name");
        std::fs::write(dir.join(file), bytes).expect("write a test resource");
    }
    let files: [(&str, &str); 7] = [
        (
            "words.txt",
            "# words\nthe\nQuick\ncaf\u{e9}\nfox's\nit's\nThe\nfoo\nbar\n",
        ),
        ("types.txt", "<NUM>\n<EMAIL>\n<URL>\n"),
        ("articles.txt", "l\nd\nm\nqu\nj\n"),
        (
            "stemdict.txt",
            "dogs\tcat\nfoxes\tfox\nthe\tTHE\ncaf\u{e9}\tcoffee\n",
        ),
        (
            "wdftypes.txt",
            "# types\n' => ALPHA\n- => DIGIT\n\\u002E => ALPHANUM\n$ => DIGIT\n",
        ),
        ("protwords.txt", "PowerShot500-42\nwi-fi\nO'Neil's\n"),
        ("snowball.txt", " | comment\nthe a | more\n  of it's\n"),
    ];
    for (file, content) in files {
        std::fs::write(dir.join(file), content).expect("write a config file");
    }
    dir
}

/// A factories spec over the whitespace tokenizer.
fn ws(id: &str, dir: &Path, filters: &[(&str, &[(&str, &str)])]) -> Spec {
    let mut chain = Chain::new().config_dir(dir).tokenizer("whitespace", &[]);
    for (name, params) in filters {
        chain = chain.filter(name, params);
    }
    Spec::factories(id, chain)
}

/// A factories spec over the keyword tokenizer.
fn kw(id: &str, dir: &Path, filters: &[(&str, &[(&str, &str)])]) -> Spec {
    let mut chain = Chain::new().config_dir(dir).tokenizer("keyword", &[]);
    for (name, params) in filters {
        chain = chain.filter(name, params);
    }
    Spec::factories(id, chain)
}

/// A factories spec over the standard tokenizer (it sets token types).
fn std_tok(id: &str, dir: &Path, filters: &[(&str, &[(&str, &str)])]) -> Spec {
    let mut chain = Chain::new().config_dir(dir).tokenizer("standard", &[]);
    for (name, params) in filters {
        chain = chain.filter(name, params);
    }
    Spec::factories(id, chain)
}

/// Runs the oracle and checks that Lucene built every spec (a spec both
/// sides refuse would compare equal without proving anything).
fn assert_oracle(o: &Oracle) {
    let java = o.java_dump();
    o.assert_covers(&java);
    let mut spec_errors: Vec<String> = Vec::new();
    let mut spec = "";
    for line in java.lines() {
        if let Some(id) = line.strip_prefix("spec ") {
            spec = id;
        } else if line.starts_with("spec-error ") {
            spec_errors.push(format!("{spec}: {line}"));
        }
    }
    assert!(
        spec_errors.is_empty(),
        "Lucene refused a spec: {spec_errors:?}"
    );
    let java = lone_surrogates_as_replacement(&java);
    if let Err(report) = analysis_oracle::compare(&java, &o.rust_dump(), o) {
        panic!("{report}");
    }
}

/// Lucene's dump with every lone surrogate of a `term=` field written as
/// U+FFFD, the character `rucene`'s `CharTermAttribute` (which holds
/// `char`s) carries in its place — the divergence declared on
/// `rucene::analysis::CharTermAttribute` and on `TruncateTokenFilter`
/// (`truncateAfterChars` cuts surrogate pairs). The `bytes=` field is not
/// touched: Lucene's UTF-8 encoding writes a lone surrogate as U+FFFD
/// (`ef bf bd`), so it must already be equal. Prints how many terms this
/// rewrote.
fn lone_surrogates_as_replacement(java: &str) -> String {
    let mut rewritten = 0;
    let mut out = String::with_capacity(java.len());
    for line in java.lines() {
        let new_line = match line.find(" term=") {
            Some(start) => {
                let value_start = start + " term=".len();
                let value_end = line[value_start..]
                    .find(' ')
                    .map_or(line.len(), |i| value_start + i);
                let term = &line[value_start..value_end];
                let fixed = replace_lone_surrogate_escapes(term);
                if fixed != term {
                    rewritten += 1;
                }
                format!("{}{}{}", &line[..value_start], fixed, &line[value_end..])
            }
            None => line.to_string(),
        };
        out.push_str(&new_line);
        out.push('\n');
    }
    println!("lone surrogates written as U+FFFD in {rewritten} Lucene terms");
    out
}

/// `\uD800`-`\uDBFF` not followed by `\uDC00`-`\uDFFF`, and
/// `\uDC00`-`\uDFFF` not preceded by a high surrogate, written as `\uFFFD`.
fn replace_lone_surrogate_escapes(term: &str) -> String {
    let unit = |s: &str, i: usize| -> Option<u16> {
        let esc = s.get(i..i + 6)?;
        let hex = esc.strip_prefix("\\u")?;
        u16::from_str_radix(hex, 16).ok()
    };
    let mut out = String::new();
    let mut i = 0;
    while i < term.len() {
        if let Some(u) = unit(term, i) {
            if (0xD800..0xDC00).contains(&u) {
                if let Some(low) = unit(term, i + 6) {
                    if (0xDC00..0xE000).contains(&low) {
                        out.push_str(&term[i..i + 12]);
                        i += 12;
                        continue;
                    }
                }
                out.push_str("\\uFFFD");
                i += 6;
                continue;
            }
            if (0xDC00..0xE000).contains(&u) {
                out.push_str("\\uFFFD");
                i += 6;
                continue;
            }
            out.push_str(&term[i..i + 6]);
            i += 6;
            continue;
        }
        let c = term[i..].chars().next().expect("a char");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// rmp #448, `analysis.util`: `ElisionFilter` and its factory (default
/// articles, an articles file, `ignoreCase`); `CharTokenizer`, through the
/// tokenizers of #449.
#[test]
fn oracle_448_util() {
    let dir = config_dir("448");
    let mut o = Oracle::new(task_corpus());
    o.spec(ws("elision-default", &dir, &[("elision", &[])]))
        .spec(ws(
            "elision-articles",
            &dir,
            &[("elision", &[("articles", "articles.txt")])],
        ))
        .spec(ws(
            "elision-articles-ic",
            &dir,
            &[(
                "elision",
                &[("articles", "frenchArticles.txt"), ("ignoreCase", "true")],
            )],
        ))
        .spec(Spec::custom(
            "custom-standard-elision-lowercase",
            Chain::new()
                .tokenizer("standard", &[])
                .filter("elision", &[])
                .filter("lowercase", &[]),
        ));
    assert_oracle(&o);
}

/// rmp #449, `analysis.core`: the analyzers (`KeywordAnalyzer`,
/// `SimpleAnalyzer`, `StopAnalyzer`, `UnicodeWhitespaceAnalyzer`,
/// `WhitespaceAnalyzer` with and without a maximum token length), the
/// tokenizer factories (`keyword`, `letter`, `whitespace` with both rules,
/// with `maxTokenLen`), and the filter factories (`decimalDigit`,
/// `flattenGraph`, `lowercase`, `stop` in both formats and cases, `type`
/// as a black and a white list, `uppercase`).
#[test]
fn oracle_449_core() {
    let dir = config_dir("449");
    let stop_words: Vec<String> = ["the", "a", "of", "it's", "caf\u{e9}"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let rust_stop = stop_words.clone();
    let mut o = Oracle::new(task_corpus());
    o.spec(Spec::analyzer(
        "keyword-analyzer",
        "org.apache.lucene.analysis.core.KeywordAnalyzer",
        || Ok(Box::new(KeywordAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(Spec::analyzer(
        "simple-analyzer",
        "org.apache.lucene.analysis.core.SimpleAnalyzer",
        || Ok(Box::new(SimpleAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "stop-analyzer",
            "org.apache.lucene.analysis.core.StopAnalyzer",
            move || {
                Ok(Box::new(StopAnalyzer::new(CharArraySet::from_iter(
                    rust_stop.clone(),
                    false,
                ))) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::CharSet(stop_words)),
    )
    .spec(Spec::analyzer(
        "unicode-whitespace-analyzer",
        "org.apache.lucene.analysis.core.UnicodeWhitespaceAnalyzer",
        || Ok(Box::new(UnicodeWhitespaceAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(Spec::analyzer(
        "whitespace-analyzer",
        "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
        || Ok(Box::new(WhitespaceAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "whitespace-analyzer-3",
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
            || Ok(Box::new(WhitespaceAnalyzer::with_max_token_length(3)) as Box<dyn Analyzer>),
        )
        .arg(Arg::Int(3)),
    )
    .spec(Spec::factories(
        "keyword-tokenizer",
        Chain::new().tokenizer("keyword", &[]),
    ))
    .spec(Spec::factories(
        "keyword-tokenizer-2",
        Chain::new().tokenizer("keyword", &[("maxTokenLen", "2")]),
    ))
    .spec(Spec::factories(
        "letter-tokenizer",
        Chain::new().tokenizer("letter", &[]),
    ))
    .spec(Spec::factories(
        "letter-tokenizer-4",
        Chain::new().tokenizer("letter", &[("maxTokenLen", "4")]),
    ))
    .spec(Spec::factories(
        "whitespace-tokenizer-java",
        Chain::new().tokenizer("whitespace", &[("rule", "java")]),
    ))
    .spec(Spec::factories(
        "whitespace-tokenizer-unicode-5",
        Chain::new().tokenizer("whitespace", &[("rule", "unicode"), ("maxTokenLen", "5")]),
    ))
    .spec(ws("decimal-digit", &dir, &[("decimalDigit", &[])]))
    .spec(ws(
        "flatten-graph",
        &dir,
        &[
            (
                "wordDelimiterGraph",
                &[("catenateAll", "1"), ("preserveOriginal", "1")],
            ),
            ("flattenGraph", &[]),
        ],
    ))
    .spec(ws("lowercase", &dir, &[("lowercase", &[])]))
    .spec(ws("uppercase", &dir, &[("uppercase", &[])]))
    .spec(ws("stop-default", &dir, &[("stop", &[])]))
    .spec(ws(
        "stop-words-ic",
        &dir,
        &[("stop", &[("words", "words.txt"), ("ignoreCase", "true")])],
    ))
    .spec(ws(
        "stop-snowball",
        &dir,
        &[(
            "stop",
            &[
                ("words", "stop-snowball.txt, snowball.txt"),
                ("format", "snowball"),
            ],
        )],
    ))
    .spec(std_tok(
        "type-blacklist",
        &dir,
        &[("type", &[("types", "types.txt")])],
    ))
    .spec(std_tok(
        "type-whitelist",
        &dir,
        &[(
            "type",
            &[
                ("types", "stoptypes-1.txt, stoptypes-2.txt"),
                ("useWhitelist", "true"),
            ],
        )],
    ))
    .spec(Spec::custom(
        "custom-letter-lower-upper",
        Chain::new()
            .tokenizer("letter", &[])
            .filter("decimalDigit", &[])
            .filter("lowercase", &[])
            .filter("uppercase", &[]),
    ));
    assert_oracle(&o);
}

/// rmp #450, `analysis.miscellaneous` part 1: every filter factory of the
/// task with its documented parameters, `LimitTokenCountAnalyzer` and
/// `PerFieldAnalyzerWrapper`.
#[test]
fn oracle_450_miscellaneous() {
    let dir = config_dir("450");
    let mut o = Oracle::new(task_corpus());
    o.spec(ws("ascii-folding", &dir, &[("asciiFolding", &[])]))
        .spec(ws(
            "ascii-folding-preserve",
            &dir,
            &[("asciiFolding", &[("preserveOriginal", "true")])],
        ))
        .spec(ws(
            "capitalization-default",
            &dir,
            &[("capitalization", &[])],
        ))
        .spec(ws(
            "capitalization-params",
            &dir,
            &[(
                "capitalization",
                &[
                    ("keep", "and the it BIG"),
                    ("onlyFirstWord", "false"),
                    ("minWordLength", "3"),
                    ("okPrefix", "McK"),
                    ("forceFirstLetter", "false"),
                ],
            )],
        ))
        .spec(kw(
            "capitalization-keyword",
            &dir,
            &[(
                "capitalization",
                &[
                    ("keep", "kitten the"),
                    ("keepIgnoreCase", "true"),
                    ("maxWordCount", "3"),
                    ("maxTokenLength", "4"),
                ],
            )],
        ))
        .spec(ws(
            "codepoint-count",
            &dir,
            &[("codepointCount", &[("min", "2"), ("max", "5")])],
        ))
        .spec(ws(
            "delimited-term-frequency",
            &dir,
            &[("delimitedTermFrequency", &[])],
        ))
        .spec(ws(
            "delimited-term-frequency-dash",
            &dir,
            &[("delimitedTermFrequency", &[("delimiter", "-")])],
        ))
        .spec(ws(
            "drop-if-flagged",
            &dir,
            &[("dropIfFlagged", &[("dropFlags", "0")])],
        ))
        .spec(ws("fix-broken-offsets", &dir, &[("fixBrokenOffsets", &[])]))
        .spec(ws("hyphenated-words", &dir, &[("hyphenatedWords", &[])]))
        .spec(ws(
            "keep-word",
            &dir,
            &[("keepWord", &[("words", "words.txt")])],
        ))
        .spec(ws(
            "keep-word-snowball-ic",
            &dir,
            &[(
                "keepWord",
                &[
                    ("words", "keep-snowball.txt, keep-1.txt"),
                    ("format", "snowball"),
                    ("ignoreCase", "true"),
                ],
            )],
        ))
        .spec(ws(
            "keyword-marker-protected",
            &dir,
            &[
                ("keywordMarker", &[("protected", "words.txt")]),
                ("porterStem", &[]),
            ],
        ))
        .spec(ws(
            "keyword-marker-pattern-ic",
            &dir,
            &[
                (
                    "keywordMarker",
                    &[("pattern", "c[a-z]+s|Dogs"), ("ignoreCase", "true")],
                ),
                ("porterStem", &[]),
            ],
        ))
        .spec(ws(
            "keyword-marker-mixed",
            &dir,
            &[
                (
                    "keywordMarker",
                    &[("protected", "words.txt"), ("pattern", "birds|Dogs")],
                ),
                ("porterStem", &[]),
            ],
        ))
        .spec(ws(
            "keyword-repeat-remove-duplicates",
            &dir,
            &[
                ("keywordRepeat", &[]),
                ("porterStem", &[]),
                ("removeDuplicates", &[]),
            ],
        ))
        .spec(ws(
            "length",
            &dir,
            &[("length", &[("min", "2"), ("max", "6")])],
        ))
        .spec(ws(
            "limit-token-count",
            &dir,
            &[("limitTokenCount", &[("maxTokenCount", "3")])],
        ))
        .spec(ws(
            "limit-token-count-consume",
            &dir,
            &[(
                "limitTokenCount",
                &[("maxTokenCount", "2"), ("consumeAllTokens", "true")],
            )],
        ))
        .spec(ws(
            "limit-token-offset",
            &dir,
            &[("limitTokenOffset", &[("maxStartOffset", "10")])],
        ))
        .spec(ws(
            "limit-token-offset-consume",
            &dir,
            &[(
                "limitTokenOffset",
                &[("maxStartOffset", "3"), ("consumeAllTokens", "true")],
            )],
        ))
        .spec(ws(
            "limit-token-position-shingles",
            &dir,
            &[
                (
                    "shingle",
                    &[
                        ("minShingleSize", "2"),
                        ("maxShingleSize", "3"),
                        ("outputUnigrams", "true"),
                    ],
                ),
                (
                    "limitTokenPosition",
                    &[("maxTokenPosition", "2"), ("consumeAllTokens", "true")],
                ),
            ],
        ))
        .spec(ws(
            "limit-token-position",
            &dir,
            &[("limitTokenPosition", &[("maxTokenPosition", "1")])],
        ))
        .spec(ws(
            "scandinavian-folding",
            &dir,
            &[("scandinavianFolding", &[])],
        ))
        .spec(ws(
            "scandinavian-normalization",
            &dir,
            &[("scandinavianNormalization", &[])],
        ))
        .spec(kw("trim", &dir, &[("trim", &[])]))
        .spec(ws(
            "truncate-code-points",
            &dir,
            &[("truncate", &[("truncateAfterCodePoints", "3")])],
        ))
        .spec(ws(
            "truncate-chars",
            &dir,
            &[("truncate", &[("truncateAfterChars", "5")])],
        ))
        .spec(ws(
            "truncate-prefix-length",
            &dir,
            &[("truncate", &[("prefixLength", "2")])],
        ))
        .spec(std_tok("type-as-synonym", &dir, &[("typeAsSynonym", &[])]))
        .spec(std_tok(
            "type-as-synonym-params",
            &dir,
            &[(
                "typeAsSynonym",
                &[
                    ("prefix", "_type_"),
                    ("ignore", "<ALPHANUM>,<NUM>"),
                    ("synFlagsMask", "5"),
                ],
            )],
        ))
        .spec(Spec::custom(
            "custom-misc",
            Chain::new()
                .tokenizer("whitespace", &[])
                .filter("asciiFolding", &[("preserveOriginal", "true")])
                .filter("removeDuplicates", &[])
                .filter("length", &[("min", "1"), ("max", "12")]),
        ));
    for consume_all in [false, true] {
        o.spec(
            Spec::analyzer(
                &format!("limit-token-count-analyzer-{consume_all}"),
                "org.apache.lucene.analysis.miscellaneous.LimitTokenCountAnalyzer",
                move || {
                    Ok(Box::new(LimitTokenCountAnalyzer::new_consuming(
                        Arc::new(WhitespaceAnalyzer::new()),
                        2,
                        consume_all,
                    )) as Box<dyn Analyzer>)
                },
            )
            .arg(Arg::Analyzer(
                "org.apache.lucene.analysis.core.WhitespaceAnalyzer".into(),
            ))
            .arg(Arg::Int(2))
            .arg(Arg::Boolean(consume_all)),
        );
    }
    // The oracle analyses the field `field`: one wrapper delegates it, the
    // other maps it.
    o.spec(
        Spec::analyzer(
            "per-field-default",
            "org.apache.lucene.analysis.miscellaneous.PerFieldAnalyzerWrapper",
            || {
                let mut map: JavaHashMap<Arc<dyn Analyzer>> = JavaHashMap::new();
                map.insert("other".to_string(), Arc::new(SimpleAnalyzer::new()));
                Ok(Box::new(PerFieldAnalyzerWrapper::new_with_fields(
                    Arc::new(WhitespaceAnalyzer::new()),
                    Some(map),
                )) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::Analyzer(
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer".into(),
        ))
        .arg(Arg::AnalyzerMap(vec![(
            "other".into(),
            "org.apache.lucene.analysis.core.SimpleAnalyzer".into(),
        )])),
    )
    .spec(
        Spec::analyzer(
            "per-field-mapped",
            "org.apache.lucene.analysis.miscellaneous.PerFieldAnalyzerWrapper",
            || {
                let mut map: JavaHashMap<Arc<dyn Analyzer>> = JavaHashMap::new();
                map.insert(
                    analysis_oracle::FIELD.to_string(),
                    Arc::new(SimpleAnalyzer::new()),
                );
                Ok(Box::new(PerFieldAnalyzerWrapper::new_with_fields(
                    Arc::new(WhitespaceAnalyzer::new()),
                    Some(map),
                )) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::Analyzer(
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer".into(),
        ))
        .arg(Arg::AnalyzerMap(vec![(
            analysis_oracle::FIELD.into(),
            "org.apache.lucene.analysis.core.SimpleAnalyzer".into(),
        )])),
    );
    assert_oracle(&o);
}

/// rmp #451, `analysis.miscellaneous` part 2: `concatenateGraph`,
/// `dateRecognizer`, `fingerprint`, `protectedTerm` (the concrete
/// `ConditionalTokenFilterFactory`; `conditional` is abstract in Lucene and
/// has no SPI name), `stemmerOverride`, `wordDelimiter` and
/// `wordDelimiterGraph`, each with its documented parameters.
#[test]
fn oracle_451_miscellaneous() {
    let dir = config_dir("451");
    let wdf_flags: &[(&str, &str)] = &[
        ("generateWordParts", "1"),
        ("generateNumberParts", "1"),
        ("catenateWords", "1"),
        ("catenateNumbers", "1"),
        ("catenateAll", "1"),
        ("splitOnCaseChange", "1"),
        ("splitOnNumerics", "1"),
        ("preserveOriginal", "1"),
        ("stemEnglishPossessive", "1"),
    ];
    let wdf_off: &[(&str, &str)] = &[
        ("generateWordParts", "0"),
        ("generateNumberParts", "1"),
        ("catenateWords", "0"),
        ("catenateNumbers", "1"),
        ("catenateAll", "0"),
        ("splitOnCaseChange", "0"),
        ("splitOnNumerics", "0"),
        ("preserveOriginal", "0"),
        ("stemEnglishPossessive", "0"),
        ("protected", "protwords.txt"),
        ("types", "wdftypes.txt"),
    ];
    let mut o = Oracle::new(task_corpus());
    o.spec(ws("concatenate-graph", &dir, &[("concatenateGraph", &[])]))
        .spec(ws(
            "concatenate-graph-sep",
            &dir,
            &[(
                "concatenateGraph",
                &[
                    ("tokenSeparator", "-"),
                    ("preservePositionIncrements", "false"),
                ],
            )],
        ))
        .spec(ws(
            "concatenate-graph-nosep",
            &dir,
            &[
                ("stop", &[("words", "words.txt")]),
                ("concatenateGraph", &[("tokenSeparator", "")]),
            ],
        ))
        .spec(ws(
            "concatenate-graph-max-expansions",
            &dir,
            &[
                (
                    "wordDelimiterGraph",
                    &[("catenateAll", "1"), ("preserveOriginal", "1")],
                ),
                ("concatenateGraph", &[("maxGraphExpansions", "2")]),
            ],
        ))
        .spec(ws(
            "concatenate-graph-graph-input",
            &dir,
            &[
                ("wordDelimiterGraph", &[("catenateAll", "1")]),
                ("concatenateGraph", &[("maxGraphExpansions", "100")]),
            ],
        ))
        .spec(kw(
            "date-recognizer-default",
            &dir,
            &[("dateRecognizer", &[])],
        ))
        .spec(ws(
            "date-recognizer-pattern",
            &dir,
            &[("dateRecognizer", &[("datePattern", "MM/dd/yyyy")])],
        ))
        .spec(kw(
            "date-recognizer-de",
            &dir,
            &[("dateRecognizer", &[("locale", "de-DE")])],
        ))
        .spec(kw(
            "date-recognizer-fr-pattern",
            &dir,
            &[(
                "dateRecognizer",
                &[("locale", "fr"), ("datePattern", "d MMM yyyy")],
            )],
        ))
        .spec(kw(
            "date-recognizer-ja-japanese",
            &dir,
            &[("dateRecognizer", &[("locale", "ja-JP-u-ca-japanese")])],
        ))
        .spec(ws("fingerprint", &dir, &[("fingerprint", &[])]))
        .spec(ws(
            "fingerprint-params",
            &dir,
            &[(
                "fingerprint",
                &[("maxOutputTokenSize", "20"), ("separator", "_")],
            )],
        ))
        .spec(ws(
            "protected-term",
            &dir,
            &[(
                "protectedTerm",
                &[
                    ("protected", "words.txt"),
                    ("ignoreCase", "true"),
                    ("wrappedFilters", "lowercase, truncate"),
                    ("truncate.prefixLength", "2"),
                ],
            )],
        ))
        .spec(ws(
            "protected-term-ids",
            &dir,
            &[(
                "protectedTerm",
                &[
                    ("protected", "words.txt, protected-1.txt"),
                    ("wrappedFilters", "truncate-A, reversestring, truncate-B"),
                    ("truncate-A.prefixLength", "3"),
                    ("truncate-B.prefixLength", "2"),
                ],
            )],
        ))
        .spec(ws(
            "stemmer-override",
            &dir,
            &[
                ("stemmerOverride", &[("dictionary", "stemdict.txt")]),
                ("porterStem", &[]),
            ],
        ))
        .spec(ws(
            "stemmer-override-ic",
            &dir,
            &[(
                "stemmerOverride",
                &[("dictionary", "stemdict.txt"), ("ignoreCase", "true")],
            )],
        ))
        .spec(ws(
            "word-delimiter-default",
            &dir,
            &[("wordDelimiter", &[])],
        ))
        .spec(ws(
            "word-delimiter-all",
            &dir,
            &[("wordDelimiter", wdf_flags)],
        ))
        .spec(ws(
            "word-delimiter-protected-types",
            &dir,
            &[("wordDelimiter", wdf_off)],
        ))
        .spec(ws(
            "word-delimiter-graph-default",
            &dir,
            &[("wordDelimiterGraph", &[])],
        ))
        .spec(ws(
            "word-delimiter-graph-all",
            &dir,
            &[("wordDelimiterGraph", wdf_flags)],
        ))
        .spec(ws(
            "word-delimiter-graph-protected-types",
            &dir,
            &[("wordDelimiterGraph", wdf_off)],
        ))
        .spec(ws(
            "word-delimiter-graph-no-adjust-keywords",
            &dir,
            &[
                ("keywordMarker", &[("protected", "protwords.txt")]),
                (
                    "wordDelimiterGraph",
                    &[
                        ("adjustOffsets", "false"),
                        ("ignoreKeywords", "1"),
                        ("catenateWords", "1"),
                    ],
                ),
            ],
        ))
        .spec(Spec::custom(
            "custom-protected-term",
            Chain::new()
                .config_dir(dir)
                .tokenizer("standard", &[])
                .filter(
                    "protectedTerm",
                    &[("protected", "words.txt"), ("wrappedFilters", "uppercase")],
                )
                .filter("fingerprint", &[]),
        ));
    assert_oracle(&o);
}
