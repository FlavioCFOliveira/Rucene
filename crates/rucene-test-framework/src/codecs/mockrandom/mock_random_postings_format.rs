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

//! Port of `org.apache.lucene.tests.codecs.mockrandom.MockRandomPostingsFormat`.

use std::fmt;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::codecs::codec_util::{
    check_footer, check_index_header, write_footer, write_index_header,
};
use rucene::codecs::{
    FieldsConsumer, FieldsProducer, Lucene103BlockTreeTermsReader, Lucene103BlockTreeTermsWriter,
    Lucene104PostingsReader, Lucene104PostingsWriter, PostingsFormat, PostingsReaderBase,
    PostingsWriterBase, SegmentReadState, SegmentWriteState, TermStats,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::FieldInfo;
use rucene::store::{DataInput, IndexInput};
use rucene::util::BytesRef;
use rucene_codecs::blockterms::{
    BlockTermsReader, BlockTermsWriter, EveryNOrDocFreqTermSelector, EveryNTermSelector,
    FixedGapTermsIndexReader, FixedGapTermsIndexWriter, IndexTermSelector, TermsIndexReaderBase,
    TermsIndexWriterBase, VariableGapTermsIndexReader, VariableGapTermsIndexWriter,
};
use rucene_codecs::blocktreeords::{OrdsBlockTreeTermsReader, OrdsBlockTreeTermsWriter};
use rucene_codecs::memory::{FSTTermsReader, FSTTermsWriter};

use crate::util::lucene_test_case::VERBOSE;
use crate::util::TestUtil;

const SEED_EXT: &str = "sd";

/// Randomly combines terms index impl w/ postings impls.
///
/// Port of `MockRandomPostingsFormat` (`final`, `extends PostingsFormat`,
/// named `MockRandom`). Each segment it writes draws, from the format's own
/// random, a seed it records in `<segment>_<suffix>.sd` (header
/// `MockRandomSeed`, version 0); that seed then chooses the terms dictionary
/// (`FSTTermsWriter`, `Lucene103BlockTreeTermsWriter`, `BlockTermsWriter`
/// with a fixed-gap or variable-gap terms index, or
/// `OrdsBlockTreeTermsWriter`) and its settings, over
/// `Lucene104PostingsWriter`; the reader replays the seed to pick the same
/// readers.
pub struct MockRandomPostingsFormat {
    /// `seedRandom`; `None` for the read-time format of the no-argument
    /// constructor, whose Java random throws `IllegalStateException("Please
    /// use MockRandomPostingsFormat(Random)")` on its first draw.
    seed_random: Option<JdkRandom>,
}

impl MockRandomPostingsFormat {
    /// `new MockRandomPostingsFormat()`: this ctor should *only* be used at
    /// read-time; writing with it fails with `IllegalStateException("Please
    /// use MockRandomPostingsFormat(Random)")`.
    pub fn new() -> Self {
        MockRandomPostingsFormat { seed_random: None }
    }

    /// `new MockRandomPostingsFormat(Random random)`: the format's random is
    /// `new Random(random.nextLong())`.
    pub fn with_random(random: &dyn Random) -> Self {
        MockRandomPostingsFormat {
            seed_random: Some(JdkRandom::new(random.next_long())),
        }
    }

    /// `seedRandom`, or the error the Java read-time random throws when it
    /// is drawn from (`Random.next(int)` overridden to throw).
    fn seed_random(&self) -> Result<&JdkRandom> {
        self.seed_random.as_ref().ok_or_else(|| {
            LuceneError::illegal_state("Please use MockRandomPostingsFormat(Random)")
        })
    }
}

impl Default for MockRandomPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MockRandomPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for MockRandomPostingsFormat {
    /// `PostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostingsFormat(name=MockRandom)")
    }
}

/// The anonymous `VariableGapTermsIndexWriter.IndexTermSelector` of the
/// random-gap terms index: a term is an index term when `rand.nextInt(gap)
/// == gap / 2`.
struct RandomGapTermSelector {
    rand: JdkRandom,
    gap: i32,
}

impl IndexTermSelector for RandomGapTermSelector {
    fn is_index_term(&mut self, _term: &BytesRef, _stats: &TermStats) -> bool {
        self.rand.next_int_bound(self.gap) == self.gap / 2
    }

    fn new_field(&mut self, _field_info: &FieldInfo) {}
}

