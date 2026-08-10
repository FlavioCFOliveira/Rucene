//! Lucene 9.9 segment-info format.
//!
//! Ported from `org.apache.lucene.codecs.lucene99.Lucene99SegmentInfoFormat`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::codec_util;
use crate::codecs::segment_info::SegmentInfoFormat;
use crate::codecs::stub::SegmentInfo;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::{
    parse_segment_name, segment_file_name, SEGMENT_INFO_EXTENSION,
};
use crate::search::write_sort;
use crate::store::{DataInput, Directory, IOContext, IndexOutput, RamDirectory};
use crate::util::Version;

const CODEC_NAME: &str = "Lucene90SegmentInfo";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;

/// Lucene 9.9 segment-info format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99SegmentInfoFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lucene99SegmentInfoFormat;

impl Lucene99SegmentInfoFormat {
    /// Creates a new segment-info format instance.
    pub fn new() -> Self {
        Self
    }
}

impl SegmentInfoFormat for Lucene99SegmentInfoFormat {
    fn name(&self) -> &str {
        "Lucene99SegmentInfo"
    }

    fn read(
        &self,
        directory: &dyn Directory,
        segment_name: &str,
        segment_id: &[u8],
        _context: &dyn IOContext,
    ) -> Result<SegmentInfo> {
        let file_name = segment_file_name(segment_name, "", SEGMENT_INFO_EXTENSION);
        let mut input = directory.open_checksum_input(&file_name)?;

        let result = (|| {
            codec_util::check_index_header(
                input.as_mut(),
                CODEC_NAME,
                VERSION_START,
                VERSION_CURRENT,
                segment_id,
                "",
            )?;
            parse_segment_info(directory, input.as_mut(), segment_name, segment_id)
        })();
        codec_util::check_footer(input.as_mut())?;
        result
    }

    fn write(
        &self,
        directory: &dyn Directory,
        info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<()> {
        let file_name = segment_file_name(&info.name, "", SEGMENT_INFO_EXTENSION);
        let mut output = directory.create_output(&file_name, context)?;
        info.add_files(std::slice::from_ref(&file_name));

        codec_util::write_index_header(
            output.as_mut(),
            CODEC_NAME,
            VERSION_CURRENT,
            &info.id(),
            "",
        )?;
        write_segment_info(output.as_mut(), info)?;
        codec_util::write_footer(output.as_mut())?;
        output.close()?;
        Ok(())
    }
}

fn parse_segment_info(
    _directory: &dyn Directory,
    input: &mut dyn DataInput,
    segment_name: &str,
    segment_id: &[u8],
) -> Result<SegmentInfo> {
    let version = Version::from_bits(
        input.read_int()? as u8,
        input.read_int()? as u8,
        input.read_int()? as u8,
    )?;
    if version.major < 7 {
        return Err(LuceneError::IllegalArgument(format!(
            "invalid major version: should be >= 7 but got: {} segment={segment_name}",
            version.major
        )));
    }

    let has_min_version = input.read_byte()?;
    let min_version = match has_min_version {
        0 => None,
        1 => Some(Version::from_bits(
            input.read_int()? as u8,
            input.read_int()? as u8,
            input.read_int()? as u8,
        )?),
        _ => {
            return Err(LuceneError::CorruptIndex(format!(
                "illegal boolean value {has_min_version}"
            )))
        }
    };

    let doc_count = input.read_int()?;
    if doc_count < 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "invalid docCount: {doc_count}"
        )));
    }

    let is_compound_file = input.read_byte()? == SegmentInfo::YES as u8;
    let has_blocks = input.read_byte()? == SegmentInfo::YES as u8;

    let diagnostics = input.read_map_of_strings()?;
    let files = input.read_set_of_strings()?;
    let attributes = input.read_map_of_strings()?;
    let index_sort = read_segment_sort(input)?;

    let id: [u8; 16] = segment_id
        .try_into()
        .map_err(|_| LuceneError::IllegalArgument("segment id must be 16 bytes".to_string()))?;

    // The directory reference in SegmentInfo is used for path information; on
    // read we attach a fresh in-memory directory placeholder. The real files are
    // accessed through the directory passed to the format methods.
    let directory = Arc::new(RamDirectory::default()) as Arc<dyn Directory>;

    let info = SegmentInfo::new_without_codec(
        directory,
        version,
        min_version,
        segment_name.to_string(),
        doc_count,
        is_compound_file,
        has_blocks,
        diagnostics,
        id,
        attributes,
        index_sort.unwrap_or_default(),
    )?;
    info.set_files(files);
    Ok(info)
}

