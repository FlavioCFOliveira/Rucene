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

//! Java-oracle portability tests of the packages `analysis.cjk` and
//! `analysis.th` (rmp #467), `analysis.en` (#452) and `analysis.synonym` /
//! `analysis.synonym.word2vec` (#453): every analyzer, tokenizer, filter,
//! char filter and factory of each task, with parameterisations of each
//! factory's documented parameters (all 16 `CJKBigramFilter` script flag
//! combinations with and without `outputUnigrams`; the Solr and WordNet
//! synonym formats with `expand`, `ignoreCase`, `tokenizerFactory` and
//! `analyzer`; the word2vec factory over Lucene's test model), analyses the
//! same corpus on Lucene 10.5.0 and Rucene through the harness of #446
//! ([`analysis_oracle`]), and every token attribute is compared. The corpus
//! is the harness's standard one (fixed edge cases and seeded random
//! inputs) plus inputs that reach each task's filters.
//!
//! **Thai.** `ThaiTokenizer` splits Thai runs with a dictionary; the port
//! uses the ICU 79.1 word list where the JDK uses its own (the divergence
//! approved on rmp #447). The Thai specs are therefore asserted equal on the
//! inputs that hold no Thai character, and measured (not asserted) on the
//! inputs that do: [`oracle_467_thai_measurement`] prints how many differ.

mod analysis_oracle;
mod support;

use std::path::{Path, PathBuf};

use analysis_oracle::{corpus, Arg, Chain, Oracle, Spec};
use rucene::analysis::{Analyzer, CharArraySet};
use rucene_analysis_common::analysis::cjk::cjk_analyzer::CJKAnalyzer;
use rucene_analysis_common::analysis::en::english_analyzer::EnglishAnalyzer;
use rucene_analysis_common::analysis::th::thai_analyzer::ThaiAnalyzer;

/// The seed of the random half of the corpus.
const SEED: i64 = 0x0452_0467;

/// The number of random inputs.
const RANDOM_INPUTS: usize = 200;

/// Inputs that reach the filters of the three tasks beyond the standard
/// corpus.
const TASK_INPUTS: &[&str] = &[
    "一二三四五六七八九十 一 二三四 五六七八九 十",
    "あいうえおabcかきくけこ あいうえおabんcかきくけ こ",
    "多くの学生が試験に落ちた。 𩬅艱鍟䇹愯瀛 一丁丂",
    "한국어 텍스트입니다 カタカナ ひらがな 漢字 ﾊﾟﾅｿﾆｯｸ ｳﾞｨｯﾂ ｶﾀｶﾅ ｱﾞｨｯﾂ ﾞｨｯﾂ ﾟﾅｿﾆｯｸ",
    "Ｔｅｓｔ １２３４ Ｔｅst １２34 ＡＢＣ ｱｲｳ ﾜｦﾝ ｰ ｡｢｣､･",
    "一 روبرت موير 一 رُوبرت موير",
    "books book the steven's steven\u{2019}s steven\u{FF07}s queries phrases corpus stress kings",
    "panels aerodynamics congress serious bricks dogs yourselves yours caresses ponies ties caress cats",
    "relational conditional rational valenci hesitanci digitizer conformabli radicalli differentli",
    "vileli analogousli vietnamization predication operator feudalism decisiveness hopefulness",
    "callousness formaliti sensitiviti sensibiliti triplicate formative formalize electriciti electrical",
    "hopeful goodness revival allowance inference airliner gyroscopic adjustable defensible irritant",
    "replacement adjustment dependent adoption homologou communism activate angulariti homologous",
    "effective bowdlerize probate rate cease controll roll generalizations oscillators traditional",
    "running ran runs runner easily fairly abilities happiness organization organizing organized",
    "Lost in the woods wood forest king baron king's evil meany GB gib gigabyte gigabytes MB TV TVs",
    "i-pod ipod ipoooood foo bar baz this test that testing spider man spiderman usa united states",
    "u s a united states of america national hockey league nhl pixima aaa bbb ccc a b c",
    "it i do to nt the that s he you is we what they said not but and know was did are have",
    "there a be all of want she for just think in can going like go time now one so how get good",
];