impl PostingsFormat for MockRandomPostingsFormat {
    fn name(&self) -> &str {
        "MockRandom"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let min_skip_interval = if state.segment_info.max_doc()? > 1_000_000 {
            // Test2BPostings can OOME otherwise:
            3
        } else {
            2
        };

        let seed_random = self.seed_random()?;
        // we pull this before the seed intentionally: because it's not consumed at runtime
        // (the skipInterval is written into postings header).
        // NOTE: Currently not passed to postings writer.
        //       before, it was being passed in wrongly as acceptableOverhead!
        let skip_interval = TestUtil::next_int(seed_random, min_skip_interval, 10);

        if *VERBOSE {
            println!("MockRandomCodec: skipInterval={skip_interval}");
        }

        let seed = seed_random.next_long();

        if *VERBOSE {
            println!(
                "MockRandomCodec: writing to seg={} formatID={} seed={seed}",
                state.segment_info.name, state.segment_suffix
            );
        }

        let seed_file_name =
            segment_file_name(&state.segment_info.name, &state.segment_suffix, SEED_EXT);
        {
            // try (IndexOutput out = state.directory.createOutput(seedFileName, state.context))
            let mut out = state
                .directory
                .create_output(&seed_file_name, state.context)?;
            let written = (|| -> Result<()> {
                write_index_header(
                    out.as_mut(),
                    "MockRandomSeed",
                    0,
                    &state.segment_info.id(),
                    &state.segment_suffix,
                )?;
                out.write_long(seed)?;
                write_footer(out.as_mut())
            })();
            match written {
                Ok(()) => out.close()?,
                Err(e) => {
                    // The resource's close exception is suppressed.
                    let _ = out.close();
                    return Err(e);
                }
            }
        }

        let random = JdkRandom::new(seed);

        random.next_int(); // consume a random for buffersize

        let postings_writer: Box<dyn PostingsWriterBase> =
            Box::new(Lucene104PostingsWriter::new(state)?);

        // The terms writers' constructors take the postings writer and close
        // it when they fail, which Java's `finally` blocks do here.
        let t1 = random.next_int_bound(4);
        let fields: Box<dyn FieldsConsumer + 'a> = if t1 == 0 {
            Box::new(FSTTermsWriter::new(state, postings_writer)?)
        } else if t1 == 1 {
            // Use BlockTree terms dict

            if *VERBOSE {
                println!("MockRandomCodec: writing BlockTree terms dict");
            }

            // TODO: would be nice to allow 1 but this is very
            // slow to write
            let min_terms_in_block = TestUtil::next_int(&random, 2, 100);
            let max_terms_in_block =
                2.max((min_terms_in_block - 1) * 2 + random.next_int_bound(100));

            Box::new(Lucene103BlockTreeTermsWriter::new(
                state,
                postings_writer,
                min_terms_in_block,
                max_terms_in_block,
            )?)
        } else if t1 == 2 {
            if *VERBOSE {
                println!("MockRandomCodec: writing Block terms dict");
            }

            let mut postings_writer = postings_writer;
            let index_writer = (|| -> Result<Box<dyn TermsIndexWriterBase>> {
                if random.next_boolean() {
                    let term_index_interval = TestUtil::next_int(&random, 1, 100);
                    if *VERBOSE {
                        println!(
                            "MockRandomCodec: fixed-gap terms index (tii={term_index_interval})"
                        );
                    }
                    Ok(Box::new(FixedGapTermsIndexWriter::with_interval(
                        state,
                        term_index_interval,
                    )?))
                } else {
                    let selector: Box<dyn IndexTermSelector> = {
                        let n2 = random.next_int_bound(3);
                        if n2 == 0 {
                            let tii = TestUtil::next_int(&random, 1, 100);
                            if *VERBOSE {
                                println!("MockRandomCodec: variable-gap terms index (tii={tii})");
                            }
                            Box::new(EveryNTermSelector::new(tii))
                        } else if n2 == 1 {
                            let doc_freq_thresh = TestUtil::next_int(&random, 2, 100);
                            let tii = TestUtil::next_int(&random, 1, 100);
                            Box::new(EveryNOrDocFreqTermSelector::new(doc_freq_thresh, tii))
                        } else {
                            let seed2 = random.next_long();
                            let gap = TestUtil::next_int(&random, 2, 40);
                            if *VERBOSE {
                                println!("MockRandomCodec: random-gap terms index (max gap={gap})");
                            }
                            Box::new(RandomGapTermSelector {
                                rand: JdkRandom::new(seed2),
                                gap,
                            })
                        }
                    };
                    Ok(Box::new(VariableGapTermsIndexWriter::new(state, selector)?))
                }
            })();
            let index_writer = match index_writer {
                Ok(w) => w,
                Err(e) => {
                    postings_writer.close()?;
                    return Err(e);
                }
            };

            // `BlockTermsWriter::new` closes both when it fails, as the
            // second `finally` does.
            Box::new(BlockTermsWriter::new(index_writer, state, postings_writer)?)
        } else if t1 == 3 {
            // Use OrdsBlockTree terms dict
            if *VERBOSE {
                println!("MockRandomCodec: writing OrdsBlockTree");
            }

            // TODO: would be nice to allow 1 but this is very
            // slow to write
            let min_terms_in_block = TestUtil::next_int(&random, 2, 100);
            let max_terms_in_block =
                2.max((min_terms_in_block - 1) * 2 + random.next_int_bound(100));

            Box::new(OrdsBlockTreeTermsWriter::new(
                state,
                postings_writer,
                min_terms_in_block,
                max_terms_in_block,
            )?)
        } else {
            // BUG!
            return Err(LuceneError::AssertionError(String::new()));
        };

