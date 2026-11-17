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

//! Port of `TestDocValuesFieldSources.java`.
//!
//! `test(DocValuesType)` takes a parameter, so JUnit's method providers do
//! not run it as a test (`LuceneJUnit3MethodProvider` selects public no-arg
//! `test*` methods); `test()` calls it for each type.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField,
};
use rucene::error::Result;
use rucene::index::{directory_reader, DocValuesType, IndexReader};
use rucene::search::sorted_numeric_selector::SortedNumericSelectorType;
use rucene::util::mutable::MutableValueObject;
use rucene::util::packed::PackedInts;
use rucene::util::{BytesRef, BytesRefBuilder};
use rucene_queries::function::value_source::{new_context, ValueSource};
use rucene_queries::function::valuesource::bytes_ref_field_source::BytesRefFieldSource;
use rucene_queries::function::valuesource::long_field_source::LongFieldSource;
use rucene_queries::function::valuesource::multi_valued_long_field_source::MultiValuedLongFieldSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::assertion_error;
use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::field_mut_as;

/// `TestDocValuesFieldSources`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestDocValuesFieldSources",
        vec![test("test", test_all)],
    )
}

/// A value of `vals` (`Object[]`): a `String` or a `Long`.
#[derive(Clone, Debug, PartialEq)]
enum Val {
    Str(String),
    Long(i64),
}

