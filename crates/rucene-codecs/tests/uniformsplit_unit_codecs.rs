//! The `lucene/codecs` uniform-split unit test classes that extend
//! `LuceneTestCase` (rmp #361), run as suites by the ported runner:
//! `TestTermBytes`, `TestBlockWriter`, `TestFSTDictionary`,
//! `TestTermBytesComparator` (`org.apache.lucene.codecs.uniformsplit`) and
//! `sharedterms.TestSTBlockReader`. A Java subclass's overrides of
//! `BlockReader`'s protected methods are `BlockReaderOverrides`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::exception::{java_assert, throw_without_message, Throwable};
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::state::SegmentReadState;
use rucene::codecs::{BlockTermState, PostingsReaderBase};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, FieldInfos, ImpactsEnum, IndexOptions,
    PostingsEnum, SeekStatus, TermsEnum, VectorEncoding, VectorSimilarityFunction,
};
use rucene::store::{
    ByteBuffersDataOutput, ByteBuffersDirectory, ByteBuffersIndexInput, ByteBuffersIndexOutput,
    DataInput, Directory, IndexInput, DEFAULT_IO_CONTEXT,
};
use rucene::util::BytesRef;
use rucene_codecs::uniformsplit::block_line::ReadLine;
use rucene_codecs::uniformsplit::block_writer::{FieldContext, WriterLine};
use rucene_codecs::uniformsplit::fst_dictionary;
use rucene_codecs::uniformsplit::index_dictionary::{Browser, BrowserSupplier, Builder};
use rucene_codecs::uniformsplit::sharedterms::{
    FieldMetadataTermState, STBlockLine, STBlockReader,
};
use rucene_codecs::uniformsplit::term_bytes::TermBytes;
use rucene_codecs::uniformsplit::{
    BlockHeader, BlockLine, BlockReader, BlockReaderOverrides, BlockWriter, FSTDictionary,
    FieldMetadata, IndexDictionary,
};
use rucene_test_framework::codecs::uniformsplit::rot13_cypher_test_util::Rot13CypherTestUtil;
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::LuceneTestCase;

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

// -----------------------------------------------------------------------------
// TestTermBytes
// -----------------------------------------------------------------------------

fn test_mdpa() -> Result<()> {
    validate_expected_mdp(&[
        ["aa", "a"],
        ["abbreviator", "ab"],
        ["abidingly", "abi"],
        ["aboiteaus", "abo"],
        ["abranchiates", "abr"],
        ["absentminded", "abs"],
    ])
}

fn test_incremental_a() -> Result<()> {
    validate_expected_suffix(&[
        ["aa", "0aa"],
        ["abbreviator", "1bbreviator"],
        ["abidingly", "2idingly"],
        ["aboiteaus", "2oiteaus"],
        ["abranchiates", "2ranchiates"],
        ["absentminded", "2sentminded"],
        ["rodriguez", "0rodriguez"],
        ["romero", "2mero"],
    ])
}

fn test_mdpmix2() -> Result<()> {
    validate_expected_mdp(&[
        ["abaco", "a"],
        ["amigo", "am"],
        ["bloom", "b"],
        ["break", "br"],
        ["can", "c"],
        ["car", "car"],
        ["carmagedon", "carm"],
        ["danger", "d"],
        ["lala", "l"],
        ["literature", "li"],
        ["lucene", "lu"],
        ["nature", "n"],
        ["naval", "nav"],
        ["rico", "r"],
        ["weird", "w"],
        ["zoo", "z"],
    ])
}

fn test_mdp() -> Result<()> {
    validate_expected_mdp(&[
        ["abaco", "a"],
        ["amigo", "am"],
        ["arco", "ar"],
        ["bloom", "b"],
        ["frien", "f"],
        ["frienchies", "frienc"],
        ["friend", "friend"],
        ["friendalan", "frienda"],
        ["friende", "friende"],
    ])
}

fn test_incremental() -> Result<()> {
    validate_expected_suffix(&[
        ["abaco", "0abaco"],
        ["amigo", "1migo"],
        ["arco", "1rco"],
        ["bloom", "0bloom"],
        ["frien", "0frien"],
        ["frienchies", "5chies"],
        ["friend", "5d"],
        ["friendalan", "6alan"],
        ["friende", "6e"],
    ])
}

fn test_incremental_simple() -> Result<()> {
    validate_expected_suffix(&[
        ["abaco", "0abaco"],
        ["rodriguez", "0rodriguez"],
        ["roma", "2ma"],
        ["romero", "3ero"],
    ])
}

fn test_mdp_simple() -> Result<()> {
    validate_expected_mdp(&[["abaco", "a"], ["rodriguez", "r"], ["romero", "rom"]])
}

fn test_mdpmix() -> Result<()> {
    validate_expected_mdp(&[
        ["aaab", "a"],
        ["arco", "ar"],
        ["busqueda", "b"],
        ["trabajo", "t"],
        ["zufix", "z"],
        ["zzfix", "zz"],
    ])
}

/// `validateExpectedSuffix(String[][] vocab)`.
fn validate_expected_suffix(vocab: &[[&str; 2]]) -> Result<()> {
    let vocab_map = to_map(vocab);
    validate_expected_suffix_map(&vocab_map)?;
    validate_incremental_decoding(&vocab_map)
}

/// The sorted keys of `vocab`, as `BytesRef`s.
fn sorted_src(vocab: &HashMap<String, String>) -> Vec<BytesRef> {
    let mut keys: Vec<&String> = vocab.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|k| BytesRef::new(k.as_bytes().to_vec()))
        .collect()
}

/// `validateExpectedSuffix(Map<String, String> vocab)`.
fn validate_expected_suffix_map(vocab: &HashMap<String, String>) -> Result<()> {
    let src = sorted_src(vocab);
    let output = compress_prefixes(&src)?;
    let src_strings = src
        .iter()
        .map(|b| b.utf8_to_string())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = Vec::with_capacity(output.len());
    for e in &output {
        result.push(format!(
            "{}{}",
            e.get_suffix_offset(),
            create_suffix_bytes(e).utf8_to_string()?
        ));
    }
    validate_map_list(vocab, &src_strings, &result);
    Ok(())
}

