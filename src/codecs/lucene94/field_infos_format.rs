//! Lucene 9.4 field-infos format.
//!
//! Ported from `org.apache.lucene.codecs.lucene94.Lucene94FieldInfosFormat`.

#![deny(unsafe_code)]

use crate::codecs::codec_util;
use crate::codecs::field_infos::FieldInfosFormat;
use crate::codecs::stub::{FieldInfo, FieldInfos, SegmentInfo};
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::{segment_file_name, FIELD_INFO_EXTENSION};
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, VectorEncoding, VectorSimilarityFunction,
};
use crate::store::{DataInput, Directory, IOContext};

const CODEC_NAME: &str = "Lucene94FieldInfos";
const FORMAT_START: i32 = 0;
const FORMAT_PARENT_FIELD: i32 = 1;
const FORMAT_DOCVALUE_SKIPPER: i32 = 2;
const FORMAT_CURRENT: i32 = FORMAT_DOCVALUE_SKIPPER;

const STORE_TERMVECTOR: u8 = 0x1;
const OMIT_NORMS: u8 = 0x2;
const STORE_PAYLOADS: u8 = 0x4;
const SOFT_DELETES_FIELD: u8 = 0x8;
const PARENT_FIELD_FIELD: u8 = 0x10;
const DOCVALUES_SKIPPER: u8 = 0x20;

/// Lucene 9.4 field-infos format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene94.Lucene94FieldInfosFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lucene94FieldInfosFormat;

impl Lucene94FieldInfosFormat {
    /// Creates a new field-infos format instance.
    pub fn new() -> Self {
        Self
    }
}

impl FieldInfosFormat for Lucene94FieldInfosFormat {
    fn name(&self) -> &str {
        "Lucene94FieldInfos"
    }

    fn read(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<FieldInfos> {
        let file_name = segment_file_name(&segment_info.name, segment_suffix, FIELD_INFO_EXTENSION);
        let mut input = directory.open_checksum_input(&file_name)?;

        let format = codec_util::check_index_header(
            input.as_mut(),
            CODEC_NAME,
            FORMAT_START,
            FORMAT_CURRENT,
            &segment_info.id(),
            segment_suffix,
        )?;

        let size = input.read_v_int()?;
        let mut infos = Vec::with_capacity(size as usize);
        for _ in 0..size {
            infos.push(read_field_info(input.as_mut(), format)?);
        }

        codec_util::check_footer(input.as_mut())?;
        FieldInfos::new(infos)
    }

    fn write(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<()> {
        let file_name = segment_file_name(&segment_info.name, segment_suffix, FIELD_INFO_EXTENSION);
        let mut output = directory.create_output(&file_name, context)?;
        codec_util::write_index_header(
            output.as_mut(),
            CODEC_NAME,
            FORMAT_CURRENT,
            &segment_info.id(),
            segment_suffix,
        )?;

        output.write_v_int(infos.iter().count() as i32)?;
        for fi in infos.iter() {
            fi.write(output.as_mut())?;
        }

        codec_util::write_footer(output.as_mut())?;
        output.close()?;
        Ok(())
    }
}

fn read_field_info(input: &mut dyn DataInput, format: i32) -> Result<FieldInfo> {
    let name = input.read_string()?;
    let field_number = input.read_v_int()?;
    if field_number < 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "invalid field number for field: {name}, fieldNumber={field_number}"
        )));
    }

    let bits = input.read_byte()?;
    let store_term_vector = (bits & STORE_TERMVECTOR) != 0;
    let omit_norms = (bits & OMIT_NORMS) != 0;
    let store_payloads = (bits & STORE_PAYLOADS) != 0;
    let soft_deletes_field = (bits & SOFT_DELETES_FIELD) != 0;
    let is_parent_field = if format >= FORMAT_PARENT_FIELD {
        (bits & PARENT_FIELD_FIELD) != 0
    } else {
        false
    };

    if (bits & 0xC0) != 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "unused bits are set \"{bits:08b}\""
        )));
    }
    if format < FORMAT_PARENT_FIELD && (bits & 0xF0) != 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "parent field bit is set but shouldn't \"{bits:08b}\""
        )));
    }
    if format < FORMAT_DOCVALUE_SKIPPER && (bits & DOCVALUES_SKIPPER) != 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "doc values skipper bit is set but shouldn't \"{bits:08b}\""
        )));
    }

    let index_options = index_options_from_byte(input.read_byte()?)?;
    let doc_values_type = doc_values_type_from_byte(input.read_byte()?)?;
    let doc_values_skip_index = if format >= FORMAT_DOCVALUE_SKIPPER {
        doc_values_skip_index_type_from_byte(input.read_byte()?)?
    } else {
        DocValuesSkipIndexType::NONE
    };
    let dv_gen = input.read_long()?;
    let attributes = input.read_map_of_strings()?;

    let point_data_dimension_count = input.read_v_int()?;
    let (point_index_dimension_count, point_num_bytes) = if point_data_dimension_count != 0 {
        (input.read_v_int()?, input.read_v_int()?)
    } else {
        (0, 0)
    };

    let vector_dimension = input.read_v_int()?;
    let vector_encoding = vector_encoding_from_byte(input.read_byte()?)?;
    let vector_similarity_function = vector_similarity_function_from_byte(input.read_byte()?)?;

    FieldInfo::new_full(
        name,
        field_number,
        store_term_vector,
        omit_norms,
        store_payloads,
        index_options,
        doc_values_type,
        doc_values_skip_index,
        dv_gen,
        attributes,
        point_data_dimension_count,
        point_index_dimension_count,
        point_num_bytes,
        vector_dimension,
        vector_encoding,
        vector_similarity_function,
        soft_deletes_field,
        is_parent_field,
    )
}