/// `test(DocValuesType type)`.
#[allow(clippy::match_same_arms)]
fn test_type(type_: DocValuesType) -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let iw_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    let n_docs = LuceneTestCase::at_least(50);
    let mut document = Document::new();
    document.add(NumericDocValuesField::new("id", 0));
    match type_ {
        DocValuesType::BINARY => {
            document.add(BinaryDocValuesField::new("dv", BytesRef::default()));
        }
        DocValuesType::SORTED => {
            document.add(SortedDocValuesField::new("dv", BytesRef::default()));
        }
        DocValuesType::NUMERIC => {
            document.add(NumericDocValuesField::new("dv", 0));
        }
        DocValuesType::SORTED_NUMERIC => {
            document.add(SortedNumericDocValuesField::new("dv", 0));
        }
        _ => return Err(assertion_error("")),
    }
    let mut vals: Vec<Option<Val>> = vec![None; n_docs as usize];
    let iw = RandomIndexWriter::new_with_config(&*random(), d.as_directory(), iw_config)?;
    for i in 0..n_docs as usize {
        field_mut_as::<NumericDocValuesField>(&mut document, "id").set_value(i as i64);
        match type_ {
            DocValuesType::SORTED | DocValuesType::BINARY => {
                let s = loop {
                    let s = TestUtil::random_simple_string_max(&*random(), 20);
                    if !s.is_empty() {
                        break s;
                    }
                };
                vals[i] = Some(Val::Str(s.clone()));
                if type_ == DocValuesType::SORTED {
                    field_mut_as::<SortedDocValuesField>(&mut document, "dv")
                        .set_bytes_value(BytesRef::from_char_sequence(&s))?;
                } else {
                    field_mut_as::<BinaryDocValuesField>(&mut document, "dv")
                        .set_bytes_value(BytesRef::from_char_sequence(&s))?;
                }
            }
            DocValuesType::NUMERIC | DocValuesType::SORTED_NUMERIC => {
                let bits_per_value = RandomNumbers::random_int_between(&*random(), 1, 31); // keep it an int
                let v = i64::from(
                    random().next_int_bound(PackedInts::max_value(bits_per_value) as i32),
                );
                vals[i] = Some(Val::Long(v));
                if type_ == DocValuesType::NUMERIC {
                    field_mut_as::<NumericDocValuesField>(&mut document, "dv").set_value(v);
                } else {
                    field_mut_as::<SortedNumericDocValuesField>(&mut document, "dv")
                        .set_long_value(v);
                }
            }
            _ => return Err(assertion_error("")),
        }
        iw.add_document(&document)?;
        if random().next_boolean() && i % 10 == 9 {
            iw.commit()?;
        }
    }
    iw.close()?;

    let rd = directory_reader::open(d.as_directory())?;
    let context = new_context(None);
    for leave in (Arc::clone(&rd) as Arc<dyn IndexReader>).leaves()? {
        let mut ids = (Arc::new(LongFieldSource::new("id")) as Arc<dyn ValueSource>)
            .get_values(&context, &leave)?;
        let vs: Arc<dyn ValueSource> = match type_ {
            DocValuesType::BINARY | DocValuesType::SORTED => {
                Arc::new(BytesRefFieldSource::new("dv"))
            }
            DocValuesType::NUMERIC => Arc::new(LongFieldSource::new("dv")),
            DocValuesType::SORTED_NUMERIC => {
                // Since we are not indexing multiple values, MIN and MAX should work the same way
                if random().next_boolean() {
                    Arc::new(MultiValuedLongFieldSource::new(
                        "dv",
                        SortedNumericSelectorType::MIN,
                    ))
                } else {
                    Arc::new(MultiValuedLongFieldSource::new(
                        "dv",
                        SortedNumericSelectorType::MAX,
                    ))
                }
            }
            _ => return Err(assertion_error("")),
        };
        let is_bytes_ref = vs.as_any().is::<BytesRefFieldSource>();
        // `vs instanceof LongFieldSource`: `MultiValuedLongFieldSource` extends it
        let is_long =
            vs.as_any().is::<LongFieldSource>() || vs.as_any().is::<MultiValuedLongFieldSource>();
        let mut values = Arc::clone(&vs).get_values(&context, &leave)?;
        let mut bytes = BytesRefBuilder::new();
        for i in 0..leave.leaf_reader().max_doc() {
            Assert::assert_true(values.exists(i)?);
            if is_bytes_ref {
                Assert::assert_true(matches!(
                    values.object_val(i)?,
                    Some(MutableValueObject::Str(_))
                ));
            } else if is_long {
                Assert::assert_true(matches!(
                    values.object_val(i)?,
                    Some(MutableValueObject::Long(_))
                ));
                Assert::assert_true(values.bytes_val(i, &mut bytes)?);
            } else {
                return Err(assertion_error(""));
            }

            let expected = vals[ids.int_val(i)? as usize]
                .clone()
                .expect("a value per doc");
            match type_ {
                DocValuesType::SORTED | DocValuesType::BINARY => {
                    if type_ == DocValuesType::SORTED {
                        values.ord_val(i)?; // no exception
                        Assert::assert_true(values.num_ord()? >= 1);
                    }
                    // fall-through
                    let Val::Str(expected) = expected else {
                        return Err(assertion_error(""));
                    };
                    let object = |values: &mut Box<
                        dyn rucene_queries::function::function_values::FunctionValues + '_,
                    >|
                     -> Result<Option<String>> {
                        Ok(match values.object_val(i)? {
                            Some(MutableValueObject::Str(s)) => Some(s),
                            _ => None,
                        })
                    };
                    Assert::assert_equals_str(
                        None,
                        Some(&expected),
                        object(&mut values)?.as_deref(),
                    );
                    Assert::assert_equals_str(None, Some(&expected), values.str_val(i)?.as_deref());
                    Assert::assert_equals_str(
                        None,
                        Some(&expected),
                        object(&mut values)?.as_deref(),
                    );
                    Assert::assert_equals_str(None, Some(&expected), values.str_val(i)?.as_deref());
                    Assert::assert_true(values.bytes_val(i, &mut bytes)?);
                    Assert::assert_true(BytesRef::from_char_sequence(&expected) == bytes.get());
                }
                DocValuesType::NUMERIC | DocValuesType::SORTED_NUMERIC => {
                    let Val::Long(expected) = expected else {
                        return Err(assertion_error(""));
                    };
                    Assert::assert_equals(expected, values.long_val(i)?);
                }
                _ => return Err(assertion_error("")),
            }
        }
    }
    rd.close()?;
    d.close()
}

fn test_all() -> Result<()> {
    // DocValuesType.values(), in declaration order
    for type_ in [
        DocValuesType::NONE,
        DocValuesType::NUMERIC,
        DocValuesType::BINARY,
        DocValuesType::SORTED,
        DocValuesType::SORTED_NUMERIC,
        DocValuesType::SORTED_SET,
    ] {
        if type_ != DocValuesType::SORTED_SET && type_ != DocValuesType::NONE {
            test_type(type_)?;
        }
    }
    Ok(())
}