/// The corpus: the harness's standard corpus and [`TASK_INPUTS`].
fn task_corpus() -> Vec<Vec<u16>> {
    let mut c = corpus::standard(SEED, RANDOM_INPUTS);
    c.extend(TASK_INPUTS.iter().map(|s| s.encode_utf16().collect()));
    c
}

/// Whether `input` holds a character of the Thai block (U+0E00..U+0E7F).
fn has_thai(input: &[u16]) -> bool {
    input.iter().any(|&u| (0x0E00..=0x0E7F).contains(&u))
}

/// A directory holding the resource files the specs name.
fn config_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rucene-oracle-t3-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the config dir");
    for (resource, bytes) in support::TEST_RESOURCES {
        let file = resource.rsplit('/').next().expect("a file name");
        std::fs::write(dir.join(file), bytes).expect("write a test resource");
    }
    let files: [(&str, &str); 5] = [
        (
            "solr-syns.txt",
            "# Solr format\n\
             the, a\n\
             quick, fast, speedy\n\
             fox => dog cat\n\
             caf\u{e9}, coffee\n\
             e mail, email\n\
             wi fi => wifi\n\
             new york => ny\n\
             lucene, apache lucene, search library\n\
             it, that\n\
             \u{4e2d}\u{6587}, chinese\n\
             king's evil => scrofula\n",
        ),
        (
            "wordnet-syns.txt",
            "s(100000001,1,'woods',n,1,0).\n\
             s(100000001,2,'wood',n,1,0).\n\
             s(100000001,3,'forest',n,1,0).\n\
             s(100000002,1,'quick',n,1,0).\n\
             s(100000002,2,'fast',n,1,0).\n\
             s(100000003,1,'king''s evil',n,1,1).\n\
             s(100000003,2,'king''s meany',n,1,1).\n\
             s(100000004,1,'the',n,1,1).\n\
             s(100000004,2,'a',n,1,1).\n",
        ),
        ("stopwords.txt", "the\na\nis\n\u{4e00}\n"),
        ("protwords.txt", "books\nrunning\nsteven's\n"),
        ("stemdict.txt", "dogs\tcat\nbooks\tlibrary\n"),
    ];
    for (file, content) in files {
        std::fs::write(dir.join(file), content).expect("write a config file");
    }
    dir
}

/// A factories spec over the tokenizer `tokenizer` and `filters`.
fn chain(id: &str, dir: &Path, tokenizer: &str, filters: &[(&str, &[(&str, &str)])]) -> Spec {
    let mut chain = Chain::new().config_dir(dir).tokenizer(tokenizer, &[]);
    for (name, params) in filters {
        chain = chain.filter(name, params);
    }
    Spec::factories(id, chain)
}

/// Runs the oracle and checks that Lucene built every spec.
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
    if let Err(report) = analysis_oracle::compare(&java, &o.rust_dump(), o) {
        panic!("{report}");
    }
}

// ---------------------------------------------------------------------------
// #467 cjk and th
// ---------------------------------------------------------------------------

