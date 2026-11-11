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

//! Port of `org.apache.lucene.tests.util.fst.FSTTester`: helper class to test
//! FSTs.

use std::collections::HashMap;

use randomizedtesting::jdk::random::Random;
use rucene::error::Result;
use rucene::store::{ByteBuffersDataOutput, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::fst::{
    Arc, Builder, FstOutputSink, InputType, IntsRefFSTEnum, Outputs, END_LABEL, FST,
};
use rucene::util::refs::IntsRefBuilder;
use rucene::util::string_helper::IntsRef;
use rucene::util::unicode_util::UnicodeUtil;
use rucene::util::BytesRef;

use crate::util::{LuceneTestCase, TestUtil};

/// `FSTTester.InputOutput`: one input/output pair, ordered by its input.
#[derive(Debug, Clone)]
pub struct InputOutput<T> {
    /// `input`.
    pub input: IntsRef,
    /// `output`.
    pub output: T,
}

/// Port of `org.apache.lucene.tests.util.fst.FSTTester`.
///
/// Java's `outputsEqual(a, b)` is a protected method (`a.equals(b)`) that
/// subclasses override; the port calls [`Outputs::equals`], or the function
/// given to [`with_outputs_equal`](Self::with_outputs_equal).
///
/// **Divergence.** Java adds each value of a `List` output separately
/// (`ListOfOutputs`, of `lucene/misc`); the port, whose outputs are typed,
/// adds every output as one value.
pub struct FSTTester<'a, O: Outputs> {
    /// `random`.
    pub random: &'a dyn Random,
    /// `pairs`, sorted by input.
    pub pairs: Vec<InputOutput<O::Output>>,
    /// `inputMode`: 0 for UTF-8 bytes (`BYTE1`), else UTF-32 (`BYTE4`).
    pub input_mode: i32,
    /// `outputs`.
    pub outputs: O,
    /// `dir`.
    pub dir: &'a dyn Directory,
    /// `nodeCount`.
    pub node_count: i64,
    /// `arcCount`.
    pub arc_count: i64,
    #[allow(clippy::type_complexity)]
    outputs_equal: Option<Box<dyn Fn(&O::Output, &O::Output) -> bool + 'a>>,
}

impl<'a, O: Outputs> FSTTester<'a, O> {
    /// `new FSTTester(random, dir, inputMode, pairs, outputs)`.
    pub fn new(
        random: &'a dyn Random,
        dir: &'a dyn Directory,
        input_mode: i32,
        pairs: Vec<InputOutput<O::Output>>,
        outputs: O,
    ) -> Self {
        FSTTester {
            random,
            pairs,
            input_mode,
            outputs,
            dir,
            node_count: 0,
            arc_count: 0,
            outputs_equal: None,
        }
    }

    /// The override of `outputsEqual(a, b)` of a subclass.
    pub fn with_outputs_equal(mut self, f: impl Fn(&O::Output, &O::Output) -> bool + 'a) -> Self {
        self.outputs_equal = Some(Box::new(f));
        self
    }

    /// `outputsEqual(a, b)`.
    fn outputs_equal(&self, a: &O::Output, b: &O::Output) -> bool {
        match &self.outputs_equal {
            Some(f) => f(a, b),
            None => self.outputs.equals(a, b),
        }
    }

    /// `inputToString(inputMode, term)`.
    pub fn input_to_string(input_mode: i32, term: &IntsRef) -> String {
        Self::input_to_string_valid(input_mode, term, true)
    }

    /// `inputToString(inputMode, term, isValidUnicode)`.
    ///
    /// **Divergence.** A term that does not decode (Java's lenient decoders
    /// would still build a string) is shown by its code units alone.
    pub fn input_to_string_valid(
        input_mode: i32,
        term: &IntsRef,
        is_valid_unicode: bool,
    ) -> String {
        if !is_valid_unicode {
            return term.to_string();
        }
        let text = if input_mode == 0 {
            // utf8
            Self::to_bytes_ref(term).utf8_to_string().ok()
        } else {
            // utf32
            UnicodeUtil::new_string(term.slice(), 0, term.slice().len()).ok()
        };
        match text {
            Some(text) => format!("{text} {term}"),
            None => term.to_string(),
        }
    }

    /// `toBytesRef(ir)`.
    fn to_bytes_ref(ir: &IntsRef) -> BytesRef {
        BytesRef::new(
            ir.slice()
                .iter()
                .map(|&x| {
                    java_assert_unchecked!((0..=255).contains(&x));
                    x as u8
                })
                .collect(),
        )
    }