/// `createSuffixBytes(TermBytes termBytes)`: `new BytesRef(term.bytes,
/// suffixOffset, suffixLength)`.
fn create_suffix_bytes(term_bytes: &TermBytes) -> BytesRef {
    BytesRef {
        bytes: term_bytes.get_term().bytes.clone(),
        offset: term_bytes.get_suffix_offset() as usize,
        length: term_bytes.get_suffix_length() as usize,
    }
}

/// `validateExpectedMDP(String[][] vocab)`.
fn validate_expected_mdp(vocab: &[[&str; 2]]) -> Result<()> {
    let vocab_map = to_map(vocab);
    validate_expected_mdp_map(&vocab_map)?;
    validate_incremental_decoding(&vocab_map)
}

/// `validateExpectedMDP(Map<String, String> vocab)`.
fn validate_expected_mdp_map(vocab: &HashMap<String, String>) -> Result<()> {
    let src = sorted_src(vocab);
    let output = compress_prefixes(&src)?;
    let src_strings = src
        .iter()
        .map(|b| b.utf8_to_string())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = Vec::with_capacity(output.len());
    for e in &output {
        let mdp = BytesRef {
            bytes: e.get_term().bytes.clone(),
            offset: 0,
            length: e.get_mdp_length() as usize,
        };
        result.push(mdp.utf8_to_string()?);
    }
    validate_map_list(vocab, &src_strings, &result);
    Ok(())
}

/// `validateIncrementalDecoding(Map<String, String> vocab)`.
fn validate_incremental_decoding(vocab: &HashMap<String, String>) -> Result<()> {
    // `new BytesRef(80)`: an 80-byte array, length 0.
    let mut previous = BytesRef {
        bytes: vec![0u8; 80],
        offset: 0,
        length: 0,
    };
    let src = sorted_src(vocab);
    let output = compress_prefixes(&src)?;
    for i in 0..src.len() {
        copy_bytes(
            &BytesRef::deep_copy_of(&create_suffix_bytes(&output[i])),
            &mut previous,
            output[i].get_suffix_offset() as usize,
        );
        Assert::assert_equals_str(
            Some(&format!("Error in line {i}")),
            Some(&src[i].utf8_to_string()?),
            Some(&previous.utf8_to_string()?),
        );
    }
    Ok(())
}

/// `validateMapList(Map<String, String> expectedMap, List<String> src,
/// List<String> result)`.
fn validate_map_list(expected_map: &HashMap<String, String>, src: &[String], result: &[String]) {
    for i in 0..src.len() {
        Assert::assert_equals_str(
            Some(&format!("Error in line {i}")),
            expected_map.get(&src[i]).map(String::as_str),
            Some(&result[i]),
        );
    }
}

/// `compressPrefixes(List<BytesRef> vocab)` (`private static`).
fn compress_prefixes(vocab: &[BytesRef]) -> Result<Vec<TermBytes>> {
    let mut term_bytes = Vec::with_capacity(vocab.len());
    let mut last: Option<&BytesRef> = None;
    let mut term;
    let mut mdp;
    for current in vocab {
        mdp = TermBytes::compute_mdp_length(last, current)?;
        term = TermBytes::new(mdp, current.clone());
        term_bytes.push(term);
        last = Some(current);
    }
    Ok(term_bytes)
}

/// `copyBytes(BytesRef source, BytesRef target, int targetOffset)`
/// (`private static`).
fn copy_bytes(source: &BytesRef, target: &mut BytesRef, target_offset: usize) {
    java_assert(target.offset == 0, String::new);
    java_assert(source.offset == 0, String::new);
    let new_length = target_offset + source.length;
    if new_length > target.bytes.len() {
        let mut copy = vec![0u8; new_length];
        copy[..target_offset].copy_from_slice(&target.bytes[..target_offset]);
        target.bytes = copy;
    }
    target.length = new_length;
    target.bytes[target_offset..target_offset + source.length]
        .copy_from_slice(&source.bytes[..source.length]);
}

/// `toMap(String[][] src)` (`private static`).
fn to_map(src: &[[&str; 2]]) -> HashMap<String, String> {
    java_assert(!src.is_empty(), || "insert at least one row".to_string());
    src.iter()
        .map(|kv| (kv[0].to_string(), kv[1].to_string()))
        .collect()
}

/// A test method `public void <name>()`.
fn t(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name).body_any(move || body().unwrap_or_else(|e| rethrow(e)))
}

fn term_bytes_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.uniformsplit.TestTermBytes")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testMDPA", test_mdpa))
        .method(t("testIncrementalA", test_incremental_a))
        .method(t("testMDPMIX2", test_mdpmix2))
        .method(t("testMDP", test_mdp))
        .method(t("testIncremental", test_incremental))
        .method(t("testIncrementalSimple", test_incremental_simple))
        .method(t("testMDPSimple", test_mdp_simple))
        .method(t("testMDPMIX", test_mdpmix))
        .build()
}

#[test]
fn test_term_bytes() {
    run(term_bytes_class);
}

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

/// `new BytesRef(String)`.
fn br(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// `BytesRef.compareTo(BytesRef)`: `Arrays.compareUnsigned` over the two
/// byte ranges.
fn compare_unsigned(a: &[u8], b: &[u8]) -> i32 {
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return i32::from(*x) - i32::from(*y);
        }
    }
    a.len() as i32 - b.len() as i32
}

/// `assertEquals(expected, actual)` for a `BytesRef` that may be `null`
/// (`None`).
fn assert_equals_bytes_ref(expected: &BytesRef, actual: Option<&BytesRef>) {
    match actual {
        Some(actual) => Assert::assert_equals(expected.clone(), actual.clone()),
        None => Assert::fail(&format!("expected:<{expected}> but was:<null>")),
    }
}

/// `new IntBlockTermState()`: `MockTermStateFactory.create()`
/// (`lucene/codecs/src/test/.../lucene90/tests/MockTermStateFactory.java`).
fn mock_term_state() -> BlockTermState {
    BlockTermState::default()
}

/// The `java.lang.NullPointerException` a dereference of Java's `null`
/// throws.
fn npe() -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.NullPointerException",
        message: None,
        cause: None,
    }
}

