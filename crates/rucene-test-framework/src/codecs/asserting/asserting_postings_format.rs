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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingPostingsFormat`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, NormsProducer, PostingsFormat, SegmentReadState,
    SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::index_writer::MAX_POSITION;
use rucene::index::{
    FieldInfos, Fields, IndexOptions, PostingsEnum, Terms, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::NO_MORE_DOCS;
use rucene::util::java_collections::java_compare_to;
use rucene::util::BytesRef;

use crate::index::asserting_leaf_reader::AssertingTerms;
use crate::util::TestUtil;

/// Just like the default postings format but with additional asserts.
///
/// Port of `AssertingPostingsFormat` (named `Asserting`, over
/// `TestUtil.getDefaultPostingsFormat()`).
pub struct AssertingPostingsFormat {
    r#in: Arc<dyn PostingsFormat>,
}

impl AssertingPostingsFormat {
    /// `new AssertingPostingsFormat()`.
    pub fn new() -> Self {
        AssertingPostingsFormat {
            r#in: TestUtil::get_default_postings_format(),
        }
    }
}

impl Default for AssertingPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AssertingPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostingsFormat(name=Asserting)")
    }
}

impl PostingsFormat for AssertingPostingsFormat {
    fn name(&self) -> &str {
        "Asserting"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        Ok(Box::new(AssertingFieldsConsumer {
            field_infos: state.field_infos,
            r#in: self.r#in.fields_consumer(state)?,
        }))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(AssertingFieldsProducer::new(
            self.r#in.fields_producer(state)?,
        )))
    }
}

/// Port of `AssertingPostingsFormat.AssertingFieldsProducer`.
///
/// Java's constructor asserts `toString() != null`, which a Rust value
/// cannot fail; [`FieldsProducer`] carries no `toString()` in this port.
struct AssertingFieldsProducer {
    r#in: Box<dyn FieldsProducer>,
}

impl AssertingFieldsProducer {
    fn new(r#in: Box<dyn FieldsProducer>) -> Self {
        AssertingFieldsProducer { r#in }
    }
}

impl Fields for AssertingFieldsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.r#in.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(match self.r#in.terms(field)? {
            None => None,
            Some(terms) => Some(Box::new(AssertingTerms::new(terms))),
        })
    }

    fn size(&self) -> i32 {
        self.r#in.size()
    }
}

impl FieldsProducer for AssertingFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        self.r#in.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(AssertingFieldsProducer::new(
            self.r#in.get_merge_instance()?,
        )))
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}

/// Port of `AssertingPostingsFormat.AssertingFieldsConsumer`: writes through
/// the delegate, then walks what it was given and checks the order of fields,
/// terms, documents and positions.
struct AssertingFieldsConsumer<'a> {
    r#in: Box<dyn FieldsConsumer + 'a>,
    field_infos: &'a FieldInfos,
}

impl fmt::Debug for AssertingFieldsConsumer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingFieldsConsumer")
    }
}

impl FieldsConsumer for AssertingFieldsConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        self.r#in.write(fields, norms)?;

        // TODO: more asserts?  can we somehow run a
        // "limited" CheckIndex here???  Or ... can we improve
        // AssertingFieldsProducer and us it also to wrap the
        // incoming Fields here?

        let mut last_field: Option<String> = None;

        for field in fields.iterator() {
            let field_info = self.field_infos.field_info(&field);
            java_assert!(field_info.is_some());
            java_assert!(last_field
                .as_deref()
                .is_none_or(|last| java_compare_to(last, &field) == Ordering::Less));
            last_field = Some(field.clone());

            let Some(terms) = fields.terms(&field)? else {
                continue;
            };
            let Some(field_info) = field_info else {
                // Only reachable with assertions disabled: Java's next line
                // dereferences the null field info.
                return Err(rucene::error::LuceneError::Unchecked {
                    class: "java.lang.NullPointerException",
                    message: None,
                    cause: None,
                });
            };

            let mut terms_enum = terms.iterator()?;
            let mut last_term: Option<BytesRef> = None;
            let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;

            let index_options = field_info.get_index_options();
            let has_freqs = index_options.subsumes(IndexOptions::DOCS_AND_FREQS);
            let has_positions = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
            let has_offsets =
                index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            let has_payloads = terms.has_payloads();

            java_assert!(has_positions == terms.has_positions());
            java_assert!(has_offsets == terms.has_offsets());

            while let Some(term) = terms_enum.next()? {
                java_assert!(last_term.as_ref().is_none_or(|last| *last < term));
                last_term = Some(BytesRef::deep_copy_of(&term));

                let mut flags = 0;
                if !has_positions {
                    if has_freqs {
                        flags |= POSTINGS_ENUM_FREQS;
                    }
                } else {
                    flags = POSTINGS_ENUM_POSITIONS;
                    if has_payloads {
                        flags |= POSTINGS_ENUM_PAYLOADS;
                    }
                    if has_offsets {
                        flags |= POSTINGS_ENUM_OFFSETS;
                    }
                }
                let reuse = postings_enum.take();
                let postings = postings_enum.insert(terms_enum.postings(reuse, flags)?);

                let mut last_doc_id = -1;

                loop {
                    let doc_id = postings.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    java_assert!(doc_id > last_doc_id);
                    last_doc_id = doc_id;
                    if has_freqs {
                        let freq = postings.freq()?;
                        java_assert!(freq > 0);

                        if has_positions {
                            let mut last_pos = -1;
                            let mut last_start_offset = -1;
                            for i in 0..freq {
                                let pos = postings.next_position()?;
                                java_assert!(
                                    pos >= last_pos,
                                    "pos={} vs lastPos={} i={} freq={}",
                                    pos,
                                    last_pos,
                                    i,
                                    freq
                                );
                                java_assert!(
                                    pos <= MAX_POSITION,
                                    "pos={} is > IndexWriter.MAX_POSITION={}",
                                    pos,
                                    MAX_POSITION
                                );
                                last_pos = pos;

                                if has_offsets {
                                    let start_offset = postings.start_offset();
                                    let end_offset = postings.end_offset();
                                    java_assert!(end_offset >= start_offset);
                                    java_assert!(start_offset >= last_start_offset);
                                    last_start_offset = start_offset;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.r#in.close()?;
        self.r#in.close() // close again
    }
}