        Ok(fields)
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let seed_file_name =
            segment_file_name(&state.segment_info.name, &state.segment_suffix, SEED_EXT);
        let mut input = state.directory.open_checksum_input(&seed_file_name)?;
        check_index_header(
            input.as_mut(),
            "MockRandomSeed",
            0,
            0,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let seed = input.read_long()?;
        check_footer(input.as_mut())?;
        if *VERBOSE {
            println!(
                "MockRandomCodec: reading from seg={} formatID={} seed={seed}",
                state.segment_info.name, state.segment_suffix
            );
        }
        input.close()?;

        let random = JdkRandom::new(seed);

        let read_buffer_size = TestUtil::next_int(&random, 1, 4096);
        if *VERBOSE {
            println!("MockRandomCodec: readBufferSize={read_buffer_size}");
        }

        let postings_reader: Box<dyn PostingsReaderBase> =
            Box::new(Lucene104PostingsReader::new(state)?);

        // The terms readers' constructors take the postings reader and close
        // it when they fail, which Java's `finally` blocks do here.
        let t1 = random.next_int_bound(4);
        let fields: Box<dyn FieldsProducer> = if t1 == 0 {
            Box::new(FSTTermsReader::new(state, postings_reader)?)
        } else if t1 == 1 {
            // Use BlockTree terms dict
            if *VERBOSE {
                println!("MockRandomCodec: reading BlockTree terms dict");
            }

            Box::new(Lucene103BlockTreeTermsReader::new(postings_reader, state)?)
        } else if t1 == 2 {
            if *VERBOSE {
                println!("MockRandomCodec: reading Block terms dict");
            }
            let mut postings_reader = postings_reader;
            let index_reader = (|| -> Result<Box<dyn TermsIndexReaderBase>> {
                let do_fixed_gap = random.next_boolean();

                // randomness diverges from writer, here:

                if do_fixed_gap {
                    if *VERBOSE {
                        println!("MockRandomCodec: fixed-gap terms index");
                    }
                    Ok(Box::new(FixedGapTermsIndexReader::new(state)?))
                } else {
                    let n2 = random.next_int_bound(3);
                    if n2 == 1 {
                        random.next_int();
                    } else if n2 == 2 {
                        random.next_long();
                    }
                    if *VERBOSE {
                        println!("MockRandomCodec: variable-gap terms index");
                    }
                    Ok(Box::new(VariableGapTermsIndexReader::new(state)?))
                }
            })();
            let index_reader = match index_reader {
                Ok(r) => r,
                Err(e) => {
                    postings_reader.close()?;
                    return Err(e);
                }
            };

            // `BlockTermsReader::new` closes both when it fails, as the
            // second `finally` does.
            Box::new(BlockTermsReader::new(index_reader, postings_reader, state)?)
        } else if t1 == 3 {
            // Use OrdsBlockTree terms dict
            if *VERBOSE {
                println!("MockRandomCodec: reading OrdsBlockTree terms dict");
            }

            Box::new(OrdsBlockTreeTermsReader::new(postings_reader, state)?)
        } else {
            // BUG!
            return Err(LuceneError::AssertionError(String::new()));
        };

        Ok(fields)
    }
}