/// The anonymous `PostingsReaderBase` of `createMockPostingReaderBase()`
/// (`TestTermBytesComparator`) and `getMockPostingReaderBase()`
/// (`TestSTBlockReader`): every method does nothing or returns `null`.
#[derive(Debug)]
struct MockPostingsReaderBase;

impl std::fmt::Display for MockPostingsReaderBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MockPostingsReaderBase")
    }
}

impl PostingsReaderBase for MockPostingsReaderBase {
    fn init(&mut self, _terms_in: &mut dyn IndexInput, _state: &SegmentReadState) -> Result<()> {
        Ok(())
    }

    /// Java returns `null`. **Divergence (declared):** the port's
    /// `newTermState` returns a state, which `BlockReader` keeps as its
    /// scratch state; a fresh state stands for `null`, and the overridden
    /// `readTermState` of the tests never reads it.
    fn new_term_state(&self) -> Result<BlockTermState> {
        Ok(BlockTermState::default())
    }

    fn decode_term(
        &self,
        _input: &mut dyn DataInput,
        _field_info: &FieldInfo,
        _state: &mut BlockTermState,
        _absolute: bool,
    ) -> Result<()> {
        Ok(())
    }

    /// Java returns `null`, which the port's non-optional return cannot
    /// carry; the `NullPointerException` a caller would meet is raised here
    /// instead (no test calls it).
    fn postings(
        &self,
        _field_info: &FieldInfo,
        _state: &BlockTermState,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(npe())
    }

    /// As [`postings`](Self::postings).
    fn impacts(
        &self,
        _field_info: &FieldInfo,
        _state: &BlockTermState,
        _flags: i32,
    ) -> Result<Box<dyn ImpactsEnum>> {
        Err(npe())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// `new FieldInfo(fieldName, number, false, false, true,
/// DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS, NONE, NONE, -1, emptyMap(), 0,
/// 0, 0, 0, FLOAT32, EUCLIDEAN, softDeletesField, false)`: the
/// `getMockFieldInfo` of `TestBlockWriter` (`softDeletesField` true) and the
/// `mockFieldInfo` of `TestSTBlockReader` (false).
fn mock_field_info(field_name: &str, number: i32, soft_deletes_field: bool) -> Result<FieldInfo> {
    FieldInfo::new_full(
        field_name,
        number,
        false,
        false,
        true,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
        DocValuesType::NONE,
        DocValuesSkipIndexType::NONE,
        -1,
        Default::default(),
        0,
        0,
        0,
        0,
        VectorEncoding::FLOAT32,
        VectorSimilarityFunction::EUCLIDEAN,
        soft_deletes_field,
        false,
    )
}

/// A test method of a class with an instance: `public void <name>()`.
fn ti<T: std::any::Any + Send>(name: &str, body: fn(&mut T) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut T| body(inst).unwrap_or_else(|e| rethrow(e)))
}

// -----------------------------------------------------------------------------
// TestBlockWriter
// -----------------------------------------------------------------------------

/// The instance of `TestBlockWriter`: its `blockOutput` field.
///
/// **Divergence (declared).** Java's `setUp()` also creates the
/// `blockWriter` field over `blockOutput`; the port's `BlockWriter` borrows
/// its output, which an instance cannot hold beside the output, so each test
/// creates the writer, as `setUp()` does, over the instance's output
/// ([`block_writer`]).
#[derive(Default)]
struct BlockWriterTest {
    block_output: Option<ByteBuffersIndexOutput>,
}

/// `new BlockWriter(blockOutput, 10, 2, null)` (`setUp()`).
fn block_writer(inst: &mut BlockWriterTest) -> BlockWriter<'_> {
    let block_output = inst
        .block_output
        .as_mut()
        .unwrap_or_else(|| throw_without_message("java.lang.NullPointerException"));
    BlockWriter::new(block_output, 10, 2, None)
}

/// `addLine(term, termState, null)` before any `setField`.
///
/// **Divergence (declared).** Java passes a `null` dictionary builder and
/// has no field set, neither of which is read while no block is flushed; the
/// port's `addLine` takes both by reference, so an unused builder and an
/// empty `FieldMetadata` stand for them.
fn add_line_no_field(writer: &mut BlockWriter<'_>, term: BytesRef) -> Result<()> {
    let mut unused_builder = fst_dictionary::Builder::new()?;
    let mut no_field = FieldMetadata::with_mutability(None, 0, true);
    writer.add_line(
        term,
        mock_term_state(),
        &mut unused_builder,
        &mut FieldContext(&mut no_field),
    )
}

fn block_writer_test_add_line(inst: &mut BlockWriterTest) -> Result<()> {
    let mut block_writer = block_writer(inst);
    let term = br("mike");
    add_line_no_field(&mut block_writer, term.clone())?;
    Assert::assert_equals(1usize, block_writer.block_lines().len());
    assert_equals_bytes_ref(&term, block_writer.last_term());
    Ok(())
}

fn block_writer_test_add_multiple_lines_single_block(inst: &mut BlockWriterTest) -> Result<()> {
    let mut block_writer = block_writer(inst);
    let terms = ["ana", "bark", "condor", "dice", "elephant"];
    for t in terms {
        add_line_no_field(&mut block_writer, br(t))?;
    }
    Assert::assert_equals(terms.len(), block_writer.block_lines().len());
    assert_equals_bytes_ref(&br(terms[terms.len() - 1]), block_writer.last_term());
    Ok(())
}

fn block_writer_test_add_multiple_lines_multi_block(inst: &mut BlockWriterTest) -> Result<()> {
    let mut block_writer = block_writer(inst);
    let terms = [
        "ana",
        "bark",
        "condor",
        "dice",
        "elephant",
        "fork",
        "gain",
        "hyper",
        "identifier",
        "judge",
        "ko",
        "large",
    ];
    // in order to build a block a FieldMetadata must be set
    let mut field_metadata = FieldMetadata::new(mock_field_info("content", 0, true)?, 0);

    let mut dictionary_builder = fst_dictionary::Builder::new()?;

    for t in terms {
        block_writer.add_line(
            br(t),
            mock_term_state(),
            &mut dictionary_builder,
            &mut FieldContext(&mut field_metadata),
        )?;
    }
    // at least one block was flushed
    Assert::assert_true(block_writer.block_output().file_pointer() > 0);

    // last term is always the last term to be writen
    assert_equals_bytes_ref(&br(terms[terms.len() - 1]), block_writer.last_term());

    // remains 'large' to be flushed
    Assert::assert_equals(1usize, block_writer.block_lines().len());

    block_writer.finish_last_block(
        &mut dictionary_builder,
        &mut FieldContext(&mut field_metadata),
    )?;

    // we release memory
    Assert::assert_true(block_writer.block_lines().is_empty());
    Assert::assert_true(block_writer.last_term().is_none());
    Assert::assert_equals(0usize, block_writer.block_lines_write_buffer().size());
    Assert::assert_equals(0usize, block_writer.term_states_write_buffer().size());
    Ok(())
}