fn read_segment_sort(input: &mut dyn DataInput) -> Result<Option<crate::search::Sort>> {
    let count = input.read_v_int()?;
    if count < 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "invalid index sort field count: {count}"
        )));
    }
    if count == 0 {
        return Ok(None);
    }
    let mut fields = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let provider = input.read_string()?;
        if provider != "SortField" {
            return Err(LuceneError::IllegalArgument(format!(
                "unknown sort field provider: {provider}"
            )));
        }
        fields.push(crate::search::SortField::deserialize(input)?);
    }
    Ok(Some(crate::search::Sort::new_fields(fields)?))
}

fn write_segment_info(output: &mut dyn IndexOutput, info: &SegmentInfo) -> Result<()> {
    let version = info.version();
    if version.major < 7 {
        return Err(LuceneError::IllegalArgument(format!(
            "invalid major version: should be >= 7 but got: {} segment={}",
            version.major, info.name
        )));
    }

    output.write_int(version.major as i32)?;
    output.write_int(version.minor as i32)?;
    output.write_int(version.bugfix as i32)?;

    if let Some(min_version) = info.min_version() {
        output.write_byte(1)?;
        output.write_int(min_version.major as i32)?;
        output.write_int(min_version.minor as i32)?;
        output.write_int(min_version.bugfix as i32)?;
    } else {
        output.write_byte(0)?;
    }

    output.write_int(info.max_doc()?)?;
    output.write_byte(if info.get_use_compound_file() {
        SegmentInfo::YES as u8
    } else {
        SegmentInfo::NO as u8
    })?;
    output.write_byte(if info.get_has_blocks() {
        SegmentInfo::YES as u8
    } else {
        SegmentInfo::NO as u8
    })?;

    output.write_map_of_strings(info.get_diagnostics())?;

    let files = info.files()?;
    for file in &files {
        if parse_segment_name(file) != info.name {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid files: expected segment={}, got={file}",
                info.name
            )));
        }
    }
    output.write_set_of_strings(&files)?;
    output.write_map_of_strings(&info.get_attributes())?;

    write_sort(output, info.index_sort())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::tests::test_segment_info;
    use crate::search::{Sort, SortField, SortFieldType};
    use crate::store::RamDirectory;
    use std::collections::HashSet;

    #[test]
    fn round_trip_segment_info() {
        let dir = RamDirectory::default();
        let format = Lucene99SegmentInfoFormat::new();
        let mut info = test_segment_info("_0", 42);
        info.set_diagnostics(
            [("source".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
        );
        info.set_files(HashSet::from(["_0.fnm".to_string(), "_0.fdt".to_string()]));
        info.set_use_compound_file(true);
        info.set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );

        format
            .write(&dir, &info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();

        let read = format
            .read(&dir, "_0", &info.id(), &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();

        assert_eq!(read.name, info.name);
        assert_eq!(read.max_doc().unwrap(), info.max_doc().unwrap());
        assert_eq!(read.get_use_compound_file(), info.get_use_compound_file());
        assert_eq!(read.get_has_blocks(), info.get_has_blocks());
        assert_eq!(read.version(), info.version());
        assert_eq!(read.min_version(), info.min_version());

        let mut read_files: Vec<String> = read.files().unwrap().into_iter().collect();
        let mut info_files: Vec<String> = info.files().unwrap().into_iter().collect();
        read_files.sort();
        info_files.sort();
        assert_eq!(read_files, info_files);

        assert_eq!(read.get_diagnostics(), info.get_diagnostics());
        assert_eq!(*read.index_sort(), *info.index_sort());
    }
}
