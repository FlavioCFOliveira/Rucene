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

//! Port of `org.apache.lucene.tests.util.TestRuleSetupAndRestoreClassEnv`:
//! "setup and restore suite-level environment (fine grained junk that
//! doesn't fit anywhere else)."
//!
//! **Divergences.**
//! * `Locale.setDefault` and `TimeZone.setDefault` set the port's default
//!   slots ([`crate::jdk::locale::set_default`],
//!   [`LuceneTestCase::set_default_time_zone`]); a Rust process has no JVM
//!   default to save, so the saved value of a slot never set is "unset", and
//!   `after()` restores that.
//! * `TimeZone.getTimeZone(tests.timezone)` resolves the IDs of
//!   `TimeZone.getAvailableIDs()`; any other ID gives `GMT`, as Java gives
//!   for an unknown ID, including the custom `GMT±hh:mm` IDs Java parses
//!   (the port's `TimeZone` holds an available ID only).
//! * `Codec.forName`, `PostingsFormat.forName` and `DocValuesFormat.forName`
//!   read the global registries, which `LuceneTestCase`'s class
//!   initialisation reloads with this crate's service providers
//!   (`lucene-test-framework.jar` on the class path).

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception;
use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Description;
use randomizedtesting::RandomizedContext;
use rucene::codecs::lucene104::codec::{Lucene104Codec, Mode};
use rucene::codecs::{Codec, DocValuesFormat, PostingsFormat};
use rucene::search::similarities::Similarity;
use rucene::util::java_collections::JavaHashSet;
use rucene::util::{InfoStream, PrintStreamInfoStream};
use rucene_codecs::simpletext::SimpleTextCodec;

use crate::codecs::asserting::{AssertingCodec, AssertingDocValuesFormat, AssertingPostingsFormat};
use crate::codecs::cheapbastard::CheapBastardCodec;
use crate::codecs::compressing::CompressingCodec;
use crate::codecs::mockrandom::MockRandomPostingsFormat;
use crate::index::RandomCodec;
use crate::jdk::locale::{self, Locale};
use crate::search::similarities::{AssertingSimilarity, RandomSimilarity};
use crate::util::abstract_before_after_rule::AbstractBeforeAfterRule;
use crate::util::lucene_test_case::{
    LiveIWCFlushMode, LuceneTestCase, SuppressCodecs, TimeZone, INFOSTREAM, TEST_CODEC,
    TEST_DOCVALUESFORMAT, TEST_POSTINGSFORMAT, VERBOSE,
};
use crate::util::null_info_stream::NullInfoStream;

/// Port of the nested class
/// `TestRuleSetupAndRestoreClassEnv.ThreadNameFixingPrintStreamInfoStream`: a
/// `PrintStreamInfoStream` over `System.out` that ignores test points and
/// names the test thread `main`.
pub struct ThreadNameFixingPrintStreamInfoStream {
    base: PrintStreamInfoStream,
}

impl ThreadNameFixingPrintStreamInfoStream {
    /// `new ThreadNameFixingPrintStreamInfoStream(System.out)`.
    pub fn new_system_out() -> ThreadNameFixingPrintStreamInfoStream {
        ThreadNameFixingPrintStreamInfoStream {
            base: PrintStreamInfoStream::system(Box::new(std::io::sink())),
        }
    }
}

impl InfoStream for ThreadNameFixingPrintStreamInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.TestRuleSetupAndRestoreClassEnv$ThreadNameFixingPrintStreamInfoStream"
    }

    /// `message(component, message)`: "ignore test points!" (`TP`); "The name
    /// of the main thread is way too long when looking at IW verbose output",
    /// so a `TEST-` thread is named `main`.
    fn message(&self, component: &str, message: &str) {
        if component == "TP" {
            return;
        }
        let current = Thread::current().name();
        let name = if current.starts_with("TEST-") {
            "main".to_string()
        } else {
            current
        };
        system::out_println(&format!(
            "{component} {} [{}; {name}]: {message}",
            self.base.message_id(),
            self.base.get_timestamp()
        ));
    }

    /// `PrintStreamInfoStream.isEnabled(component)`: always.
    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    /// `PrintStreamInfoStream.close()`: `System.out` is not closed.
    fn close(&self) {}
}