fn block_writer_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.uniformsplit.TestBlockWriter")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(BlockWriterTest::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut BlockWriterTest| {
                LuceneTestCase::super_set_up();
                inst.block_output = Some(ByteBuffersIndexOutput::new(
                    ByteBuffersDataOutput::new_resettable_instance(),
                    "Test",
                    "Test",
                ));
            },
        ))
        .method(ti("testAddLine", block_writer_test_add_line))
        .method(ti(
            "testAddMultipleLinesSingleBlock",
            block_writer_test_add_multiple_lines_single_block,
        ))
        .method(ti(
            "testAddMultipleLinesMultiBlock",
            block_writer_test_add_multiple_lines_multi_block,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestFSTDictionary
// -----------------------------------------------------------------------------

/// `assertNotSame(Long, Long)` over two autoboxed `long`s: `Long.valueOf`
/// returns its cached instance for `-128..=127`, so two equal values in that
/// range are the same object and any other pair is not.
fn assert_not_same_long(unexpected: i64, actual: i64) {
    if unexpected == actual && (-128..=127).contains(&actual) {
        Assert::fail(&format!("expected not same:<{unexpected}>"));
    }
}

/// `assertNotSame(srcDictionary, fstDictionary)`: object identity.
fn assert_not_same_dictionary(a: &dyn IndexDictionary, b: &dyn IndexDictionary) {
    if std::ptr::addr_eq(
        a as *const dyn IndexDictionary,
        b as *const dyn IndexDictionary,
    ) {
        Assert::fail("expected not same");
    }
}

fn fst_dictionary_test_empty_term_supported() -> Result<()> {
    let index_dictionary = create_fst_dictionary(&[BytesRef::default()], &[588])?;
    Assert::assert_equals(
        588i64,
        index_dictionary
            .browser()?
            .seek_block(&BytesRef::default())?,
    );
    Ok(())
}

fn fst_dictionary_test_repeated_term_not_allowed() -> Result<()> {
    for term in [BytesRef::default(), br("a")] {
        // `try { ... fail(...); } catch (Exception e) { ... }`: the error the
        // call returns or the exception it throws.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            create_fst_dictionary(&[term.clone(), term.clone()], &[0, 1])
        }));
        let e = match outcome {
            Ok(Ok(_)) => Assert::fail("Expected exception not thrown"),
            Ok(Err(e)) => java_throwable(&e),
            Err(payload) => Throwable::from_panic(payload),
        };
        if !e.is_instance("java.lang.Exception") {
            // `catch (Exception e)` does not catch an `Error`.
            e.throw();
        }
        // `assertSame(UnsupportedOperationException.class, e.getClass())`:
        // a class is one object per name.
        Assert::assert_equals_str(
            None,
            Some("java.lang.UnsupportedOperationException"),
            Some(e.class_name()),
        );
    }
    Ok(())
}

fn fst_dictionary_test_repeated_output_allowed() -> Result<()> {
    let terms = [br("a"), br("b")];
    let index_dictionary = create_fst_dictionary(&terms, &[588, 588])?;
    Assert::assert_equals(588i64, index_dictionary.browser()?.seek_block(&br("a"))?);
    Assert::assert_equals(588i64, index_dictionary.browser()?.seek_block(&br("b"))?);
    Ok(())
}

fn fst_dictionary_test_serialization() -> Result<()> {
    let vocab = ["aswoon", "asyl", "asyla", "asyllabic"];

    for should_encode in [false, true] {
        let src_dictionary = create_fst_dictionary_vocab(&vocab)?;
        let fst_dictionary = serialize_and_read_dictionary(&*src_dictionary, should_encode)?;
        assert_not_same_dictionary(&*src_dictionary, &fst_dictionary);
        Assert::assert_equals(
            -1i64,
            fst_dictionary.browser()?.seek_block(&BytesRef::default())?,
        );
        assert_not_same_long(-1, fst_dictionary.browser()?.seek_block(&br("aswoon"))?);
        assert_not_same_long(-1, fst_dictionary.browser()?.seek_block(&br("z"))?);
    }
    Ok(())
}

fn fst_dictionary_test_serialization_empty_term() -> Result<()> {
    for should_encode in [false, true] {
        let src_dictionary = create_fst_dictionary(&[BytesRef::default()], &[0])?;
        let fst_dictionary = serialize_and_read_dictionary(&*src_dictionary, should_encode)?;
        assert_not_same_dictionary(&*src_dictionary, &fst_dictionary);
        Assert::assert_equals(
            0i64,
            fst_dictionary.browser()?.seek_block(&BytesRef::default())?,
        );
    }
    Ok(())
}

fn fst_dictionary_test_common_prefixes() -> Result<()> {
    let vocab = vec![
        "aswoon",
        "asyl",
        "asyla",
        "asyllabic",
        "asylum",
        "asylums",
        "asymmetric",
        "asymmetrical",
        "asymmetrically",
        "asymmetries",
        "asymmetry",
        "asymptomatic",
        "asymptomatically",
        "asymptote",
        "asymptotes",
        "asymptotic",
        "asymptotical",
        "asymptotically",
        "asynapses",
        "asynapsis",
    ];

    let mut block_fps = vec![0i32; vocab.len()];
    for (i, fp) in block_fps.iter_mut().enumerate() {
        *fp = i as i32;
    }
    let block_keys: Vec<BytesRef> = vocab.iter().map(|s| br(s)).collect();
    let index_dictionary = create_fst_dictionary(&block_keys, &block_fps)?;
    let mut browser = index_dictionary.browser()?;
    for i in 0..vocab.len() {
        Assert::assert_equals(i64::from(block_fps[i]), browser.seek_block(&block_keys[i])?);
    }
    Assert::assert_equals(
        i64::from(block_fps[vocab.len() - 1]),
        browser.seek_block(&br("zoo"))?,
    );
    Assert::assert_equals(-1i64, browser.seek_block(&br("A"))?);
    Assert::assert_equals(
        i64::from(block_fps[9]),
        browser.seek_block(&br("asymmetriesz"))?,
    );
    Ok(())
}

