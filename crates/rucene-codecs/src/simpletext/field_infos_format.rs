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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldInfosFormat`.

use rucene::codecs::FieldInfosFormat;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, FieldInfos, IndexOptions, SegmentInfo,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::store::{Directory, IOContext};
use rucene::util::java_collections::{JavaHashMap, JavaStringMap};

use super::util::{self, bool_str, parse_boolean, parse_int, parse_long, read_line, read_string};

/// Extension of field infos.
pub(crate) const FIELD_INFOS_EXTENSION: &str = "inf";

pub(crate) const NUMFIELDS: &[u8] = b"number of fields ";
pub(crate) const NAME: &[u8] = b"  name ";
pub(crate) const NUMBER: &[u8] = b"  number ";
pub(crate) const STORETV: &[u8] = b"  term vectors ";
pub(crate) const STORETVPOS: &[u8] = b"  term vector positions ";
pub(crate) const STORETVOFF: &[u8] = b"  term vector offsets ";
pub(crate) const PAYLOADS: &[u8] = b"  payloads ";
pub(crate) const NORMS: &[u8] = b"  norms ";
pub(crate) const DOCVALUES: &[u8] = b"  doc values ";
pub(crate) const DOCVALUES_SKIP_INDEX: &[u8] = b"  doc values skip index";
pub(crate) const DOCVALUES_GEN: &[u8] = b"  doc values gen ";
pub(crate) const INDEXOPTIONS: &[u8] = b"  index options ";
pub(crate) const NUM_ATTS: &[u8] = b"  attributes ";
pub(crate) const ATT_KEY: &[u8] = b"    key ";
pub(crate) const ATT_VALUE: &[u8] = b"    value ";
pub(crate) const DATA_DIM_COUNT: &[u8] = b"  data dimensional count ";
pub(crate) const INDEX_DIM_COUNT: &[u8] = b"  index dimensional count ";
pub(crate) const DIM_NUM_BYTES: &[u8] = b"  dimensional num bytes ";
pub(crate) const VECTOR_NUM_DIMS: &[u8] = b"  vector number of dimensions ";
pub(crate) const VECTOR_ENCODING: &[u8] = b"  vector encoding ";
pub(crate) const VECTOR_SIMILARITY: &[u8] = b"  vector similarity ";
pub(crate) const SOFT_DELETES: &[u8] = b"  soft-deletes ";
pub(crate) const PARENT: &[u8] = b"  parent ";

// Unused by the format, kept as Java declares them.
const _: (&[u8], &[u8]) = (STORETVPOS, STORETVOFF);

/// Plaintext field infos format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextFieldInfosFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextFieldInfosFormat;

impl SimpleTextFieldInfosFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }

    /// Port of `SimpleTextFieldInfosFormat.docValuesType`: `DocValuesType.valueOf`.
    ///
    /// # Errors
    ///
    /// Returns `valueOf`'s `IllegalArgumentException` for an unknown name.
    pub fn doc_values_type(&self, dv_type: &str) -> Result<DocValuesType> {
        Ok(match dv_type {
            "NONE" => DocValuesType::NONE,
            "NUMERIC" => DocValuesType::NUMERIC,
            "BINARY" => DocValuesType::BINARY,
            "SORTED" => DocValuesType::SORTED,
            "SORTED_NUMERIC" => DocValuesType::SORTED_NUMERIC,
            "SORTED_SET" => DocValuesType::SORTED_SET,
            _ => return Err(no_enum_constant("DocValuesType", dv_type)),
        })
    }

    /// Port of `SimpleTextFieldInfosFormat.docValuesSkipIndexType`:
    /// `DocValuesSkipIndexType.valueOf`.
    ///
    /// # Errors
    ///
    /// Returns `valueOf`'s `IllegalArgumentException` for an unknown name.
    pub fn doc_values_skip_index_type(
        &self,
        dv_skip_index_type: &str,
    ) -> Result<DocValuesSkipIndexType> {
        Ok(match dv_skip_index_type {
            "NONE" => DocValuesSkipIndexType::NONE,
            "RANGE" => DocValuesSkipIndexType::RANGE,
            _ => {
                return Err(no_enum_constant(
                    "DocValuesSkipIndexType",
                    dv_skip_index_type,
                ))
            }
        })
    }

    /// Port of `SimpleTextFieldInfosFormat.vectorEncoding`: `VectorEncoding.valueOf`.
    ///
    /// # Errors
    ///
    /// Returns `valueOf`'s `IllegalArgumentException` for an unknown name.
    pub fn vector_encoding(&self, vector_encoding: &str) -> Result<VectorEncoding> {
        Ok(match vector_encoding {
            "BYTE" => VectorEncoding::BYTE,
            "FLOAT32" => VectorEncoding::FLOAT32,
            _ => return Err(no_enum_constant("VectorEncoding", vector_encoding)),
        })
    }

    /// Port of `SimpleTextFieldInfosFormat.distanceFunction`:
    /// `VectorSimilarityFunction.valueOf`.
    ///
    /// # Errors
    ///
    /// Returns `valueOf`'s `IllegalArgumentException` for an unknown name.
    pub fn distance_function(&self, score_function: &str) -> Result<VectorSimilarityFunction> {
        Ok(match score_function {
            "EUCLIDEAN" => VectorSimilarityFunction::EUCLIDEAN,
            "DOT_PRODUCT" => VectorSimilarityFunction::DOT_PRODUCT,
            "COSINE" => VectorSimilarityFunction::COSINE,
            "MAXIMUM_INNER_PRODUCT" => VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
            _ => return Err(no_enum_constant("VectorSimilarityFunction", score_function)),
        })
    }
}

/// `IndexOptions.valueOf`.
pub(crate) fn index_options_value_of(s: &str) -> Result<IndexOptions> {
    Ok(match s {
        "NONE" => IndexOptions::NONE,
        "DOCS" => IndexOptions::DOCS,
        "DOCS_AND_FREQS" => IndexOptions::DOCS_AND_FREQS,
        "DOCS_AND_FREQS_AND_POSITIONS" => IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        "DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS" => {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
        }
        "DOCS_AND_CUSTOM_FREQS" => IndexOptions::DOCS_AND_CUSTOM_FREQS,
        _ => return Err(no_enum_constant("IndexOptions", s)),
    })
}

/// `Enum.valueOf`'s `IllegalArgumentException`:
/// `No enum constant org.apache.lucene.index.<Type>.<name>`.
pub(crate) fn no_enum_constant(type_name: &str, name: &str) -> LuceneError {
    LuceneError::IllegalArgument(format!(
        "No enum constant org.apache.lucene.index.{type_name}.{name}"
    ))
}

/// `Enum.name()` / `toString()` of the index enums, whose Rust variants carry
/// the Java constant names.
pub(crate) fn enum_name<T: std::fmt::Debug>(value: T) -> String {
    format!("{value:?}")
}

fn read_str(offset: usize, scratch: &[u8]) -> String {
    read_string(scratch, offset)
}

impl FieldInfosFormat for SimpleTextFieldInfosFormat {
    fn name(&self) -> &str {
        "SimpleTextFieldInfosFormat"
    }

    /// Port of `SimpleTextFieldInfosFormat.read`.
    fn read(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        _iocontext: &dyn IOContext,
    ) -> Result<FieldInfos> {
        let file_name =
            segment_file_name(&segment_info.name, segment_suffix, FIELD_INFOS_EXTENSION);
        let mut input = directory.open_checksum_input(&file_name)?;
        // `success ? input.close() : IOUtils.closeWhileHandlingException(input)`
        // in the `finally` (`SimpleTextFieldInfosFormat.java:85-218`).
        let result = (|| -> Result<_> {
            let mut scratch = Vec::new();

            read_line(input.as_mut(), &mut scratch)?;
            debug_assert!(scratch.starts_with(NUMFIELDS));
            let size = parse_int(&read_str(NUMFIELDS.len(), &scratch))?;
            let mut infos = Vec::with_capacity(size.max(0) as usize);

            for _ in 0..size {
                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(NAME));
                let name = read_str(NAME.len(), &scratch);

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(NUMBER));
                let field_number = parse_int(&read_str(NUMBER.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(INDEXOPTIONS));
                let s = read_str(INDEXOPTIONS.len(), &scratch);
                let index_options = index_options_value_of(&s)?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(STORETV));
                let store_term_vector = parse_boolean(&read_str(STORETV.len(), &scratch));

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(PAYLOADS));
                let store_payloads = parse_boolean(&read_str(PAYLOADS.len(), &scratch));

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(NORMS));
                let omit_norms = !parse_boolean(&read_str(NORMS.len(), &scratch));

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(DOCVALUES));
                let dv_type = read_str(DOCVALUES.len(), &scratch);
                let doc_values_type = self.doc_values_type(&dv_type)?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(DOCVALUES_SKIP_INDEX));
                let doc_value_skipper = self
                    .doc_values_skip_index_type(&read_str(DOCVALUES_SKIP_INDEX.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(DOCVALUES_GEN));
                let dv_gen = parse_long(&read_str(DOCVALUES_GEN.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(NUM_ATTS));
                let num_atts = parse_int(&read_str(NUM_ATTS.len(), &scratch))?;
                let mut atts = JavaHashMap::new();

                for _ in 0..num_atts {
                    read_line(input.as_mut(), &mut scratch)?;
                    debug_assert!(scratch.starts_with(ATT_KEY));
                    let key = read_str(ATT_KEY.len(), &scratch);

                    read_line(input.as_mut(), &mut scratch)?;
                    debug_assert!(scratch.starts_with(ATT_VALUE));
                    let value = read_str(ATT_VALUE.len(), &scratch);
                    atts.insert(key, value);
                }

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(DATA_DIM_COUNT));
                let dimensional_count = parse_int(&read_str(DATA_DIM_COUNT.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(INDEX_DIM_COUNT));
                let index_dimensional_count =
                    parse_int(&read_str(INDEX_DIM_COUNT.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(DIM_NUM_BYTES));
                let dimensional_num_bytes = parse_int(&read_str(DIM_NUM_BYTES.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(VECTOR_NUM_DIMS));
                let vector_num_dimensions = parse_int(&read_str(VECTOR_NUM_DIMS.len(), &scratch))?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(VECTOR_ENCODING));
                let encoding = read_str(VECTOR_ENCODING.len(), &scratch);
                let vector_encoding = self.vector_encoding(&encoding)?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(VECTOR_SIMILARITY));
                let score_function = read_str(VECTOR_SIMILARITY.len(), &scratch);
                let vector_dist_func = self.distance_function(&score_function)?;

                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(SOFT_DELETES));
                let is_soft_deletes_field = parse_boolean(&read_str(SOFT_DELETES.len(), &scratch));
                read_line(input.as_mut(), &mut scratch)?;
                debug_assert!(scratch.starts_with(PARENT));
                let is_parent_field = parse_boolean(&read_str(PARENT.len(), &scratch));

                infos.push(FieldInfo::new_full(
                    name,
                    field_number,
                    store_term_vector,
                    omit_norms,
                    store_payloads,
                    index_options,
                    doc_values_type,
                    doc_value_skipper,
                    dv_gen,
                    JavaStringMap::Hash(atts),
                    dimensional_count,
                    index_dimensional_count,
                    dimensional_num_bytes,
                    vector_num_dimensions,
                    vector_encoding,
                    vector_dist_func,
                    is_soft_deletes_field,
                    is_parent_field,
                )?);
            }

            util::check_footer(input.as_mut())?;

            FieldInfos::new(infos)
        })();
        rucene::store::close_after(input.as_mut(), result)
    }

    /// Port of `SimpleTextFieldInfosFormat.write`.
    fn write(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<()> {
        let file_name =
            segment_file_name(&segment_info.name, segment_suffix, FIELD_INFOS_EXTENSION);
        let mut output = directory.create_output(&file_name, context)?;
        let out = output.as_mut();
        util::write_bytes(out, NUMFIELDS)?;
        util::write_str(out, &infos.size().to_string())?;
        util::write_newline(out)?;

        for fi in infos.iter() {
            util::write_bytes(out, NAME)?;
            util::write_str(out, &fi.name)?;
            util::write_newline(out)?;

            util::write_bytes(out, NUMBER)?;
            util::write_str(out, &fi.number.to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, INDEXOPTIONS)?;
            let index_options = fi.get_index_options();
            debug_assert!(
                index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
                    || !fi.has_payloads()
            );
            util::write_str(out, &enum_name(index_options))?;
            util::write_newline(out)?;

            util::write_bytes(out, STORETV)?;
            util::write_str(out, bool_str(fi.has_term_vectors()))?;
            util::write_newline(out)?;

            util::write_bytes(out, PAYLOADS)?;
            util::write_str(out, bool_str(fi.has_payloads()))?;
            util::write_newline(out)?;

            util::write_bytes(out, NORMS)?;
            util::write_str(out, bool_str(!fi.omits_norms()))?;
            util::write_newline(out)?;

            util::write_bytes(out, DOCVALUES)?;
            util::write_str(out, &enum_name(fi.get_doc_values_type()))?;
            util::write_newline(out)?;

            util::write_bytes(out, DOCVALUES_SKIP_INDEX)?;
            util::write_str(out, &enum_name(fi.doc_values_skip_index_type()))?;
            util::write_newline(out)?;

            util::write_bytes(out, DOCVALUES_GEN)?;
            util::write_str(out, &fi.get_doc_values_gen().to_string())?;
            util::write_newline(out)?;

            let atts = fi.attributes();
            let num_atts = atts.len();
            util::write_bytes(out, NUM_ATTS)?;
            util::write_str(out, &num_atts.to_string())?;
            util::write_newline(out)?;

            for (key, value) in atts.iter() {
                util::write_bytes(out, ATT_KEY)?;
                util::write_str(out, key)?;
                util::write_newline(out)?;

                util::write_bytes(out, ATT_VALUE)?;
                util::write_str(out, value)?;
                util::write_newline(out)?;
            }

            util::write_bytes(out, DATA_DIM_COUNT)?;
            util::write_str(out, &fi.get_point_dimension_count().to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, INDEX_DIM_COUNT)?;
            util::write_str(out, &fi.get_point_index_dimension_count().to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, DIM_NUM_BYTES)?;
            util::write_str(out, &fi.get_point_num_bytes().to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, VECTOR_NUM_DIMS)?;
            util::write_str(out, &fi.get_vector_dimension().to_string())?;
            util::write_newline(out)?;

            util::write_bytes(out, VECTOR_ENCODING)?;
            util::write_str(out, &enum_name(fi.get_vector_encoding()))?;
            util::write_newline(out)?;

            util::write_bytes(out, VECTOR_SIMILARITY)?;
            util::write_str(out, &enum_name(fi.get_vector_similarity_function()))?;
            util::write_newline(out)?;

            util::write_bytes(out, SOFT_DELETES)?;
            util::write_str(out, bool_str(fi.is_soft_deletes_field()))?;
            util::write_newline(out)?;

            util::write_bytes(out, PARENT)?;
            util::write_str(out, bool_str(fi.is_parent_field()))?;
            util::write_newline(out)?;
        }
        util::write_checksum(out)?;
        output.close()
    }
}