/// The anonymous `AssertingCodec` of `before()` for a user-wired postings or
/// doc-values format: every field gets `format` and `dvFormat`, and
/// `toString()` is `super.toString() + ": " + format + ", " + dvFormat`.
struct WiredAssertingCodec {
    asserting: AssertingCodec,
    format: Arc<dyn PostingsFormat>,
    dv_format: Arc<dyn DocValuesFormat>,
}

impl std::fmt::Debug for WiredAssertingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

impl Codec for WiredAssertingCodec {
    fn name(&self) -> &str {
        self.asserting.name()
    }
    fn postings_format(&self) -> &dyn PostingsFormat {
        self.asserting.postings_format()
    }
    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.asserting.doc_values_format()
    }
    fn stored_fields_format(&self) -> &dyn rucene::codecs::StoredFieldsFormat {
        self.asserting.stored_fields_format()
    }
    fn term_vectors_format(&self) -> &dyn rucene::codecs::TermVectorsFormat {
        self.asserting.term_vectors_format()
    }
    fn field_infos_format(&self) -> &dyn rucene::codecs::FieldInfosFormat {
        self.asserting.field_infos_format()
    }
    fn segment_info_format(&self) -> &dyn rucene::codecs::SegmentInfoFormat {
        self.asserting.segment_info_format()
    }
    fn norms_format(&self) -> &dyn rucene::codecs::NormsFormat {
        self.asserting.norms_format()
    }
    fn live_docs_format(&self) -> &dyn rucene::codecs::LiveDocsFormat {
        self.asserting.live_docs_format()
    }
    fn compound_format(&self) -> &dyn rucene::codecs::CompoundFormat {
        self.asserting.compound_format()
    }
    fn points_format(&self) -> &dyn rucene::codecs::PointsFormat {
        self.asserting.points_format()
    }
    fn knn_vectors_format(&self) -> &dyn rucene::codecs::KnnVectorsFormat {
        self.asserting.knn_vectors_format()
    }
    fn to_java_string(&self) -> String {
        format!(
            "{}: {}, {}",
            self.asserting,
            self.format.to_java_string(),
            &*self.dv_format as &dyn DocValuesFormat
        )
    }
}

/// The fields of the rule; the static `classEnvRule` holds one instance for
/// every suite, as Java's does.
#[derive(Default)]
pub struct ClassEnv {
    saved_codec: Option<Arc<dyn Codec>>,
    saved_locale: Option<Option<Locale>>,
    saved_time_zone: Option<Option<TimeZone>>,
    saved_info_stream: Option<Arc<dyn InfoStream>>,
    /// `locale`.
    pub locale: Option<Locale>,
    /// `timeZone`.
    pub time_zone: Option<TimeZone>,
    /// `similarity`.
    pub similarity: Option<Arc<AssertingSimilarity>>,
    /// `codec`.
    pub codec: Option<Arc<dyn Codec>>,
    /// "Indicates whether the rule has executed its `before()` method fully."
    pub initialized: bool,
    /// `avoidCodecs` (see `SuppressCodecs`).
    pub avoid_codecs: HashSet<String>,
    /// `avoidCodecs` as Java's `HashSet` iterates it (`toArray()`).
    avoid_codecs_java: JavaHashSet,
    /// The `RandomCodec` the codec is, for `checkCodecRestrictions`'
    /// `codec instanceof RandomCodec`.
    random_codec: Option<Arc<RandomCodec>>,
}

impl ClassEnv {
    /// `shouldAvoidCodec(codec)`: "should a given codec be avoided for the
    /// currently executing suite?"
    fn should_avoid_codec(&self, codec: &str) -> bool {
        !self.avoid_codecs.is_empty() && self.avoid_codecs.contains(codec)
    }
}