/// `createFSTDictionary(List<BytesRef> blockKeys, int[] blockFPs)`.
fn create_fst_dictionary(
    block_keys: &[BytesRef],
    block_fps: &[i32],
) -> Result<Box<dyn IndexDictionary>> {
    let mut builder = fst_dictionary::Builder::new()?;
    for i in 0..block_keys.len() {
        builder.add(&block_keys[i], i64::from(block_fps[i]))?;
    }
    builder.build()
}

/// `createFSTDictionary(List<String> vocab)`.
fn create_fst_dictionary_vocab(vocab: &[&str]) -> Result<Box<dyn IndexDictionary>> {
    let mut builder = fst_dictionary::Builder::new()?;
    for (i, word) in vocab.iter().enumerate() {
        builder.add(&br(word), i as i64)?;
    }
    builder.build()
}

/// `serializeAndReadDictionary(FSTDictionary srcDictionary, boolean
/// shouldEncrypt)`.
///
/// **Divergence (declared).** Java reads the dictionary from
/// `output.toDataInput()`, a `ByteBuffersDataInput`; the port's
/// `FSTDictionary.read` takes an `IndexInput`, so the same data input is
/// wrapped in a `ByteBuffersIndexInput`. The FST is loaded on-heap, as in
/// Java.
fn serialize_and_read_dictionary(
    src_dictionary: &dyn IndexDictionary,
    should_encrypt: bool,
) -> Result<FSTDictionary> {
    let mut output = ByteBuffersDataOutput::new_resettable_instance();
    let encoder = should_encrypt.then(Rot13CypherTestUtil::get_block_encoder);
    src_dictionary.write(&mut output, encoder.as_deref())?;
    // We must load the FST on-heap since we use a ByteBuffersDataInput which is not an instance of
    // IndexInput.
    let decoder = should_encrypt.then(Rot13CypherTestUtil::get_block_decoder);
    let mut input = ByteBuffersIndexInput::new(output.to_data_input()?, "serialized dictionary");
    FSTDictionary::read(&mut input, decoder.as_deref(), true)
}

fn fst_dictionary_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.uniformsplit.TestFSTDictionary")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t(
            "testEmptyTermSupported",
            fst_dictionary_test_empty_term_supported,
        ))
        .method(t(
            "testRepeatedTermNotAllowed",
            fst_dictionary_test_repeated_term_not_allowed,
        ))
        .method(t(
            "testRepeatedOutputAllowed",
            fst_dictionary_test_repeated_output_allowed,
        ))
        .method(t("testSerialization", fst_dictionary_test_serialization))
        .method(t(
            "testSerializationEmptyTerm",
            fst_dictionary_test_serialization_empty_term,
        ))
        .method(t("testCommonPrefixes", fst_dictionary_test_common_prefixes))
        .build()
}

// -----------------------------------------------------------------------------
// TestTermBytesComparator
// -----------------------------------------------------------------------------

/// The `null` `IndexDictionary.BrowserSupplier` `MockBlockReader` passes to
/// `BlockReader`: using it throws Java's `NullPointerException`.
///
/// **Divergence (declared).** The port's `BlockReader` takes the supplier
/// by value; this supplier stands for `null`.
struct NullBrowserSupplier;

impl BrowserSupplier for NullBrowserSupplier {
    fn get(&self) -> Result<Box<dyn Browser>> {
        Err(npe())
    }
}

/// The overrides of `MockBlockReader extends BlockReader`: its `lines`.
struct MockBlockReader {
    lines: Vec<BlockLine>,
}

impl BlockReaderOverrides for MockBlockReader {
    fn compare_to_middle_and_jump(
        &self,
        _reader: &mut BlockReader,
        _searched_term: &[u8],
    ) -> Result<i32> {
        // Do not jump in test.
        Ok(-1)
    }

    fn read_line_in_block(&self, reader: &mut BlockReader) -> Result<Option<ReadLine>> {
        if reader.line_index_in_block() as usize >= self.lines.len() {
            reader.set_line_index_in_block(0);
            return Ok(reader.set_block_line(None));
        }
        let index = reader.line_index_in_block();
        reader.set_line_index_in_block(index + 1);
        Ok(reader.set_block_line(Some(&self.lines[index as usize])))
    }

    fn initialize_header(
        &self,
        reader: &mut BlockReader,
        searched_term: Option<&[u8]>,
        target_block_start_fp: i64,
    ) -> Result<()> {
        // Force blockStartFP to an impossible value so we never trigger the optimization
        // that keeps the current block with our mock block reader.
        reader.set_block_start_fp(i64::MIN);
        reader.base_initialize_header(searched_term, target_block_start_fp)
    }

    fn read_header(&self, reader: &mut BlockReader) -> Result<Option<BlockHeader>> {
        let header = if reader.line_index_in_block() as usize >= self.lines.len() {
            None
        } else {
            Some(BlockHeader::new(self.lines.len() as i32, 0, 0, 0, 0, 0))
        };
        reader.set_block_header(header);
        Ok(header)
    }
}

/// `MockBlockReader.reset()`.
fn mock_block_reader_reset(block_reader: &mut BlockReader) {
    block_reader.set_line_index_in_block(0);
    block_reader.set_block_header(None);
    block_reader.set_block_line(None);
}

/// `new MockBlockReader(lines, directory)`.
fn new_mock_block_reader(lines: Vec<BlockLine>, directory: &dyn Directory) -> Result<BlockReader> {
    let reader = BlockReader::new(
        Arc::new(NullBrowserSupplier),
        Arc::new(directory.open_input("temp.bin", &*DEFAULT_IO_CONTEXT)?),
        Arc::new(MockPostingsReaderBase),
        Arc::new(FieldMetadata::with_mutability(None, 1, true)),
        None,
    )?;
    Ok(reader.with_overrides(Arc::new(MockBlockReader { lines })))
}

