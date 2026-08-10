//! Bundled Lucene 9.9 sub-formats reused by the default `Lucene104` codec.

pub mod segment_info_format;

pub use segment_info_format::Lucene99SegmentInfoFormat;
