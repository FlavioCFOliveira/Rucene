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

//! Port of `BaseFragmentsBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use rucene::document::{Field, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    FieldInfo, IndexReader, IndexableFieldType, StoredFieldVisitor, StoredFieldVisitorStatus,
};

use crate::highlight::default_encoder::DefaultEncoder;
use crate::highlight::encoder::Encoder;
use crate::java::{substring, units};
use crate::vectorhighlight::boundary_scanner::BoundaryScanner;
use crate::vectorhighlight::field_frag_list::{FieldFragList, SubInfo, WeightedFragInfo};
use crate::vectorhighlight::field_phrase_list::Toffs;
use crate::vectorhighlight::fragments_builder::FragmentsBuilder;
use crate::vectorhighlight::simple_boundary_scanner::SimpleBoundaryScanner;

/// Equivalent to `BaseFragmentsBuilder.COLORED_PRE_TAGS`.
pub const COLORED_PRE_TAGS: [&str; 20] = [
    "<b style=\"background:yellow\">",
    "<b style=\"background:lawngreen\">",
    "<b style=\"background:aquamarine\">",
    "<b style=\"background:magenta\">",
    "<b style=\"background:palegreen\">",
    "<b style=\"background:coral\">",
    "<b style=\"background:wheat\">",
    "<b style=\"background:khaki\">",
    "<b style=\"background:lime\">",
    "<b style=\"background:deepskyblue\">",
    "<b style=\"background:deeppink\">",
    "<b style=\"background:salmon\">",
    "<b style=\"background:peachpuff\">",
    "<b style=\"background:violet\">",
    "<b style=\"background:mediumpurple\">",
    "<b style=\"background:palegoldenrod\">",
    "<b style=\"background:darkkhaki\">",
    "<b style=\"background:springgreen\">",
    "<b style=\"background:turquoise\">",
    "<b style=\"background:powderblue\">",
];

/// Equivalent to `BaseFragmentsBuilder.COLORED_POST_TAGS`.
pub const COLORED_POST_TAGS: [&str; 1] = ["</b>"];

/// The state of the abstract class
/// `org.apache.lucene.search.vectorhighlight.BaseFragmentsBuilder`; its
/// behaviour is [`BaseFragmentsBuilderImpl`].
#[derive(Debug)]
pub struct BaseFragmentsBuilder {
    /// Equivalent to the protected field `preTags`.
    pub pre_tags: Vec<String>,
    /// Equivalent to the protected field `postTags`.
    pub post_tags: Vec<String>,
    multi_valued_separator: u16,
    boundary_scanner: Box<dyn BoundaryScanner>,
    discrete_multi_value_highlighting: bool,
}

impl BaseFragmentsBuilder {
    /// Equivalent to the protected `BaseFragmentsBuilder()`.
    pub fn new() -> Self {
        Self::with_tags(vec!["<b>".to_string()], vec!["</b>".to_string()])
    }

    /// Equivalent to the protected `BaseFragmentsBuilder(String[],
    /// String[])`.
    pub fn with_tags(pre_tags: Vec<String>, post_tags: Vec<String>) -> Self {
        Self::with_tags_and_scanner(pre_tags, post_tags, Box::new(SimpleBoundaryScanner::new()))
    }

    /// Equivalent to the protected `BaseFragmentsBuilder(BoundaryScanner)`.
    pub fn with_scanner(boundary_scanner: Box<dyn BoundaryScanner>) -> Self {
        Self::with_tags_and_scanner(
            vec!["<b>".to_string()],
            vec!["</b>".to_string()],
            boundary_scanner,
        )
    }

    /// Equivalent to the protected `BaseFragmentsBuilder(String[], String[],
    /// BoundaryScanner)`.
    pub fn with_tags_and_scanner(
        pre_tags: Vec<String>,
        post_tags: Vec<String>,
        boundary_scanner: Box<dyn BoundaryScanner>,
    ) -> Self {
        Self {
            pre_tags,
            post_tags,
            multi_valued_separator: b' ' as u16,
            boundary_scanner,
            discrete_multi_value_highlighting: false,
        }
    }