fn term_bytes_comparator_test_comparison() -> Result<()> {
    let vocab = [
        term_bytes(1, "abaco"),
        term_bytes(2, "amiga"),
        term_bytes(5, "amigo"),
        term_bytes(2, "arco"),
        term_bytes(1, "bloom"),
        term_bytes(1, "frien"),
        term_bytes(6, "frienchies"),
        term_bytes(6, "friend"),
        term_bytes(7, "friendalan"),
        term_bytes(7, "friende"),
        term_bytes(8, "friendez"),
    ];
    let lines = generate_block_lines(&vocab);
    let directory = ByteBuffersDirectory::new();
    {
        let mut index_output = directory.create_output("temp.bin", &*DEFAULT_IO_CONTEXT)?;
        index_output.write_v_int(5)?;
        index_output.close()?;
    }

    let mut block_reader = new_mock_block_reader(lines, &directory)?;

    assert_always_greater(&mut block_reader, &br("z"))?;

    assert_greater_until(1, &mut block_reader, &br("abacu"))?;

    assert_greater_until(4, &mut block_reader, &br("bar"))?;

    assert_greater_until(2, &mut block_reader, &br("amigas"))?;

    assert_greater_until(10, &mut block_reader, &br("friendez"))?;
    Ok(())
}

/// `assertGreaterUntil(int expectedPosition, MockBlockReader blockReader,
/// BytesRef lookedTerm)` (`private`).
fn assert_greater_until(
    expected_position: i32,
    block_reader: &mut BlockReader,
    looked_term: &BytesRef,
) -> Result<SeekStatus> {
    let seek_status = block_reader.seek_in_block(looked_term.slice())?;
    Assert::assert_equals_msg(
        Some(&format!("looked Term: {}", looked_term.utf8_to_string()?)),
        expected_position,
        block_reader.line_index_in_block() - 1,
    );

    // reset the state
    mock_block_reader_reset(block_reader);
    Ok(seek_status)
}

/// `assertAlwaysGreater(MockBlockReader blockReader, BytesRef lookedTerm)`
/// (`private`).
fn assert_always_greater(block_reader: &mut BlockReader, looked_term: &BytesRef) -> Result<()> {
    let seek_status = assert_greater_until(-1, block_reader, looked_term)?;
    Assert::assert_equals(format!("{:?}", SeekStatus::END), format!("{seek_status:?}"));
    Ok(())
}

/// `generateBlockLines(TermBytes[] words)` (`private`): `new
/// BlockLine(word, null)` for each word.
fn generate_block_lines(words: &[TermBytes]) -> Vec<BlockLine> {
    let mut lines = Vec::with_capacity(words.len());
    for word in words {
        lines.push(BlockLine::for_writing(word.clone(), None));
    }
    lines
}

/// `termBytes(int mdpLength, String term)` (`private static`).
fn term_bytes(mdp_length: i32, term: &str) -> TermBytes {
    TermBytes::new(mdp_length, br(term))
}

fn term_bytes_comparator_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.uniformsplit.TestTermBytesComparator")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testComparison", term_bytes_comparator_test_comparison))
        .build()
}

// -----------------------------------------------------------------------------
// TestSTBlockReader (`sharedterms`)
// -----------------------------------------------------------------------------

const MOCK_BLOCK_OUTPUT_NAME: &str = "TestSTBlockReader.tmp";

/// The instance of `TestSTBlockReader`: its fields `fieldInfos`,
/// `blockLines`, `supplier` and `directory`, which `setUp()` sets.
#[derive(Default)]
struct STBlockReaderTest {
    field_infos: Option<Arc<FieldInfos>>,
    block_lines: Option<Arc<Vec<MockSTBlockLine>>>,
    supplier: Option<Arc<dyn BrowserSupplier>>,
    directory: Option<ByteBuffersDirectory>,
}

/// The anonymous `IndexDictionary.BrowserSupplier` of `setUp()`:
/// `indexDictionary.browser()`.
struct DictionarySupplier(Box<dyn IndexDictionary>);

impl BrowserSupplier for DictionarySupplier {
    fn get(&self) -> Result<Box<dyn Browser>> {
        self.0.browser()
    }
}

/// `TestSTBlockReader.setUp()`.
fn st_block_reader_set_up(inst: &mut STBlockReaderTest) -> Result<()> {
    LuceneTestCase::super_set_up();

    inst.field_infos = Some(Arc::new(mock_field_infos()?));
    let vocab = vec![
        block_line_def(1, "abaco", &["f1", "f3"]),
        block_line_def(2, "amiga", &["f1", "f2", "f4"]),
        block_line_def(5, "amigo", &["f1", "f2", "f3", "f4"]),
        block_line_def(2, "arco", &["f1"]),
        block_line_def(1, "bloom", &["f2"]),
        block_line_def(1, "frien", &["f2"]),
        block_line_def(6, "frienchies", &["f3"]),
    ];

    inst.block_lines = Some(Arc::new(generate_st_block_lines(vocab)));
    let directory = ByteBuffersDirectory::new();
    {
        let mut block_output =
            directory.create_output(MOCK_BLOCK_OUTPUT_NAME, &*DEFAULT_IO_CONTEXT)?;
        block_output.write_v_int(5)?;
        block_output.close()?;
    }
    inst.directory = Some(directory);
    let mut builder = fst_dictionary::Builder::new()?;
    builder.add(&br("a"), 0)?;
    let index_dictionary = builder.build()?;
    inst.supplier = Some(Arc::new(DictionarySupplier(index_dictionary)));
    Ok(())
}

/// `TestSTBlockReader.tearDown()`.
fn st_block_reader_tear_down(inst: &mut STBlockReaderTest) -> Result<()> {
    let result = (|| {
        // `blockLines.clear()`
        inst.block_lines = Some(Arc::new(Vec::new()));
        match inst.directory.as_mut() {
            Some(directory) => directory.close(),
            None => Err(npe()),
        }
    })();
    LuceneTestCase::super_tear_down();
    result
}