    /// `getRandomString(random)`: a realistic unicode string or a string of
    /// the limited alphabet `a..f`, at random.
    pub fn get_random_string<R: Random + ?Sized>(random: &R) -> String {
        if random.next_boolean() {
            TestUtil::random_realistic_unicode_string(random)
        } else {
            // we want to mix in limited-alphabet symbols so
            // we get more sharing of the nodes given how few
            // terms we are testing...
            Self::simple_random_string(random)
        }
    }

    /// `simpleRandomString(r)`: up to 9 characters of `a..f`.
    pub fn simple_random_string<R: Random + ?Sized>(r: &R) -> String {
        let end = r.next_int_bound(10);
        if end == 0 {
            // allow 0 length
            return String::new();
        }
        (0..end)
            .map(|_| char::from(TestUtil::next_int(r, 97, 102) as u8))
            .collect()
    }

    /// `toIntsRef(s, inputMode)`: the UTF-8 bytes (`inputMode == 0`) or the
    /// code points of `s`.
    pub fn to_ints_ref(s: &str, input_mode: i32) -> IntsRef {
        let mut ir = IntsRefBuilder::new();
        if input_mode == 0 {
            // utf8
            for &b in s.as_bytes() {
                ir.append(i32::from(b));
            }
        } else {
            // utf32
            for c in s.chars() {
                ir.append(c as i32);
            }
        }
        ir.get()
    }

    /// `run(fst, term, prefixLength)`: the output of `term`, or `None` if the
    /// term isn't accepted; with `prefix_length`, the length of the term
    /// prefix that matches is set and the output of that prefix returned.
    fn run(
        fst: &FST<O>,
        term: &IntsRef,
        prefix_length: Option<&mut usize>,
    ) -> Result<Option<O::Output>> {
        let mut arc = Arc::default();
        fst.get_first_arc(&mut arc);
        let mut output = fst.outputs().no_output();
        let mut fst_reader = fst.get_bytes_reader()?;
        let ints = term.slice();
        for i in 0..=ints.len() {
            let label = if i == ints.len() { END_LABEL } else { ints[i] };
            if !fst.find_target_arc_in_place(label, &mut arc, &mut *fst_reader)? {
                return Ok(match prefix_length {
                    Some(p) => {
                        *p = i;
                        Some(output)
                    }
                    None => None,
                });
            }
            output = fst.outputs().add(&output, arc.output());
        }

        if let Some(p) = prefix_length {
            *p = ints.len();
        }

        Ok(Some(output))
    }

    /// `randomAcceptedWord(fst, in)`.
    fn random_accepted_word(&self, fst: &FST<O>, input: &mut IntsRefBuilder) -> Result<O::Output> {
        let mut arc = Arc::default();
        fst.get_first_arc(&mut arc);

        let mut arcs = Vec::new();
        input.clear();
        let mut output = fst.outputs().no_output();
        let mut fst_reader = fst.get_bytes_reader()?;

        loop {
            // read all arcs:
            fst.read_first_target_arc_in_place(&mut arc, &mut *fst_reader)?;
            arcs.push(arc.clone());
            while !arc.is_last() {
                fst.read_next_arc(&mut arc, &mut *fst_reader)?;
                arcs.push(arc.clone());
            }

            // pick one
            arc = arcs[self.random.next_int_bound(arcs.len() as i32) as usize].clone();
            arcs.clear();

            // accumulate output
            output = fst.outputs().add(&output, arc.output());

            // append label
            if arc.label() == END_LABEL {
                break;
            }

            input.append(arc.label());
        }

        Ok(output)
    }