    /// Equivalent to the package-private static `checkTagsArgument(Object)`:
    /// `tags` must be a `String` or a `String[]` (`Vec<String>`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for any other type.
    pub(crate) fn check_tags_argument(tags: &dyn Any) -> Result<&dyn Any> {
        if tags.is::<String>() || tags.is::<Vec<String>>() {
            return Ok(tags);
        }
        Err(LuceneError::illegal_argument(
            "type of preTags/postTags must be a String or String[]",
        ))
    }

    /// Equivalent to `setMultiValuedSeparator(char)`.
    pub fn set_multi_valued_separator(&mut self, separator: u16) {
        self.multi_valued_separator = separator;
    }

    /// Equivalent to `getMultiValuedSeparator()`.
    pub fn get_multi_valued_separator(&self) -> u16 {
        self.multi_valued_separator
    }

    /// Equivalent to `isDiscreteMultiValueHighlighting()`.
    pub fn is_discrete_multi_value_highlighting(&self) -> bool {
        self.discrete_multi_value_highlighting
    }

    /// Equivalent to `setDiscreteMultiValueHighlighting(boolean)`.
    pub fn set_discrete_multi_value_highlighting(
        &mut self,
        discrete_multi_value_highlighting: bool,
    ) {
        self.discrete_multi_value_highlighting = discrete_multi_value_highlighting;
    }

    /// Equivalent to the protected `getPreTag(int)`.
    pub fn get_pre_tag(&self, num: i32) -> &str {
        Self::get_pre_tag_of(&self.pre_tags, num)
    }

    /// Equivalent to the protected `getPostTag(int)`.
    pub fn get_post_tag(&self, num: i32) -> &str {
        Self::get_post_tag_of(&self.post_tags, num)
    }

    /// Equivalent to the protected `getPreTag(String[], int)`.
    ///
    /// # Panics
    ///
    /// When `pre_tags` is empty, where Java throws `ArithmeticException`.
    pub fn get_pre_tag_of(pre_tags: &[String], num: i32) -> &str {
        let n = num % pre_tags.len() as i32;
        &pre_tags[n as usize]
    }

    /// Equivalent to the protected `getPostTag(String[], int)`.
    ///
    /// # Panics
    ///
    /// When `post_tags` is empty, where Java throws `ArithmeticException`.
    pub fn get_post_tag_of(post_tags: &[String], num: i32) -> &str {
        let n = num % post_tags.len() as i32;
        &post_tags[n as usize]
    }
}

impl Default for BaseFragmentsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The anonymous `StoredFieldVisitor` of `getFields`.
struct FieldsVisitor<'a> {
    field_name: &'a str,
    fields: Vec<Field>,
}

impl StoredFieldVisitor for FieldsVisitor<'_> {
    fn string_field(&mut self, field_info: &Arc<FieldInfo>, value: String) -> Result<()> {
        let ft = FieldType::new_from(TextField::type_stored());
        ft.set_store_term_vectors(field_info.has_term_vectors())?;
        self.fields
            .push(Field::new(field_info.name.clone(), value, ft)?);
        Ok(())
    }

    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        Ok(if field_info.name == self.field_name {
            StoredFieldVisitorStatus::Yes
        } else {
            StoredFieldVisitorStatus::No
        })
    }
}

/// The behaviour of the abstract class
/// `org.apache.lucene.search.vectorhighlight.BaseFragmentsBuilder`: the
/// abstract `getWeightedFragInfoList` and the protected methods a subclass
/// may override, each defaulting to the base class's body. Every
/// implementation is a [`FragmentsBuilder`].
pub trait BaseFragmentsBuilderImpl: Debug + Send + Sync {
    /// The base class's state.
    fn base(&self) -> &BaseFragmentsBuilder;

    /// Equivalent to the abstract `getWeightedFragInfoList(List)`.
    fn get_weighted_frag_info_list(&self, src: Vec<WeightedFragInfo>) -> Vec<WeightedFragInfo>;