/// `new MockSTBlockReader(supplier, blockLines, directory,
/// fieldInfos.fieldInfo(name), fieldInfos)`.
fn mock_st_block_reader(inst: &STBlockReaderTest, field_name: &str) -> Result<STBlockReader> {
    let field_infos = inst.field_infos.clone().ok_or_else(npe)?;
    let lines = inst.block_lines.clone().ok_or_else(npe)?;
    let supplier = inst.supplier.clone().ok_or_else(npe)?;
    let directory = inst.directory.as_ref().ok_or_else(npe)?;
    let field_info = field_infos.field_info(field_name).ok_or_else(npe)?.clone();
    MockSTBlockReader::new(supplier, lines, directory, field_info, field_infos)
}

fn st_block_reader_test_seek_exact_ignore_field_f1(inst: &mut STBlockReaderTest) -> Result<()> {
    // when block reader for field 1 -> f1
    let mut block_reader = mock_st_block_reader(inst, "f1")?; // last term "arco"

    // when seekCeil
    block_reader.seek_ceil(&br("arco2"))?;
    // then
    Assert::assert_true(block_reader.term_bytes().is_none());

    // when seekCeilIgnoreField
    block_reader.seek_ceil_ignore_field(&br("arco2"))?;
    // then
    Assert::assert_equals_str(None, Some("bloom"), Some(&term_string(&block_reader)?));
    Ok(())
}

fn st_block_reader_test_seek_exact_ignore_field_f2(inst: &mut STBlockReaderTest) -> Result<()> {
    let mut block_reader = mock_st_block_reader(inst, "f2")?; // last term "frien"

    // when seekCeil
    block_reader.seek_ceil_ignore_field(&br("arco2"))?;
    // then
    Assert::assert_equals_str(None, Some("bloom"), Some(&term_string(&block_reader)?));
    Ok(())
}

fn st_block_reader_test_seek_exact_ignore_field_f3(inst: &mut STBlockReaderTest) -> Result<()> {
    let mut block_reader = mock_st_block_reader(inst, "f3")?; // last term "frienchies"

    // when seekCeilIgnoreField
    block_reader.seek_ceil_ignore_field(&br("arco2"))?;
    // then
    Assert::assert_equals_str(None, Some("bloom"), Some(&term_string(&block_reader)?));

    // when seekCeil
    block_reader.seek_ceil(&br("arco2"))?;
    // then
    Assert::assert_equals_str(None, Some("frienchies"), Some(&term_string(&block_reader)?));
    Ok(())
}

fn st_block_reader_test_seek_exact_ignore_field_f4(inst: &mut STBlockReaderTest) -> Result<()> {
    let mut block_reader = mock_st_block_reader(inst, "f4")?; // last term "amigo"

    // when seekCeilIgnoreField
    block_reader.seek_ceil_ignore_field(&br("abaco"))?;
    // then
    Assert::assert_equals_str(None, Some("abaco"), Some(&term_string(&block_reader)?));

    // when seekCeil
    block_reader.seek_ceil(&br("abaco"))?;
    // then
    Assert::assert_equals_str(None, Some("amiga"), Some(&term_string(&block_reader)?));
    Ok(())
}

/// `blockReader.term().utf8ToString()`: `NullPointerException` for a `null`
/// term.
fn term_string(block_reader: &STBlockReader) -> Result<String> {
    Ok(block_reader
        .term_bytes()
        .ok_or_else(npe)?
        .utf8_to_string()?)
}

/// `mockFieldInfos()` (`private static`).
fn mock_field_infos() -> Result<FieldInfos> {
    FieldInfos::new(vec![
        mock_field_info("f1", 0, false)?,
        mock_field_info("f2", 1, false)?,
        mock_field_info("f3", 2, false)?,
        mock_field_info("f4", 3, false)?,
    ])
}

/// `record BlockLineDefinition(TermBytes termBytes, List<String> fields)`.
struct BlockLineDefinition {
    term_bytes: TermBytes,
    fields: Vec<String>,
}

/// `blockLineDef(int mdpLength, String term, String... fields)`.
fn block_line_def(mdp_length: i32, term: &str, fields: &[&str]) -> BlockLineDefinition {
    BlockLineDefinition {
        term_bytes: TermBytes::new(mdp_length, br(term)),
        fields: fields.iter().map(|f| f.to_string()).collect(),
    }
}

/// `generateBlockLines(Iterable<BlockLineDefinition>)` (`private static`).
fn generate_st_block_lines(
    block_line_definitions: Vec<BlockLineDefinition>,
) -> Vec<MockSTBlockLine> {
    let mut lines = Vec::new();
    for block_line_definition in block_line_definitions {
        lines.push(MockSTBlockLine::new(
            block_line_definition.term_bytes,
            &block_line_definition.fields,
        ));
    }
    lines
}

/// `MockSTBlockLine extends STBlockLine`: the line and its `termStates`
/// map.
struct MockSTBlockLine {
    line: STBlockLine,
    term_states: HashMap<String, BlockTermState>,
}

impl MockSTBlockLine {
    /// `MockSTBlockLine(TermBytes termBytes, List<String> fields)`.
    ///
    /// **Divergence (declared).** Java's single term state is `new
    /// FieldMetadataTermState(null, null)`; the port's
    /// `FieldMetadataTermState` holds a field index and a state, neither of
    /// which can be `null`, so field 0 and a fresh state stand for them (the
    /// test never reads them).
    fn new(term_bytes: TermBytes, fields: &[String]) -> Self {
        let line = STBlockLine::new(
            term_bytes,
            &[FieldMetadataTermState::new(0, BlockTermState::default())],
        );
        let mut term_states = HashMap::new();
        for field in fields {
            term_states.insert(field.clone(), mock_term_state());
        }
        Self { line, term_states }
    }

    /// `getFields()`.
    fn get_fields(&self) -> impl Iterator<Item = &String> {
        self.term_states.keys()
    }
}

/// The overrides of `MockSTBlockReader extends STBlockReader`: its `lines`.
struct MockSTBlockReader {
    lines: Arc<Vec<MockSTBlockLine>>,
}