/// Port of `TestRuleSetupAndRestoreClassEnv`.
#[derive(Default)]
pub struct TestRuleSetupAndRestoreClassEnv {
    env: Mutex<ClassEnv>,
}

impl TestRuleSetupAndRestoreClassEnv {
    /// `new TestRuleSetupAndRestoreClassEnv()`.
    pub fn new() -> TestRuleSetupAndRestoreClassEnv {
        TestRuleSetupAndRestoreClassEnv::default()
    }

    /// The rule's fields.
    pub fn env(&self) -> MutexGuard<'_, ClassEnv> {
        self.env
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `isInitialized()`.
    pub fn is_initialized(&self) -> bool {
        self.env().initialized
    }

    /// The similarity the rule drew (`similarity`), as `newSearcher` installs it.
    pub fn similarity(&self) -> Option<Arc<dyn Similarity>> {
        self.env()
            .similarity
            .clone()
            .map(|s| s as Arc<dyn Similarity>)
    }

    /// `checkCodecRestrictions(codec)`: "check codec restrictions."
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` "if the class does not work with a given
    /// codec."
    fn check_codec_restrictions(env: &ClassEnv, codec: &dyn Codec) {
        LuceneTestCase::assume_false(
            &format!("Class not allowed to use codec: {}.", codec.name()),
            env.should_avoid_codec(codec.name()),
        );
        if let Some(random_codec) = &env.random_codec {
            if !env.avoid_codecs.is_empty() {
                for name in random_codec.format_names.iter() {
                    LuceneTestCase::assume_false(
                        &format!("Class not allowed to use postings format: {name}."),
                        env.should_avoid_codec(name),
                    );
                }
            }
        }
        let pf = codec.postings_format();
        LuceneTestCase::assume_false(
            &format!("Class not allowed to use postings format: {}.", pf.name()),
            env.should_avoid_codec(pf.name()),
        );
        LuceneTestCase::assume_false(
            &format!(
                "Class not allowed to use postings format: {}.",
                TEST_POSTINGSFORMAT.as_str()
            ),
            env.should_avoid_codec(&TEST_POSTINGSFORMAT),
        );
    }
}

/// `Codec.forName(name)`.
fn codec_for_name(name: &str) -> Arc<dyn Codec> {
    rucene::codecs::for_name(name).unwrap_or_else(|| {
        spi_missing(
            "org.apache.lucene.codecs.Codec",
            name,
            &rucene::codecs::available_codecs(),
        )
    })
}

/// `PostingsFormat.forName(name)`.
fn postings_for_name(name: &str) -> Arc<dyn PostingsFormat> {
    rucene::codecs::postings::postings_for_name(name).unwrap_or_else(|| {
        spi_missing(
            "org.apache.lucene.codecs.PostingsFormat",
            name,
            &rucene::codecs::postings::available_postings_formats(),
        )
    })
}

/// `DocValuesFormat.forName(name)`.
fn doc_values_for_name(name: &str) -> Arc<dyn DocValuesFormat> {
    rucene::codecs::doc_values::doc_values_for_name(name).unwrap_or_else(|| {
        spi_missing(
            "org.apache.lucene.codecs.DocValuesFormat",
            name,
            &rucene::codecs::doc_values::available_doc_values_formats(),
        )
    })
}

/// `NamedSPILoader.lookup`'s `IllegalArgumentException`.
fn spi_missing(clazz: &str, name: &str, available: &[String]) -> ! {
    exception::illegal_argument(format!(
        "An SPI class of type {clazz} with name '{name}' does not exist.  You need to add the \
         corresponding JAR file supporting this SPI to your classpath.  The current classpath \
         supports the following names: [{}]",
        available.join(", ")
    ))
}

impl AbstractBeforeAfterRule for TestRuleSetupAndRestoreClassEnv {
    /// `before()`.
    fn before(&self, _description: &Arc<Description>) {
        // if verbose: print some debugging stuff about which codecs are loaded.
        if *VERBOSE {
            system::out_println(&format!(
                "Loaded codecs: [{}]",
                rucene::codecs::available_codecs().join(", ")
            ));
            system::out_println(&format!(
                "Loaded postingsFormats: [{}]",
                rucene::codecs::postings::available_postings_formats().join(", ")
            ));
        }

        let mut env = self.env();
        env.saved_info_stream = Some(rucene::util::default_info_stream());
        let context = RandomizedContext::current();
        let random = context.get_random();
        let v = random.next_boolean();
        if *INFOSTREAM {
            rucene::util::set_default_info_stream(Arc::new(
                ThreadNameFixingPrintStreamInfoStream::new_system_out(),
            ));
        } else if v {
            rucene::util::set_default_info_stream(Arc::new(NullInfoStream));
        }

        let Some(target_class) = context.get_target_class_object() else {
            exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
        };
        env.avoid_codecs = HashSet::new();
        env.avoid_codecs_java = JavaHashSet::new();
        if let Some(a) = target_class.get_annotation::<SuppressCodecs>() {
            env.avoid_codecs.extend(a.value.iter().cloned());
            for name in &a.value {
                env.avoid_codecs_java.insert(name.clone());
            }
        }

        env.saved_codec = rucene::codecs::get_default();
        env.random_codec = None;
        let random_val = random.next_int_bound(11);
        let test_codec = TEST_CODEC.as_str();
        let codec: Arc<dyn Codec> = if test_codec == "default" {
            // just use the default, don't randomize
            env.saved_codec.clone().unwrap_or_else(|| {
                exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
            })
        } else if TEST_POSTINGSFORMAT.as_str() != "random"
            || TEST_DOCVALUESFORMAT.as_str() != "random"
        {
            // the user wired postings or DV: this is messy
            // refactor into RandomCodec....
            let format: Arc<dyn PostingsFormat> = if TEST_POSTINGSFORMAT.as_str() == "random" {
                Arc::new(AssertingPostingsFormat::new())
            } else if TEST_POSTINGSFORMAT.as_str() == "MockRandom" {
                Arc::new(MockRandomPostingsFormat::with_random(&JdkRandom::new(
                    random.next_long(),
                )))
            } else {
                postings_for_name(&TEST_POSTINGSFORMAT)
            };
            let dv_format: Arc<dyn DocValuesFormat> = if TEST_DOCVALUESFORMAT.as_str() == "random" {
                Arc::new(AssertingDocValuesFormat::new())
            } else {
                doc_values_for_name(&TEST_DOCVALUESFORMAT)
            };
            let (f, d) = (Arc::clone(&format), Arc::clone(&dv_format));
            Arc::new(WiredAssertingCodec {
                asserting: AssertingCodec::with_formats_for_field(
                    Some(Arc::new(move |_field: &str| Arc::clone(&f))),
                    Some(Arc::new(move |_field: &str| Arc::clone(&d))),
                    None,
                ),
                format,
                dv_format,
            })
        } else if test_codec == "SimpleText"
            || (test_codec == "random"
                && random_val == 9
                && LuceneTestCase::rarely_with(&*random)
                && !env.should_avoid_codec("SimpleText"))
        {
            Arc::new(SimpleTextCodec::new())
        } else if test_codec == "CheapBastard"
            || (test_codec == "random"
                && random_val == 8
                && !env.should_avoid_codec("CheapBastard")
                && !env.should_avoid_codec("Lucene41"))
        {
            // we also avoid this codec if Lucene41 is avoided, since thats the postings format it uses.
            Arc::new(CheapBastardCodec::new())
        } else if test_codec == "Asserting"
            || (test_codec == "random" && random_val == 7 && !env.should_avoid_codec("Asserting"))
        {
            Arc::new(AssertingCodec::new())
        } else if test_codec == "Compressing"
            || (test_codec == "random" && random_val == 6 && !env.should_avoid_codec("Compressing"))
        {
            match CompressingCodec::random_instance(&*random) {
                Ok(c) => Arc::new(c),
                Err(e) => exception::illegal_argument(e),
            }
        } else if test_codec == "Lucene104"
            || (test_codec == "random" && random_val == 5 && !env.should_avoid_codec("Lucene104"))
        {
            let mode =
                *RandomPicks::random_from(&*random, &[Mode::BestSpeed, Mode::BestCompression]);
            Arc::new(
                Lucene104Codec::with_mode(Some(mode))
                    .expect("INVARIANT: a mode picked from Mode.values() is not null"),
            )
        } else if test_codec != "random" {
            codec_for_name(test_codec)
        } else if TEST_POSTINGSFORMAT.as_str() == "random" {
            let random_codec = Arc::new(RandomCodec::new_avoiding(
                &*random,
                env.avoid_codecs.clone(),
            ));
            env.random_codec = Some(Arc::clone(&random_codec));
            random_codec
        } else {
            exception::java_assert(false, String::new);
            unreachable!("assert false")
        };
        rucene::codecs::set_default(Arc::clone(&codec));
        env.codec = Some(Arc::clone(&codec));

        // Initialize locale/ timezone.
        let test_locale =
            system::get_property("tests.locale").unwrap_or_else(|| "random".to_string());
        let test_time_zone =
            system::get_property("tests.timezone").unwrap_or_else(|| "random".to_string());

        // Always pick a random one for consistency (whether tests.locale was specified or not).
        env.saved_locale = Some(locale::get_default());
        let random_locale = LuceneTestCase::random_locale(&*random);
        let chosen_locale = if test_locale == "random" {
            random_locale
        } else {
            LuceneTestCase::locale_for_language_tag(&test_locale)
        };
        locale::set_default(Some(chosen_locale.clone()));
        env.locale = Some(chosen_locale);

        env.saved_time_zone = Some(LuceneTestCase::get_default_time_zone());
        let random_time_zone = LuceneTestCase::random_time_zone(&*LuceneTestCase::random());
        let time_zone = if test_time_zone == "random" {
            random_time_zone
        } else {
            TimeZone::get_time_zone(&test_time_zone)
        };
        LuceneTestCase::set_default_time_zone(Some(time_zone));
        env.time_zone = Some(time_zone);
        env.similarity = Some(Arc::new(AssertingSimilarity::new(Arc::new(
            RandomSimilarity::new(&*LuceneTestCase::random()),
        ))));

        // Check codec restrictions once at class level.
        let check = crate::util::abstract_before_after_rule::try_run(|| {
            Self::check_codec_restrictions(&env, &*codec)
        });
        if let Err(e) = check {
            if e.is_instance(randomizedtesting::ASSUMPTION_VIOLATED_EXCEPTION) {
                let avoided: Vec<&str> = env.avoid_codecs_java.iter().map(|s| s.as_str()).collect();
                system::err_println(&format!(
                    "NOTE: {} Suppressed codecs: [{}]",
                    e.message().unwrap_or("null"),
                    avoided.join(", ")
                ));
            }
            drop(env);
            e.throw();
        }

        // We have "stickiness" so that sometimes all we do is vary the RAM buffer size, other times
        // just the doc count to flush by, else both.
        // This way the assertMemory in DocumentsWriterFlushControl sometimes runs (when we always flush
        // by RAM).
        let flush_mode = match LuceneTestCase::random().next_int_bound(3) {
            0 => LiveIWCFlushMode::ByRam,
            1 => LiveIWCFlushMode::ByDocs,
            2 => LiveIWCFlushMode::Either,
            _ => exception::throw_without_message(exception::ASSERTION_ERROR),
        };
        LuceneTestCase::set_live_iwc_flush_mode(Some(flush_mode));

        env.initialized = true;
    }

    /// `after()`: "after suite cleanup (always invoked)."
    fn after(&self) {
        let mut env = self.env();
        if let Some(codec) = env.saved_codec.take() {
            rucene::codecs::set_default(codec);
        }
        if let Some(info_stream) = env.saved_info_stream.take() {
            rucene::util::set_default_info_stream(info_stream);
        }
        if let Some(saved) = env.saved_locale.take() {
            locale::set_default(saved);
        }
        if let Some(saved) = env.saved_time_zone.take() {
            LuceneTestCase::set_default_time_zone(saved);
        }
    }
}