    /// Equivalent to the protected `getFields(IndexReader, int, String)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn get_fields(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
    ) -> Result<Vec<Field>> {
        // according to javadoc, doc.getFields(fieldName) cannot be used with
        // lazy loaded field???
        let mut visitor = FieldsVisitor {
            field_name,
            fields: Vec::new(),
        };
        reader
            .stored_fields()?
            .document_with_visitor(doc_id, &mut visitor)?;
        Ok(visitor.fields)
    }

    /// Equivalent to the protected `makeFragment(StringBuilder, int[],
    /// Field[], WeightedFragInfo, String[], String[], Encoder)`: `buffer`
    /// is the UTF-16 source buffer and `index` the next value index.
    ///
    /// # Errors
    ///
    /// Java's `StringIndexOutOfBoundsException` and
    /// `ArrayIndexOutOfBoundsException` for inconsistent offsets.
    #[allow(clippy::too_many_arguments)]
    fn make_fragment(
        &self,
        buffer: &mut Vec<u16>,
        index: &mut i32,
        values: &[Field],
        frag_info: &WeightedFragInfo,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<String> {
        let mut fragment = String::new();
        let s = frag_info.get_start_offset();
        let mut modified_start_offset = s;
        let src = units(&self.get_fragment_source_mso(
            buffer,
            index,
            values,
            s,
            frag_info.get_end_offset(),
            &mut modified_start_offset,
        )?);
        let mut src_index = 0;
        for sub_info in frag_info.get_sub_infos() {
            for to in sub_info.terms_offsets() {
                fragment.push_str(&encoder.encode_text(&substring(
                    &src,
                    src_index,
                    to.get_start_offset() - modified_start_offset,
                )?));
                fragment.push_str(BaseFragmentsBuilder::get_pre_tag_of(
                    pre_tags,
                    sub_info.seqnum(),
                ));
                fragment.push_str(&encoder.encode_text(&substring(
                    &src,
                    to.get_start_offset() - modified_start_offset,
                    to.get_end_offset() - modified_start_offset,
                )?));
                fragment.push_str(BaseFragmentsBuilder::get_post_tag_of(
                    post_tags,
                    sub_info.seqnum(),
                ));
                src_index = to.get_end_offset() - modified_start_offset;
            }
        }
        fragment.push_str(&encoder.encode_text(&substring(&src, src_index, src.len() as i32)?));
        Ok(fragment)
    }

    /// Equivalent to the protected `getFragmentSourceMSO(StringBuilder,
    /// int[], Field[], int, int, int[])`.
    ///
    /// # Errors
    ///
    /// Java's `StringIndexOutOfBoundsException` and
    /// `ArrayIndexOutOfBoundsException` for inconsistent offsets.
    fn get_fragment_source_mso(
        &self,
        buffer: &mut Vec<u16>,
        index: &mut i32,
        values: &[Field],
        start_offset: i32,
        end_offset: i32,
        modified_start_offset: &mut i32,
    ) -> Result<String> {
        let base = self.base();
        while (buffer.len() as i32) < end_offset && (*index as usize) < values.len() {
            let value = values[*index as usize].string_value().unwrap_or_default();
            *index += 1;
            buffer.extend(value.encode_utf16());
            buffer.push(base.get_multi_valued_separator());
        }
        let mut buffer_length = buffer.len() as i32;
        // we added the multi value char to the last buffer, ignore it
        let last = *index - 1;
        if last < 0 || last as usize >= values.len() {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {last} out of bounds for length {}", values.len()),
            ));
        }
        if values[last as usize].field_type().tokenized() {
            buffer_length -= 1;
        }
        let eo = if buffer_length < end_offset {
            buffer_length
        } else {
            base.boundary_scanner.find_end_offset(buffer, end_offset)
        };
        *modified_start_offset = base
            .boundary_scanner
            .find_start_offset(buffer, start_offset);
        substring(buffer, *modified_start_offset, eo)
    }

    /// Equivalent to the protected `getFragmentSource(StringBuilder, int[],
    /// Field[], int, int)`.
    ///
    /// # Errors
    ///
    /// Java's `StringIndexOutOfBoundsException` for inconsistent offsets.
    fn get_fragment_source(
        &self,
        buffer: &mut Vec<u16>,
        index: &mut i32,
        values: &[Field],
        start_offset: i32,
        end_offset: i32,
    ) -> Result<String> {
        let base = self.base();
        while (buffer.len() as i32) < end_offset && (*index as usize) < values.len() {
            let value = values[*index as usize].string_value().unwrap_or_default();
            buffer.extend(value.encode_utf16());
            buffer.push(base.multi_valued_separator);
            *index += 1;
        }
        let eo = if (buffer.len() as i32) < end_offset {
            buffer.len() as i32
        } else {
            end_offset
        };
        substring(buffer, start_offset, eo)
    }

    /// Equivalent to the protected
    /// `discreteMultiValueHighlighting(List, Field[])`; the sub infos of
    /// `frag_infos` are edited as Java edits them.
    fn discrete_multi_value_highlighting(
        &self,
        frag_infos: &mut [WeightedFragInfo],
        fields: &[Field],
    ) -> Vec<WeightedFragInfo> {
        // Java's HashMap<String, List> keyed by the field names; every value
        // of `fields` has the one name `getFields` selected, so the order of
        // its values is the order of the names' first appearance.
        let mut field_name_to_frag_infos: Vec<(String, Vec<WeightedFragInfo>)> = Vec::new();
        for field in fields {
            if !field_name_to_frag_infos
                .iter()
                .any(|(name, _)| name == field.name())
            {
                field_name_to_frag_infos.push((field.name().to_string(), Vec::new()));
            } else if let Some(entry) = field_name_to_frag_infos
                .iter_mut()
                .find(|(name, _)| name == field.name())
            {
                // `put` replaces the list of a repeated name
                entry.1 = Vec::new();
            }
        }
        let list_of = |map: &mut Vec<(String, Vec<WeightedFragInfo>)>, name: &str| -> usize {
            map.iter()
                .position(|(n, _)| n == name)
                .expect("INVARIANT: every field name is a key")
        };

        'frag_infos: for frag_info in frag_infos.iter_mut() {
            let mut field_start;
            let mut field_end = 0;
            for field in fields {
                let value = field.string_value().unwrap_or_default();
                if value.is_empty() {
                    field_end += 1;
                    continue;
                }
                field_start = field_end;
                field_end += value.encode_utf16().count() as i32 + 1; // + 1 for going to next field with same name.

                if frag_info.get_start_offset() >= field_start
                    && frag_info.get_end_offset() >= field_start
                    && frag_info.get_start_offset() <= field_end
                    && frag_info.get_end_offset() <= field_end
                {
                    let i = list_of(&mut field_name_to_frag_infos, field.name());
                    field_name_to_frag_infos[i].1.push(frag_info.clone());
                    continue 'frag_infos;
                }

                if frag_info.get_sub_infos().is_empty() {
                    continue 'frag_infos;
                }

                let first_toffs = frag_info.get_sub_infos()[0].terms_offsets()[0];
                if frag_info.get_start_offset() >= field_end
                    || first_toffs.get_start_offset() >= field_end
                {
                    continue;
                }

                let mut frag_start = field_start;
                if frag_info.get_start_offset() > field_start
                    && frag_info.get_start_offset() < field_end
                {
                    frag_start = frag_info.get_start_offset();
                }

                let mut frag_end = field_end;
                if frag_info.get_end_offset() > field_start
                    && frag_info.get_end_offset() < field_end
                {
                    frag_end = frag_info.get_end_offset();
                }

                let mut sub_infos = Vec::new();
                let mut boost = 0.0f32;
                let mut s = 0;
                while s < frag_info.get_sub_infos().len() {
                    let sub_info: &mut SubInfo = &mut frag_info.get_sub_infos_mut()[s];
                    let mut toffs_list: Vec<Toffs> = Vec::new();
                    let mut t = 0;
                    while t < sub_info.terms_offsets().len() {
                        let toffs = sub_info.terms_offsets()[t];
                        if toffs.get_start_offset() >= field_end {
                            // We've gone past the boundary of this field, so
                            // break out.
                            break;
                        }

                        let starts_after_field = toffs.get_start_offset() >= field_start;
                        let ends_before_field = toffs.get_end_offset() < field_end;
                        if starts_after_field && ends_before_field {
                            // The Toff is entirely within this field.
                            toffs_list.push(toffs);
                            sub_info.terms_offsets_mut().remove(t);
                        } else if starts_after_field {
                            // The Toff starts within this field, but ends
                            // after this field. A field with a Toff that
                            // spans multiple values... nothing to do here.
                            toffs_list.push(Toffs::new(toffs.get_start_offset(), field_end - 1));
                            t += 1;
                        } else if ends_before_field {
                            // The Toff starts before this field and ends in
                            // this field.
                            toffs_list.push(Toffs::new(field_start, toffs.get_end_offset()));
                            sub_info.terms_offsets_mut().remove(t);
                        } else {
                            // The Toff starts before this field and ends after
                            // this field. Should never happen...
                            toffs_list.push(Toffs::new(field_start, field_end - 1));
                            t += 1;
                        }
                    }
                    if !toffs_list.is_empty() {
                        sub_infos.push(SubInfo::new(
                            sub_info.text().to_string(),
                            toffs_list,
                            sub_info.seqnum(),
                            sub_info.boost(),
                        ));
                        boost += sub_info.boost();
                    }

                    if sub_info.terms_offsets().is_empty() {
                        frag_info.get_sub_infos_mut().remove(s);
                    } else {
                        s += 1;
                    }
                }
                let weighted_frag_info =
                    WeightedFragInfo::new(frag_start, frag_end, sub_infos, boost);
                let i = list_of(&mut field_name_to_frag_infos, field.name());
                field_name_to_frag_infos[i].1.push(weighted_frag_info);
            }
        }

        let mut result: Vec<WeightedFragInfo> = Vec::new();
        for (_, weighted_frag_infos) in field_name_to_frag_infos {
            result.extend(weighted_frag_infos);
        }
        result.sort_by(|info1, info2| {
            info1
                .get_start_offset()
                .wrapping_sub(info2.get_start_offset())
                .cmp(&0)
        });

        result
    }
}

