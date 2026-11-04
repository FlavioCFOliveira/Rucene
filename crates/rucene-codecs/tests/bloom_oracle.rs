//! Oracle parity of `org.apache.lucene.codecs.bloom` (rmp #337:
//! `BloomFilteringPostingsFormat`, `FuzzySet`, `DefaultBloomFilterFactory`)
//! against Apache Lucene 10.5.0.
//!
//! The Java oracle (`PostingsFormatOracle bloom postings
//! BloomFilter(Lucene104) blm`, in the workspace harness
//! `crates/randomizedtesting/tests/fixtures/oracle/`) writes the corpus of
//! `SimpleTextOracle` with `Lucene104Codec` routing every field but `e_id` to
//! `new BloomFilteringPostingsFormat(new Lucene104PostingsFormat())`. The
//! tests write the same corpus with Rucene and compare every file of the
//! format byte for byte (the `.blm` filters in the writer's order, see
//! [`blm_in_rucenes_order`]), read both sides' files with Rucene, run
//! Lucene's reader on Rucene's files, compare the errors of corrupted copies
//! and resolve the format's service name.

mod oracle;

use std::sync::Arc;

use oracle::*;
use rucene::codecs::{Codec, Lucene104PostingsFormat};
use rucene::store::{ByteArrayDataInput, DataInput};
use rucene::util::resource_loader::ClassLoader;
use rucene_codecs::bloom::{BloomFilteringPostingsFormat, FuzzySet};

fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_codecs::class_loader()
}

fn bloom_format() -> BloomFilteringPostingsFormat {
    BloomFilteringPostingsFormat::new(Arc::new(Lucene104PostingsFormat::new()))
}

fn bloom_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(bloom_format()))
}

/// The delegate keeps its postings in `Lucene104PostingsWriter`'s files, so
/// the dump's `live` lines meet core defect 1.
const BLOOM: FormatOracle = FormatOracle {
    prefix: "bloom",
    format_name: "BloomFilter",
    extensions: &["blm"],
    codec: bloom_codec,
    extra_jars: &[],
    live_impacts_differ: true,
};

/// A `.blm` file cut into its header and delegate name, its filters
/// (`FieldNumber, FuzzySet`, each with its field number) and its footer.
struct Blm {
    head: Vec<u8>,
    filters: Vec<(i32, Vec<u8>)>,
    footer: Vec<u8>,
}

fn parse_blm(bytes: &[u8]) -> Blm {
    let header_end = id_offset(bytes) + 16;
    let header_end = header_end + 1 + bytes[header_end] as usize;
    let mut input = ByteArrayDataInput::new(&bytes[header_end..]);
    input.read_string().unwrap();
    let num = input.read_int().unwrap();
    let head = bytes[..header_end + input.position()].to_vec();
    let mut filters = Vec::new();
    for _ in 0..num {
        let start = header_end + input.position();
        let field = input.read_int().unwrap();
        FuzzySet::deserialize(&mut input).unwrap();
        filters.push((field, bytes[start..header_end + input.position()].to_vec()));
    }
    let end = header_end + input.position();
    assert_eq!(bytes.len() - end, 16, "the footer follows the filters");
    Blm {
        head,
        filters,
        footer: bytes[end..].to_vec(),
    }
}

/// Lucene's `.blm` with its filters in the order of Rucene's: Lucene writes
/// them in `HashMap<FieldInfo, FuzzySet>` order, which follows identity
/// hash codes (the `nocfs` and `cfs` fixtures hold segment `_5`'s three
/// filters in two orders). Every filter of one side must be byte for byte a
/// filter of the other; the footer's CRC-32 is recomputed.
fn blm_in_rucenes_order(name: &str, java: Vec<u8>, rust: &[u8]) -> Vec<u8> {
    if !name.ends_with(".blm") {
        return java;
    }
    let java_blm = parse_blm(&java);
    let rust_blm = parse_blm(rust);
    let mut reordered = java_blm.head.clone();
    assert_eq!(
        java_blm.filters.len(),
        rust_blm.filters.len(),
        "{name}: filter count"
    );
    for (field, _) in &rust_blm.filters {
        let (_, filter) = java_blm
            .filters
            .iter()
            .find(|(f, _)| f == field)
            .unwrap_or_else(|| panic!("{name}: Lucene wrote no filter for field {field}"));
        reordered.extend_from_slice(filter);
    }
    reordered.extend_from_slice(&java_blm.footer);
    with_fixed_footer(reordered)
}

#[test]
fn bloom_writes_lucenes_bytes() {
    BLOOM.assert_writes_lucenes_bytes_with(blm_in_rucenes_order);
}