#[test]
fn oracle_467_cjk() {
    let dir = config_dir("cjk");
    let mut o = Oracle::new(task_corpus());
    o.spec(Spec::analyzer(
        "cjk-analyzer",
        "org.apache.lucene.analysis.cjk.CJKAnalyzer",
        || Ok(Box::new(CJKAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "cjk-analyzer-stop",
            "org.apache.lucene.analysis.cjk.CJKAnalyzer",
            || {
                Ok(
                    Box::new(CJKAnalyzer::with_stop_words(CharArraySet::from_iter(
                        ["\u{4e00}\u{4e8c}", "the", "abc"],
                        false,
                    ))) as Box<dyn Analyzer>,
                )
            },
        )
        .arg(Arg::CharSet(vec![
            "\u{4e00}\u{4e8c}".into(),
            "the".into(),
            "abc".into(),
        ])),
    );
    // Every script-flag combination of `CJKBigramFilterFactory`, with and
    // without `outputUnigrams`.
    for flags in 0..16 {
        for unigrams in [false, true] {
            let b = |bit: i32| if flags & bit != 0 { "true" } else { "false" };
            let params: [(&str, &str); 5] = [
                ("han", b(1)),
                ("hiragana", b(2)),
                ("katakana", b(4)),
                ("hangul", b(8)),
                ("outputUnigrams", if unigrams { "true" } else { "false" }),
            ];
            o.spec(chain(
                &format!("cjkbigram-{flags}-{unigrams}"),
                &dir,
                "standard",
                &[("cjkBigram", &params)],
            ));
        }
    }
    o.spec(chain(
        "cjkbigram-default",
        &dir,
        "standard",
        &[("cjkBigram", &[])],
    ))
    // over a tokenizer that sets no CJK types: the filter passes tokens through
    .spec(chain(
        "cjkbigram-whitespace",
        &dir,
        "whitespace",
        &[("cjkBigram", &[])],
    ))
    .spec(chain(
        "cjkwidth-filter",
        &dir,
        "whitespace",
        &[("cjkWidth", &[])],
    ))
    .spec(chain(
        "cjkwidth-filter-standard",
        &dir,
        "standard",
        &[
            ("cjkWidth", &[]),
            ("cjkBigram", &[("outputUnigrams", "true")]),
        ],
    ))
    .spec(Spec::factories(
        "cjkwidth-charfilter",
        Chain::new()
            .config_dir(&dir)
            .char_filter("cjkWidth", &[])
            .tokenizer("whitespace", &[]),
    ))
    .spec(Spec::factories(
        "cjkwidth-charfilter-bigram",
        Chain::new()
            .config_dir(&dir)
            .char_filter("cjkWidth", &[])
            .tokenizer("standard", &[])
            .filter("cjkBigram", &[]),
    ))
    .spec(Spec::custom(
        "cjk-custom",
        Chain::new()
            .config_dir(&dir)
            .char_filter("cjkWidth", &[])
            .tokenizer("standard", &[])
            .filter("lowercase", &[])
            .filter("cjkBigram", &[("hangul", "false")]),
    ));
    assert_oracle(&o);
}

/// The Thai specs (`ThaiAnalyzer` and the `thai` tokenizer factory).
fn thai_specs(o: &mut Oracle, dir: &Path) {
    o.spec(Spec::analyzer(
        "thai-analyzer",
        "org.apache.lucene.analysis.th.ThaiAnalyzer",
        || Ok(Box::new(ThaiAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "thai-analyzer-nostop",
            "org.apache.lucene.analysis.th.ThaiAnalyzer",
            || {
                Ok(
                    Box::new(ThaiAnalyzer::with_stop_words(CharArraySet::from_iter(
                        Vec::<&str>::new(),
                        false,
                    ))) as Box<dyn Analyzer>,
                )
            },
        )
        .arg(Arg::CharSet(vec![])),
    )
    .spec(chain("thai-tokenizer", dir, "thai", &[]))
    .spec(chain(
        "thai-tokenizer-lower",
        dir,
        "thai",
        &[("lowercase", &[]), ("decimalDigit", &[])],
    ));
}

#[test]
fn oracle_467_thai_non_thai_inputs() {
    let dir = config_dir("th");
    let corpus: Vec<Vec<u16>> = task_corpus().into_iter().filter(|i| !has_thai(i)).collect();
    assert!(
        corpus.len() > 200,
        "the non-Thai corpus holds {} inputs",
        corpus.len()
    );
    let mut o = Oracle::new(corpus);
    thai_specs(&mut o, &dir);
    assert_oracle(&o);
}

/// The `input` sections of `dump`, per spec: `(spec, input, lines)`.
fn input_sections(dump: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut spec = String::new();
    let mut input: Option<(String, String)> = None;
    for line in dump.lines() {
        if let Some(id) = line.strip_prefix("spec ") {
            if let Some((i, l)) = input.take() {
                out.push((spec.clone(), i, l));
            }
            spec = id.to_string();
        } else if let Some(i) = line.strip_prefix("input ") {
            if let Some((i, l)) = input.take() {
                out.push((spec.clone(), i, l));
            }
            input = Some((i.to_string(), String::new()));
        } else if let Some((_, l)) = input.as_mut() {
            l.push_str(line);
            l.push('\n');
        }
    }
    if let Some((i, l)) = input.take() {
        out.push((spec, i, l));
    }
    out
}

/// Measures (does not assert) the Thai specs on the inputs that hold Thai
/// text: the dictionary data differs by the approved divergence of #447.
/// Prints, per spec, the inputs whose token dumps differ.
#[test]
fn oracle_467_thai_measurement() {
    let dir = config_dir("th-measure");
    let corpus: Vec<Vec<u16>> = task_corpus().into_iter().filter(|i| has_thai(i)).collect();
    let mut o = Oracle::new(corpus.clone());
    thai_specs(&mut o, &dir);
    let java = o.java_dump();
    o.assert_covers(&java);
    let rust = o.rust_dump();
    let j = input_sections(&java);
    let r = input_sections(&rust);
    assert_eq!(j.len(), r.len(), "the dumps hold the same sections");
    let mut differing = 0;
    for ((spec, input, jl), (_, _, rl)) in j.iter().zip(&r) {
        if jl != rl {
            differing += 1;
            println!("THAI-DIFF spec={spec} input={input}");
        }
    }
    println!(
        "THAI-MEASURE inputs={} sections={} differing={}",
        corpus.len(),
        j.len(),
        differing
    );
}

// ---------------------------------------------------------------------------
// #452 en
// ---------------------------------------------------------------------------

#[test]
fn oracle_452_en() {
    let dir = config_dir("en");
    let mut o = Oracle::new(task_corpus());
    o.spec(Spec::analyzer(
        "english-analyzer",
        "org.apache.lucene.analysis.en.EnglishAnalyzer",
        || Ok(Box::new(EnglishAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "english-analyzer-stop",
            "org.apache.lucene.analysis.en.EnglishAnalyzer",
            || {
                Ok(
                    Box::new(EnglishAnalyzer::with_stop_words(CharArraySet::from_iter(
                        ["is", "not", "books"],
                        false,
                    ))) as Box<dyn Analyzer>,
                )
            },
        )
        .arg(Arg::CharSet(vec![
            "is".into(),
            "not".into(),
            "books".into(),
        ])),
    )
    .spec(
        Spec::analyzer(
            "english-analyzer-exclusion",
            "org.apache.lucene.analysis.en.EnglishAnalyzer",
            || {
                Ok(Box::new(EnglishAnalyzer::with_stem_exclusion_set(
                    CharArraySet::from_iter(["the", "a"], true),
                    &CharArraySet::from_iter(["books", "running", "queries"], false),
                )) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::CharSetIgnoreCase(vec!["the".into(), "a".into()]))
        .arg(Arg::CharSet(vec![
            "books".into(),
            "running".into(),
            "queries".into(),
        ])),
    );
    for (name, filter) in [
        ("minimal", "englishMinimalStem"),
        ("possessive", "englishPossessive"),
        ("kstem", "kStem"),
        ("porter", "porterStem"),
    ] {
        o.spec(chain(
            &format!("{name}-ws"),
            &dir,
            "whitespace",
            &[(filter, &[])],
        ))
        .spec(chain(
            &format!("{name}-std-lower"),
            &dir,
            "standard",
            &[("lowercase", &[]), (filter, &[])],
        ))
        .spec(chain(
            &format!("{name}-keyword-marked"),
            &dir,
            "whitespace",
            &[
                ("lowercase", &[]),
                ("keywordMarker", &[("protected", "protwords.txt")]),
                (filter, &[]),
            ],
        ));
    }
    o.spec(chain(
        "en-full-chain",
        &dir,
        "standard",
        &[
            ("englishPossessive", &[]),
            ("lowercase", &[]),
            ("stop", &[("words", "stopwords.txt")]),
            ("stemmerOverride", &[("dictionary", "stemdict.txt")]),
            ("kStem", &[]),
            ("porterStem", &[]),
        ],
    ))
    .spec(Spec::custom(
        "en-custom",
        Chain::new()
            .config_dir(&dir)
            .tokenizer("standard", &[])
            .filter("englishPossessive", &[])
            .filter("lowercase", &[])
            .filter("englishMinimalStem", &[]),
    ));
    assert_oracle(&o);
}

// ---------------------------------------------------------------------------
// #453 synonym and synonym.word2vec
// ---------------------------------------------------------------------------

/// A synonym-factory variant: its id, tokenizer and parameters.
type Variant<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>);

#[test]
fn oracle_453_synonym() {
    let dir = config_dir("syn");
    let mut o = Oracle::new(task_corpus());
    for factory in ["synonym", "synonymGraph"] {
        let variants: [Variant; 9] = [
            ("solr", "whitespace", vec![("synonyms", "solr-syns.txt")]),
            (
                "solr-noexpand",
                "whitespace",
                vec![("synonyms", "solr-syns.txt"), ("expand", "false")],
            ),
            (
                "solr-ignorecase",
                "whitespace",
                vec![("synonyms", "solr-syns.txt"), ("ignoreCase", "true")],
            ),
            (
                "solr-lucene-file",
                "whitespace",
                vec![("synonyms", "synonyms.txt"), ("format", "solr")],
            ),
            (
                "wordnet",
                "whitespace",
                vec![("synonyms", "wordnet-syns.txt"), ("format", "wordnet")],
            ),
            (
                "wordnet-noexpand-ic",
                "whitespace",
                vec![
                    ("synonyms", "synonyms-wordnet.txt"),
                    ("format", "wordnet"),
                    ("expand", "false"),
                    ("ignoreCase", "true"),
                ],
            ),
            (
                "tokenizer-factory",
                "standard",
                vec![
                    ("synonyms", "solr-syns.txt"),
                    (
                        "tokenizerFactory",
                        "org.apache.lucene.analysis.core.WhitespaceTokenizerFactory",
                    ),
                    ("ignoreCase", "true"),
                ],
            ),
            (
                "analyzer",
                "standard",
                vec![
                    ("synonyms", "solr-syns.txt"),
                    ("analyzer", "org.apache.lucene.analysis.core.SimpleAnalyzer"),
                ],
            ),
            (
                "analyzer-cjk",
                "standard",
                vec![
                    ("synonyms", "synonyms2.txt"),
                    ("analyzer", "org.apache.lucene.analysis.cjk.CJKAnalyzer"),
                ],
            ),
        ];
        for (name, tokenizer, params) in &variants {
            o.spec(chain(
                &format!("{factory}-{name}"),
                &dir,
                tokenizer,
                &[(factory, params.as_slice())],
            ));
        }
        o.spec(chain(
            &format!("{factory}-lower"),
            &dir,
            "standard",
            &[
                ("lowercase", &[]),
                (factory, &[("synonyms", "solr-syns.txt")]),
            ],
        ));
    }
    o.spec(chain(
        "synonymGraph-flatten",
        &dir,
        "whitespace",
        &[
            ("synonymGraph", &[("synonyms", "solr-syns.txt")]),
            ("flattenGraph", &[]),
        ],
    ))
    .spec(Spec::custom(
        "synonym-custom",
        Chain::new()
            .config_dir(&dir)
            .tokenizer("standard", &[])
            .filter("lowercase", &[])
            .filter("synonymGraph", &[("synonyms", "solr-syns.txt")])
            .filter("flattenGraph", &[]),
    ));
    assert_oracle(&o);
}

#[test]
fn oracle_453_word2vec() {
    let dir = config_dir("w2v");
    let mut o = Oracle::new(task_corpus());
    for (name, params) in [
        ("default", vec![("model", "word2vec-model.zip")]),
        (
            "07-3",
            vec![
                ("model", "word2vec-model.zip"),
                ("minAcceptedSimilarity", "0.7"),
                ("maxSynonymsPerTerm", "3"),
            ],
        ),
        (
            "05-10-dl4j",
            vec![
                ("model", "word2vec-model.zip"),
                ("format", "dl4j"),
                ("minAcceptedSimilarity", "0.5"),
                ("maxSynonymsPerTerm", "10"),
            ],
        ),
    ] {
        o.spec(chain(
            &format!("word2vec-{name}-ws"),
            &dir,
            "whitespace",
            &[("Word2VecSynonym", params.as_slice())],
        ))
        .spec(chain(
            &format!("word2vec-{name}-std-lower"),
            &dir,
            "standard",
            &[("lowercase", &[]), ("Word2VecSynonym", params.as_slice())],
        ));
    }
    assert_oracle(&o);
}