impl MockSTBlockReader {
    /// `MockSTBlockReader(supplier, lines, directory, fieldInfo, fieldInfos)`.
    fn new(
        supplier: Arc<dyn BrowserSupplier>,
        lines: Arc<Vec<MockSTBlockLine>>,
        directory: &dyn Directory,
        field_info: FieldInfo,
        field_infos: Arc<FieldInfos>,
    ) -> Result<STBlockReader> {
        let last_term = Self::get_last_term_for_field(&lines, field_info.get_name());
        let reader = STBlockReader::new(
            supplier,
            Arc::new(directory.open_input(MOCK_BLOCK_OUTPUT_NAME, &*DEFAULT_IO_CONTEXT)?),
            Arc::new(MockPostingsReaderBase),
            Arc::new(Self::mock_field_metadata(field_info, last_term)),
            None,
            field_infos,
        )?;
        Ok(reader.with_overrides(Arc::new(MockSTBlockReader { lines })))
    }

    /// `mockFieldMetadata(FieldInfo fieldInfo, BytesRef lastTerm)`.
    ///
    /// `setLastTerm(null)` (a field no line holds) leaves the last term
    /// unset, which is the `null` it already is.
    fn mock_field_metadata(field_info: FieldInfo, last_term: Option<BytesRef>) -> FieldMetadata {
        let mut field_metadata = FieldMetadata::new(field_info, 1);
        if let Some(last_term) = last_term {
            field_metadata.set_last_term(last_term);
        }
        field_metadata.set_last_block_start_fp(1);
        field_metadata
    }

    /// `getLastTermForField(List<MockSTBlockLine> lines, String fieldName)`.
    fn get_last_term_for_field(lines: &[MockSTBlockLine], field_name: &str) -> Option<BytesRef> {
        let mut last_term = None;
        for line in lines {
            if line.get_fields().any(|f| f == field_name) {
                last_term = Some(line.line.block_line().get_term_bytes().get_term().clone());
            }
        }
        last_term
    }
}

impl BlockReaderOverrides for MockSTBlockReader {
    /// `termState = lines.get(lineIndexInBlock - 1).termStates.get(
    /// fieldMetadata.getFieldInfo().name)`: the state is copied into the
    /// reader's term state, and `false` stands for `null`.
    fn read_term_state(&self, reader: &mut BlockReader) -> Result<bool> {
        let index = reader.line_index_in_block() - 1;
        let line = usize::try_from(index)
            .ok()
            .and_then(|i| self.lines.get(i))
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.IndexOutOfBoundsException",
                    format!(
                        "Index {index} out of bounds for length {}",
                        self.lines.len()
                    ),
                )
            })?;
        let name = reader
            .field_metadata()
            .get_field_info()
            .ok_or_else(npe)?
            .get_name()
            .to_string();
        match line.term_states.get(&name) {
            Some(state) => {
                *reader.scratch_term_state_mut() = state.clone();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn compare_to_middle_and_jump(
        &self,
        reader: &mut BlockReader,
        searched_term: &[u8],
    ) -> Result<i32> {
        reader.set_block_line(Some(self.lines[self.lines.len() >> 1].line.block_line()));
        let middle = reader
            .block_header()
            .ok_or_else(npe)?
            .get_middle_line_index();
        reader.set_line_index_in_block(middle);
        let term = reader.term_bytes().ok_or_else(npe)?;
        let compare = compare_unsigned(searched_term, term.slice());
        if compare < 0 {
            reader.set_line_index_in_block(0);
        }
        Ok(compare)
    }

    fn read_line_in_block(&self, reader: &mut BlockReader) -> Result<Option<ReadLine>> {
        if reader.line_index_in_block() as usize >= self.lines.len() {
            return Ok(reader.set_block_line(None));
        }
        let index = reader.line_index_in_block();
        reader.set_line_index_in_block(index + 1);
        Ok(reader.set_block_line(Some(self.lines[index as usize].line.block_line())))
    }

    fn initialize_header(
        &self,
        reader: &mut BlockReader,
        searched_term: Option<&[u8]>,
        start_block_line_pos: i64,
    ) -> Result<()> {
        // Force blockStartFP to an impossible value so we never trigger the optimization
        // that keeps the current block with our mock block reader.
        reader.set_block_start_fp(-1);
        reader.base_initialize_header(searched_term, start_block_line_pos)
    }

    fn read_header(&self, reader: &mut BlockReader) -> Result<Option<BlockHeader>> {
        let header = if reader.line_index_in_block() as usize >= self.lines.len() {
            None
        } else {
            Some(mock_block_header(self.lines.len() as i32))
        };
        reader.set_block_header(header);
        Ok(header)
    }
}

/// `new MockBlockHeader(linesCount)`: `MockBlockHeader extends BlockHeader`,
/// whose constructor is `super(linesCount, 0, 0, 0, 1, 0)`.
fn mock_block_header(lines_count: i32) -> BlockHeader {
    BlockHeader::new(lines_count, 0, 0, 0, 1, 0)
}

fn st_block_reader_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.uniformsplit.sharedterms.TestSTBlockReader")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(STBlockReaderTest::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut STBlockReaderTest| {
                st_block_reader_set_up(inst).unwrap_or_else(|e| rethrow(e))
            },
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut STBlockReaderTest| {
                    st_block_reader_tear_down(inst).unwrap_or_else(|e| rethrow(e))
                }),
        )
        .method(ti(
            "testSeekExactIgnoreFieldF1",
            st_block_reader_test_seek_exact_ignore_field_f1,
        ))
        .method(ti(
            "testSeekExactIgnoreFieldF2",
            st_block_reader_test_seek_exact_ignore_field_f2,
        ))
        .method(ti(
            "testSeekExactIgnoreFieldF3",
            st_block_reader_test_seek_exact_ignore_field_f3,
        ))
        .method(ti(
            "testSeekExactIgnoreFieldF4",
            st_block_reader_test_seek_exact_ignore_field_f4,
        ))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(class: fn() -> Arc<Class>) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(class);
}

#[test]
fn test_block_writer() {
    run(block_writer_class);
}

#[test]
fn test_fst_dictionary() {
    run(fst_dictionary_class);
}

#[test]
fn test_term_bytes_comparator() {
    run(term_bytes_comparator_class);
}

#[test]
fn test_st_block_reader() {
    run(st_block_reader_class);
}