fn index_options_from_byte(b: u8) -> Result<IndexOptions> {
    match b {
        0 => Ok(IndexOptions::NONE),
        1 => Ok(IndexOptions::DOCS),
        2 => Ok(IndexOptions::DOCS_AND_FREQS),
        3 => Ok(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS),
        4 => Ok(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS),
        5 => Ok(IndexOptions::DOCS_AND_CUSTOM_FREQS),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid IndexOptions byte: {b}"
        ))),
    }
}

fn doc_values_type_from_byte(b: u8) -> Result<DocValuesType> {
    match b {
        0 => Ok(DocValuesType::NONE),
        1 => Ok(DocValuesType::NUMERIC),
        2 => Ok(DocValuesType::BINARY),
        3 => Ok(DocValuesType::SORTED),
        4 => Ok(DocValuesType::SORTED_SET),
        5 => Ok(DocValuesType::SORTED_NUMERIC),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid docvalues byte: {b}"
        ))),
    }
}

fn doc_values_skip_index_type_from_byte(b: u8) -> Result<DocValuesSkipIndexType> {
    match b {
        0 => Ok(DocValuesSkipIndexType::NONE),
        1 => Ok(DocValuesSkipIndexType::RANGE),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid docvaluesskipindex byte: {b}"
        ))),
    }
}

fn vector_encoding_from_byte(b: u8) -> Result<VectorEncoding> {
    match b {
        0 => Ok(VectorEncoding::BYTE),
        1 => Ok(VectorEncoding::FLOAT32),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid vector encoding: {b}"
        ))),
    }
}

fn vector_similarity_function_from_byte(b: u8) -> Result<VectorSimilarityFunction> {
    match b {
        0 => Ok(VectorSimilarityFunction::EUCLIDEAN),
        1 => Ok(VectorSimilarityFunction::DOT_PRODUCT),
        2 => Ok(VectorSimilarityFunction::COSINE),
        3 => Ok(VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid vector similarity function: {b}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    /// The four serialization sites for `VectorEncoding` and
    /// `VectorSimilarityFunction` each carry their own ordinal table. Java has
    /// one, `Enum.ordinal()`, so the tables must agree with the declaration
    /// order of the Rust enums and therefore with each other; a divergence here
    /// silently writes an unreadable index.
    #[test]
    fn vector_ordinals_match_the_enum_declaration_order() {
        use crate::index::{VectorEncoding, VectorSimilarityFunction};

        for encoding in [VectorEncoding::BYTE, VectorEncoding::FLOAT32] {
            let ordinal = encoding as i32;
            assert_eq!(
                super::vector_encoding_from_byte(ordinal as u8).unwrap(),
                encoding
            );
        }

        for similarity in [
            VectorSimilarityFunction::EUCLIDEAN,
            VectorSimilarityFunction::DOT_PRODUCT,
            VectorSimilarityFunction::COSINE,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        ] {
            let ordinal = similarity as i32;
            assert_eq!(
                super::vector_similarity_function_from_byte(ordinal as u8).unwrap(),
                similarity
            );
        }
    }

    use super::*;
    use crate::codecs::tests::test_segment_info;
    use crate::store::RamDirectory;
    use std::collections::HashMap;

    #[test]
    fn round_trip_field_infos() {
        let dir = RamDirectory::default();
        let format = Lucene94FieldInfosFormat::new();
        let info = test_segment_info("_0", 10);

        let fi1 = FieldInfo::new("id", 0);
        let fi2 = FieldInfo::new_full(
            "title",
            1,
            true,
            false,
            false,
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            [("foo".to_string(), "bar".to_string())]
                .into_iter()
                .collect(),
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .unwrap();
        let fi3 = FieldInfo::new_full(
            "vector",
            2,
            false,
            false,
            false,
            IndexOptions::NONE,
            DocValuesType::NUMERIC,
            DocValuesSkipIndexType::NONE,
            -1,
            HashMap::new(),
            0,
            0,
            0,
            128,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::COSINE,
            false,
            false,
        )
        .unwrap();

        let infos = FieldInfos::new(vec![fi1.clone(), fi2.clone(), fi3.clone()]).unwrap();
        format
            .write(&dir, &info, "", &infos, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let read = format
            .read(&dir, &info, "", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();

        assert_eq!(read.iter().count(), 3);
        assert_eq!(read.field_info("id").unwrap().number, 0);
        assert_eq!(read.field_info("title").unwrap().number, 1);
        assert_eq!(
            read.field_info("title").unwrap().get_attribute("foo"),
            Some("bar".to_string())
        );
        assert_eq!(read.field_info("vector").unwrap().vector_dimension, 128);
    }
}