/// Rucene writes the filters in the order the fields were filtered, the
/// order of the `Fields` it was given, which for the corpus is the order of
/// the field names (`String.compareTo`) in every segment, whatever order
/// Lucene chose.
#[test]
fn bloom_writes_filters_in_field_order() {
    let tmp = TempDir::new("bloom-order");
    BLOOM.write_case("nocfs", &tmp.0);
    let reader = rucene::index::directory_reader::open(open_dir(&tmp.0)).unwrap();
    let leaves = reader.leaves().unwrap();
    let infos = leaves[0].leaf_reader().get_field_infos();
    let mut filtered: Vec<(String, i32)> = infos
        .iter()
        .filter(|fi| {
            fi.get_index_options() != rucene::index::IndexOptions::NONE && fi.name != "e_id"
        })
        .map(|fi| (fi.name.clone(), fi.number))
        .collect();
    filtered.sort_by(|a, b| rucene::util::java_collections::java_compare_to(&a.0, &b.0));
    let by_name: Vec<i32> = filtered.iter().map(|(_, n)| *n).collect();
    let files = read_dir(&tmp.0);
    let mut segments = 0;
    for (name, bytes) in &files {
        if name.ends_with(".blm") {
            let fields: Vec<i32> = parse_blm(bytes).filters.iter().map(|(f, _)| *f).collect();
            assert_eq!(fields, by_name, "{name}: filters in field-name order");
            segments += 1;
        }
    }
    assert_eq!(segments, 3);
}

#[test]
fn bloom_reads_what_lucene_reads() {
    BLOOM.assert_reads_what_lucene_reads();
}

/// Core defect 5, outside this crate: `BloomFilteredTerms.intersect` is the
/// delegate's, `Lucene103`'s `IntersectTermsEnum`, whose `get_state`
/// (`src/codecs/lucene103/intersect_terms_enum.rs`) checks Java's `assert
/// state != -1` (`IntersectTermsEnum.java:235`) with `debug_assert!`, a
/// panic, where Java under `-ea` throws `AssertionError`, which the oracle
/// lists as `!AssertionError`. The trace lists the panic as that error.
#[cfg(debug_assertions)]
#[test]
fn bloom_enum_trace_matches_lucenes() {
    BLOOM.assert_enum_trace_matches_but_core_panic("assertion failed: state != -1");
}

#[test]
fn lucene_reads_what_bloom_writes() {
    BLOOM.assert_lucene_reads_what_rucene_writes();
}

#[test]
fn bloom_corrupted_files_raise_lucenes_errors() {
    assert!(BLOOM
        .assert_corrupted_files_raise_lucenes_errors()
        .is_empty());
}

/// `PostingsFormat.forName("BloomFilter")` resolves once the crate's service
/// files are loaded, to the format built with the no-argument constructor
/// (`delegatePostingsFormat == null`).
#[test]
fn bloom_resolves_by_name() {
    install_codecs();
    let spi = BLOOM.spi();
    let format = rucene::codecs::postings_for_name("BloomFilter").expect("BloomFilter resolves");
    assert_eq!(format.name(), spi["getName"]);
    assert_eq!(bloom_format().to_string(), spi["toString"]);
    assert_eq!(
        BloomFilteringPostingsFormat::for_reading().to_string(),
        spi["forName.toString"]
    );
    assert_eq!(
        spi["forName.getClass"],
        "org.apache.lucene.codecs.bloom.BloomFilteringPostingsFormat"
    );
}

/// The format `forName` returns cannot write: Java's `fieldsConsumer`
/// throws `UnsupportedOperationException`.
#[test]
fn bloom_for_reading_cannot_write() {
    let format = BloomFilteringPostingsFormat::for_reading();
    let tmp = TempDir::new("bloom-for-reading");
    let codec = per_field_postings_codec(Arc::new(format));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        write_corpus(&tmp.0, false, codec)
    }));
    assert!(result.is_err(), "writing with the read-only format fails");
}

/// Rucene's `FuzzySet` hashes a value to the bits Lucene's set: every term
/// of every filtered field of Lucene's segments is `MAYBE` in the filter
/// Lucene wrote for it, and the filters reject values they never saw.
#[test]
fn lucenes_filters_hold_every_term_of_their_field() {
    use rucene_codecs::bloom::ContainsResult;
    let tmp = TempDir::new("bloom-filters");
    let java = java_files("bloom", "nocfs");
    write_files(&tmp.0, &java);
    install_codecs();
    let infos = rucene::index::SegmentInfos::read_latest_commit(open_dir(&tmp.0)).unwrap();
    let reader = rucene::index::directory_reader::open(open_dir(&tmp.0)).unwrap();
    let leaves = reader.leaves().unwrap();
    let (mut present, mut rejected) = (0, 0);
    for (i, ctx) in leaves.iter().enumerate() {
        let segment = infos.info(i).info.name.clone();
        let blm = parse_blm(&java[&format!("{segment}_BloomFilter_0.blm")]);
        let leaf = ctx.leaf_reader();
        for (field, bytes) in &blm.filters {
            let set = FuzzySet::deserialize(&mut ByteArrayDataInput::new(&bytes[4..])).unwrap();
            let fi = leaf
                .get_field_infos()
                .field_info_by_number(*field)
                .unwrap()
                .clone();
            let terms = leaf.terms(&fi.name).unwrap().unwrap();
            let mut te = terms.iterator().unwrap();
            while let Some(term) = te.next().unwrap() {
                assert_eq!(
                    set.contains(&term),
                    ContainsResult::MAYBE,
                    "{segment}: {}: {term:?}",
                    fi.name
                );
                present += 1;
            }
            for probe in 0..100 {
                let absent = rucene::util::BytesRef::new(format!("absent-{probe}").into_bytes());
                if set.contains(&absent) == ContainsResult::NO {
                    rejected += 1;
                }
            }
        }
    }
    assert!(
        present > 0 && rejected > 0,
        "{present} present, {rejected} rejected"
    );
}