    /// `doTest()`: builds the FST of the pairs (written off-heap to the
    /// directory, or on-heap and maybe saved and read back), then verifies
    /// it.
    ///
    /// # Errors
    ///
    /// The FST's and the directory's errors.
    ///
    /// # Panics
    ///
    /// The `AssertionError` of a failed verification.
    pub fn do_test(&mut self) -> Result<Option<FST<O>>> {
        let input_type = if self.input_mode == 0 {
            InputType::Byte1
        } else {
            InputType::Byte4
        };
        let use_off_heap = self.random.next_boolean();

        let fst = if use_off_heap {
            let mut index_output = self
                .dir
                .create_output("fstOffHeap.bin", &*DEFAULT_IO_CONTEXT)?;
            let (metadata, node_count, arc_count) = {
                let mut fst_compiler = Builder::new(input_type, self.outputs.clone())
                    .data_output(FstOutputSink::Streaming(&mut *index_output))
                    .build()?;
                for pair in &self.pairs {
                    fst_compiler.add(&pair.input, pair.output.clone())?;
                }
                let metadata = fst_compiler.compile()?;
                (
                    metadata,
                    fst_compiler.node_count(),
                    fst_compiler.arc_count(),
                )
            };
            self.node_count = node_count;
            self.arc_count = arc_count;
            index_output.close()?;
            drop(index_output);
            match metadata {
                None => {
                    self.dir.delete_file("fstOffHeap.bin")?;
                    None
                }
                Some(metadata) => {
                    let fst = {
                        // `try (IndexInput in = dir.openInput(...))`
                        // (`FSTTester.java:287`): closed however the read ends.
                        let mut input = self
                            .dir
                            .open_input("fstOffHeap.bin", &*DEFAULT_IO_CONTEXT)?;
                        let read = FST::new(metadata, &mut *input);
                        let closed = input.close();
                        read.and_then(|fst| closed.map(|()| fst))
                    };
                    self.dir.delete_file("fstOffHeap.bin")?;
                    Some(fst?)
                }
            }
        } else {
            let mut fst_compiler = Builder::new(input_type, self.outputs.clone()).build()?;
            for pair in &self.pairs {
                fst_compiler.add(&pair.input, pair.output.clone())?;
            }
            let metadata = fst_compiler.compile()?;
            self.node_count = fst_compiler.node_count();
            self.arc_count = fst_compiler.arc_count();
            match metadata {
                None => None,
                Some(metadata) => {
                    let fst = FST::from_fst_reader(Some(metadata), fst_compiler.into_fst_reader()?)
                        .expect("INVARIANT: metadata is present");
                    if self.random.next_boolean() {
                        let context = LuceneTestCase::new_io_context(self.random);
                        {
                            let mut out = self.dir.create_output("fst.bin", &*context)?;
                            // fst.save(out, out): the metadata, then the bytes,
                            // into one output.
                            let mut meta = ByteBuffersDataOutput::new();
                            let mut body = ByteBuffersDataOutput::new();
                            fst.save(&mut meta, &mut body)?;
                            meta.copy_to(&mut *out)?;
                            body.copy_to(&mut *out)?;
                            out.close()?;
                        }
                        let fst = {
                            // `try (IndexInput in = dir.openInput(...))`
                            // (`FSTTester.java:300`).
                            let mut input = self.dir.open_input("fst.bin", &*context)?;
                            let read = FST::read_metadata(&mut *input, self.outputs.clone())
                                .and_then(|metadata| FST::new(metadata, &mut *input));
                            let closed = input.close();
                            read.and_then(|fst| closed.map(|()| fst))
                        };
                        self.dir.delete_file("fst.bin")?;
                        Some(fst?)
                    } else {
                        Some(fst)
                    }
                }
            }
        };

        if *crate::util::lucene_test_case::VERBOSE {
            match &fst {
                None => println!("  fst has 0 nodes (fully pruned)"),
                Some(_) => println!(
                    "  fst has {} nodes and {} arcs",
                    self.node_count, self.arc_count
                ),
            }
        }

        self.verify_un_pruned(self.input_mode, fst.as_ref())?;

        Ok(fst)
    }