static NULL_ENCODER: DefaultEncoder = DefaultEncoder;

impl<T: BaseFragmentsBuilderImpl> FragmentsBuilder for T {
    fn create_fragment(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
    ) -> Result<Option<String>> {
        let base = self.base();
        self.create_fragment_with_tags(
            reader,
            doc_id,
            field_name,
            field_frag_list,
            &base.pre_tags,
            &base.post_tags,
            &NULL_ENCODER,
        )
    }

    fn create_fragments(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
        max_num_fragments: i32,
    ) -> Result<Option<Vec<String>>> {
        let base = self.base();
        self.create_fragments_with_tags(
            reader,
            doc_id,
            field_name,
            field_frag_list,
            max_num_fragments,
            &base.pre_tags,
            &base.post_tags,
            &NULL_ENCODER,
        )
    }

    fn create_fragment_with_tags(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<String>> {
        let fragments = self.create_fragments_with_tags(
            reader,
            doc_id,
            field_name,
            field_frag_list,
            1,
            pre_tags,
            post_tags,
            encoder,
        )?;
        match fragments {
            Some(mut fragments) if !fragments.is_empty() => Ok(Some(fragments.swap_remove(0))),
            _ => Ok(None),
        }
    }

    fn create_fragments_with_tags(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
        max_num_fragments: i32,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<Vec<String>>> {
        if max_num_fragments < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxNumFragments({max_num_fragments}) must be positive number."
            )));
        }

        let values = self.get_fields(reader, doc_id, field_name)?;
        if values.is_empty() {
            return Ok(None);
        }

        // Java reads the frag list's own list, which the builder may sort
        // and edit in place; the list is taken, worked on and put back.
        let mut frag_infos: Vec<WeightedFragInfo> =
            std::mem::take(field_frag_list.get_frag_infos_mut());
        let discrete = self.base().discrete_multi_value_highlighting && values.len() > 1;
        let frag_infos = if discrete {
            let result = self.discrete_multi_value_highlighting(&mut frag_infos, &values);
            *field_frag_list.get_frag_infos_mut() = frag_infos;
            self.get_weighted_frag_info_list(result)
        } else {
            let sorted = self.get_weighted_frag_info_list(frag_infos);
            *field_frag_list.get_frag_infos_mut() = sorted.clone();
            sorted
        };
        let limit_fragments = if (max_num_fragments as usize) < frag_infos.len() {
            max_num_fragments as usize
        } else {
            frag_infos.len()
        };
        let mut fragments: Vec<String> = Vec::with_capacity(limit_fragments);

        let mut buffer: Vec<u16> = Vec::new();
        let mut next_value_index = 0;
        for frag_info in &frag_infos[..limit_fragments] {
            fragments.push(self.make_fragment(
                &mut buffer,
                &mut next_value_index,
                &values,
                frag_info,
                pre_tags,
                post_tags,
                encoder,
            )?);
        }
        Ok(Some(fragments))
    }
}