    /// `verifyUnPruned(inputMode, fst)`: the FST is complete.
    fn verify_un_pruned(&self, input_mode: i32, fst: Option<&FST<O>>) -> Result<()> {
        if self.pairs.is_empty() {
            assert!(fst.is_none());
            return Ok(());
        }

        let fst = fst.expect("assertNotNull(fst)");
        let pairs = &self.pairs;

        // visit valid pairs in order -- make sure all words
        // are accepted, and FSTEnum's next() steps through
        // them correctly
        {
            let mut fst_enum = IntsRefFSTEnum::new(fst)?;
            for pair in pairs {
                let term = &pair.input;
                let output = Self::run(fst, term, None)?.unwrap_or_else(|| {
                    panic!(
                        "term {} is not accepted",
                        Self::input_to_string(input_mode, term)
                    )
                });
                assert!(self.outputs_equal(&pair.output, &output));

                // verify enum's next
                let t = fst_enum.next()?.expect("assertNotNull(t)");
                assert_eq!(
                    term,
                    t.input,
                    "expected input={} but fstEnum returned {}",
                    Self::input_to_string(input_mode, term),
                    Self::input_to_string(input_mode, t.input)
                );
                assert!(self.outputs_equal(&pair.output, t.output));
            }
            assert!(fst_enum.next()?.is_none());
        }

        let terms_map: HashMap<Vec<i32>, &O::Output> = pairs
            .iter()
            .map(|pair| (pair.input.slice().to_vec(), &pair.output))
            .collect();

        // find random matching word and make sure it's valid
        let mut scratch = IntsRefBuilder::new();
        let num = LuceneTestCase::at_least_with(self.random, 500);
        for _ in 0..num {
            let output = self.random_accepted_word(fst, &mut scratch)?;
            let word = scratch.get();
            let expected = terms_map.get(word.slice()).unwrap_or_else(|| {
                panic!(
                    "accepted word {} is not valid",
                    Self::input_to_string(input_mode, &word)
                )
            });
            assert!(self.outputs_equal(expected, &output));
        }

        // test IntsRefFSTEnum.seek:
        let mut fst_enum = IntsRefFSTEnum::new(fst)?;
        let num = LuceneTestCase::at_least_with(self.random, 100);
        let search = |term: &IntsRef| pairs.binary_search_by(|p| p.input.cmp(term));
        for _ in 0..num {
            if self.random.next_boolean() {
                // seek to term that doesn't exist:
                loop {
                    let term = Self::to_ints_ref(&Self::get_random_string(self.random), input_mode);
                    if let Err(insertion) = search(&term) {
                        let mut pos = insertion as i64;
                        // ok doesn't exist
                        let seek_result = if self.random.next_int_bound(3) == 0 {
                            pos = -1;
                            fst_enum.seek_exact(&term)?
                        } else if self.random.next_boolean() {
                            pos -= 1;
                            fst_enum.seek_floor(&term)?
                        } else {
                            fst_enum.seek_ceil(&term)?
                        };
                        if pos != -1 && (pos as usize) < pairs.len() {
                            let expected = &pairs[pos as usize];
                            let seek_result = seek_result.unwrap_or_else(|| {
                                panic!(
                                    "got null but expected term={}",
                                    Self::input_to_string(input_mode, &expected.input)
                                )
                            });
                            assert_eq!(&expected.input, seek_result.input);
                            assert!(self.outputs_equal(&expected.output, seek_result.output));
                        } else {
                            // seeked before start or beyond end
                            assert!(
                                seek_result.is_none(),
                                "expected null but got {}",
                                seek_result
                                    .map(|r| Self::input_to_string(input_mode, r.input))
                                    .unwrap_or_default()
                            );
                        }
                        break;
                    }
                }
            } else {
                // seek to term that does exist:
                let pair = &pairs[self.random.next_int_bound(pairs.len() as i32) as usize];
                let seek_result = if self.random.next_int_bound(3) == 2 {
                    fst_enum.seek_exact(&pair.input)?
                } else if self.random.next_boolean() {
                    fst_enum.seek_floor(&pair.input)?
                } else {
                    fst_enum.seek_ceil(&pair.input)?
                };
                let seek_result = seek_result.expect("assertNotNull(seekResult)");
                assert_eq!(&pair.input, seek_result.input);
                assert!(self.outputs_equal(&pair.output, seek_result.output));
            }
        }

        // test mixed next/seek
        let num = LuceneTestCase::at_least_with(self.random, 100);
        for _ in 0..num {
            // reset:
            let mut fst_enum = IntsRefFSTEnum::new(fst)?;
            let mut upto: i64 = -1;
            loop {
                let is_done;
                let n = pairs.len() as i64;
                if upto == n - 1 || self.random.next_boolean() {
                    // next
                    upto += 1;
                    is_done = fst_enum.next()?.is_none();
                } else if upto != -1
                    && (upto as f64) < 0.75 * n as f64
                    && self.random.next_boolean()
                {
                    let mut attempt = 0;
                    let mut done = None;
                    while attempt < 10 {
                        let term =
                            Self::to_ints_ref(&Self::get_random_string(self.random), input_mode);
                        if !terms_map.contains_key(term.slice())
                            && term.cmp(&pairs[upto as usize].input).is_gt()
                        {
                            let pos = search(&term);
                            java_assert_unchecked!(pos.is_err());
                            upto = pos.err().unwrap_or(0) as i64;

                            if self.random.next_boolean() {
                                upto -= 1;
                                assert!(upto != -1);
                                done = Some(fst_enum.seek_floor(&term)?.is_none());
                            } else {
                                done = Some(fst_enum.seek_ceil(&term)?.is_none());
                            }
                            break;
                        }
                        attempt += 1;
                    }
                    match done {
                        Some(d) => is_done = d,
                        None => continue,
                    }
                } else {
                    let inc = self.random.next_int_bound((n - upto - 1) as i32);
                    upto += i64::from(inc);
                    if upto == -1 {
                        upto = 0;
                    }

                    let target = &pairs[upto as usize].input;
                    is_done = if self.random.next_boolean() {
                        fst_enum.seek_ceil(target)?.is_none()
                    } else {
                        fst_enum.seek_floor(target)?.is_none()
                    };
                }
                if upto == n {
                    assert!(is_done);
                    break;
                } else {
                    assert!(!is_done);
                    let current = fst_enum.current().expect("current");
                    assert_eq!(&pairs[upto as usize].input, current.input);
                    assert!(self.outputs_equal(&pairs[upto as usize].output, current.output));
                }
            }
        }
        Ok(())
    }
}
