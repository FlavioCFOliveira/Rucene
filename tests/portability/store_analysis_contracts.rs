//! Portability tests for the `store` and `analysis` methods ported by rmp task
//! #173.
//!
//! Every expected value here is **measured from `lucene-core-10.5.0.jar`** by
//! `org.apache.lucene.store.RuceneStoreAnalysisFixture` in the same run. None
//! is derived from this crate's own arithmetic: asserting an implementation
//! against itself is exactly what lets a dropped term, a wrong trim rule or an
//! unflushed buffer go unnoticed.
//!
//! What is pinned:
//!
//! 1. `ByteBuffersDataInput.ramBytesUsed()` and the private `blockSize()`
//!    across eighteen block layouts, four of which end in a block whose
//!    capacity exceeds its remaining bytes — without those the estimate's
//!    capacity term cannot be told from a length term. The estimate charges
//!    each block's *capacity* plus one object reference, and a single-block
//!    input reports a
//!    block size of `1` — Java because `blockBits` is 32 and `1 << 32` on an
//!    `int` wraps to `1 << 0`, this port because it stores `0` as the same
//!    sentinel. `toString()` is asserted too, since it is the only caller of
//!    `blockSize()` in Lucene.
//! 2. `ByteBuffersDataOutput.ramBytesUsed()` across six write sizes, as a
//!    cross-check of the curve `compressing_accounting.rs` already measures.
//! 3. `WordlistLoader.getLines(InputStream, Charset)` over a corpus built to
//!    separate its four rules and their order — BOM strip guarded by "nothing
//!    kept yet" rather than "first line", comment test *before* the trim,
//!    `String.trim()` semantics, blank drop last.
//! 4. `String.trim()` itself, which trims by code point `<= U+0020` and so is
//!    neither Rust's Unicode [`str::trim`] nor a subset of it: `U+0001` is
//!    trimmed by Java and not by Rust, `U+00A0` by Rust and not by Java.
//! 5. `WordlistLoader.getSnowballWordSet`, whose `split("\\s+")` is ASCII-only
//!    for the same reason.
//! 6. `Tokenizer.correctOffset(int)`, with no reader, with a plain reader, with
//!    one `CharFilter` and with two nested ones.
//! 7. **File bytes.** Rucene writes a file with `OutputStreamDataOutput` and
//!    closes it; Lucene reads it back with `InputStreamDataInput` and reports
//!    what it decoded. Lucene writes the same sequence and Rucene reads it
//!    back. The two files are then compared byte for byte. The test also holds
//!    a positive control for `close()`: the file is still empty while the
//!    buffered writer is alive, and complete once `close()` has run.

use std::fs;
use std::io::{BufWriter, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rucene::analysis::{
    get_buffered_reader, get_lines, get_snowball_word_set, get_word_set, CharFilterFn, CharReader,
    StringCharReader, Tokenizer, TokenizerLogic,
};
use rucene::store::{
    ByteBuffersDataInput, ByteBuffersDataOutput, DataInput, DataOutput, InputStreamDataInput,
    OutputStreamDataOutput,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::{Accountable, RamUsageEstimator};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-codec-harness")
}

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "/usr/bin/mvn", "/usr/local/bin/mvn"] {
        if Command::new(candidate)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    Err("Maven is not available on PATH".to_string())
}

/// Runs the Java fixture against `dir` and returns its `key=value` lines.
fn run_fixture(dir: &Path) -> Vec<(String, String)> {
    let mvn = which_mvn().unwrap_or_else(|e| {
        panic!("the store/analysis portability test needs Maven and a JDK: {e}")
    });
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.store.RuceneStoreAnalysisFixture")
        .arg(format!("-Dexec.args={}", dir.display()))
        .current_dir(harness_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn Maven: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success() && stdout.lines().any(|l| l.trim() == "read_ok=true"),
        "the Java fixture failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Decodes the standard base64 the fixture wraps every text value in.
///
/// Written out rather than pulled from a crate so the test has no dependency
/// the library does not already carry.
fn unb64(encoded: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits: u32 = 0;
    let mut have = 0;
    let mut out: Vec<u8> = Vec::new();
    for byte in encoded.bytes() {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET
            .iter()
            .position(|c| *c == byte)
            .unwrap_or_else(|| panic!("not base64: {encoded}")) as u32;
        bits = (bits << 6) | value;
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    String::from_utf8(out).expect("the fixture encodes UTF-8")
}

struct Measured(Vec<(String, String)>);

impl Measured {
    fn one(&self, key: &str) -> String {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("the fixture printed no {key}"))
            .1
            .clone()
    }

    fn all(&self, key: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Every `<key>=<idx>:<b64>` row, decoded and ordered by `<idx>`.
    fn indexed_text(&self, key: &str) -> Vec<String> {
        let mut rows: Vec<(usize, String)> = self
            .all(key)
            .iter()
            .map(|v| {
                let (idx, text) = v.split_once(':').expect("<idx>:<b64>");
                (idx.parse::<usize>().expect("index"), unb64(text))
            })
            .collect();
        rows.sort_by_key(|(i, _)| *i);
        rows.into_iter().map(|(_, t)| t).collect()
    }
}

/// Builds the blocks of one measured layout, e.g. `"1024@1024.1024@300"`.
///
/// Each block is `<capacity>@<remaining>`. `Vec::capacity` stands for
/// `ByteBuffer.capacity()` and `Vec::len` for `remaining()`, and the two differ
/// on the short last block of several layouts — which is what makes the test
/// able to tell `ramBytesUsed()`'s capacity term from the length it would be
/// mistaken for. The capacity is asserted to be exactly what Java allocated,
/// because comparing against a `Vec` that over-allocated would compare two
/// different objects and call it parity.
fn blocks_for(layout: &str) -> Vec<Vec<u8>> {
    layout
        .split('.')
        .map(|block| {
            let (cap, remaining) = block.split_once('@').expect("<capacity>@<remaining>");
            let cap: usize = cap.parse().expect("block capacity");
            let remaining: usize = remaining.parse().expect("block remaining");
            let mut block = Vec::with_capacity(cap);
            block.extend((0..remaining).map(|i| i as u8));
            assert_eq!(
                block.capacity(),
                cap,
                "the test needs a Vec of capacity {cap} to mirror ByteBuffer.allocate({cap})"
            );
            assert_eq!(block.len(), remaining, "and of length {remaining}");
            block
        })
        .collect()
}

/// The number of file descriptors this process holds.
///
/// Linux-only, and used for one purpose: to observe that
/// `InputStreamDataInput::close` really released the file rather than merely
/// returning `Ok`. `read_dir` opens a descriptor of its own, so the two
/// readings are comparable as long as both are taken the same way.
fn open_fds() -> usize {
    fs::read_dir("/proc/self/fd")
        .expect("this test observes descriptor release through /proc, so it needs Linux")
        .count()
}

/// A tokenizer body that emits nothing; it exists to expose `correct_offset`,
/// exactly as the fixture's `ProbeTokenizer` does on the Java side.
#[derive(Debug)]
struct ProbeLogic;

impl TokenizerLogic for ProbeLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> rucene::error::Result<bool> {
        Ok(false)
    }
}

fn doubling_filter(inner: Box<dyn CharReader>) -> Box<dyn CharReader> {
    Box::new(CharFilterFn::new(inner, |off: i32| off * 2))
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn store_and_analysis_contracts_match_lucene() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dir = temp.path();

    // -----------------------------------------------------------------
    // 7a. Write the file *before* the fixture runs, so Lucene reads it.
    //     The length checks around close() are the positive control: if
    //     close() did not flush, `after` would still be 0 and the fixture
    //     would hit EOF.
    // -----------------------------------------------------------------
    let rust_file = dir.join("rust_out.bin");
    {
        let file = fs::File::create(&rust_file).expect("create");
        let mut out = OutputStreamDataOutput::new(BufWriter::new(file));
        out.write_byte(0x42).expect("byte");
        out.write_short(0x1234).expect("short");
        out.write_int(0xDEADBEEF_u32 as i32).expect("int");
        out.write_long(0x1234567890ABCDEF_i64).expect("long");
        out.write_v_int(16384).expect("vint");
        out.write_v_long(1_000_000_000_000).expect("vlong");
        out.write_string("stream test").expect("string");

        let before = fs::metadata(&rust_file).expect("metadata").len();
        assert_eq!(
            before, 0,
            "the bytes must still be inside the BufWriter before close()"
        );
        out.close().expect("close flushes");
        let after = fs::metadata(&rust_file).expect("metadata").len();
        assert!(
            after > 0,
            "close() must push the buffered bytes to the file; got {after}"
        );
    }

    let measured = Measured(run_fixture(dir));

    // -----------------------------------------------------------------
    // 0. The one constant both estimates are built from.
    // -----------------------------------------------------------------
    assert_eq!(
        RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        measured
            .one("num_bytes_object_ref")
            .parse::<i64>()
            .expect("object ref"),
    );

    // -----------------------------------------------------------------
    // 1. ByteBuffersDataInput: ramBytesUsed, blockSize, toString.
    // -----------------------------------------------------------------
    let mut layouts_checked = 0;
    for row in measured.all("bbdi_ram") {
        let (layout, expected) = row.split_once(':').expect("<layout>:<bytes>");
        let input = ByteBuffersDataInput::new(blocks_for(layout)).expect("input");
        assert_eq!(
            input.ram_bytes_used(),
            expected.parse::<i64>().expect("ram bytes"),
            "ByteBuffersDataInput.ramBytesUsed() for block layout {layout}"
        );
        layouts_checked += 1;
    }
    assert!(
        layouts_checked >= 18,
        "the fixture must measure every layout; saw {layouts_checked}"
    );
    assert!(
        measured
            .all("bbdi_ram")
            .iter()
            .any(|row| row
                .split_once(':')
                .is_some_and(|(layout, _)| layout.split('.').any(|b| {
                    let (cap, rem) = b.split_once('@').expect("<cap>@<rem>");
                    cap != rem
                }))),
        "at least one layout must have capacity != remaining, or the estimate's \
         capacity term is indistinguishable from a length term"
    );

    for row in measured.all("bbdi_blocksize") {
        let (layout, expected) = row.split_once(':').expect("<layout>:<size>");
        let input = ByteBuffersDataInput::new(blocks_for(layout)).expect("input");
        assert_eq!(
            input.block_size(),
            expected.parse::<usize>().expect("block size"),
            "ByteBuffersDataInput.blockSize() for block layout {layout}"
        );
    }

    for row in measured.all("bbdi_tostring") {
        let (layout, expected) = row.split_once(':').expect("<layout>:<b64>");
        let input = ByteBuffersDataInput::new(blocks_for(layout)).expect("input");
        assert_eq!(
            input.to_string(),
            unb64(expected),
            "ByteBuffersDataInput.toString() for block layout {layout}"
        );
    }

    // -----------------------------------------------------------------
    // 2. ByteBuffersDataOutput: the same estimate on the writing side.
    // -----------------------------------------------------------------
    for row in measured.all("bbdo_ram") {
        let (written, expected) = row.split_once(':').expect("<n>:<bytes>");
        let written: usize = written.parse().expect("write count");
        let mut out = ByteBuffersDataOutput::new_resettable_instance();
        for i in 0..written {
            out.write_byte(i as u8).expect("write");
        }
        assert_eq!(
            out.ram_bytes_used(),
            expected.parse::<i64>().expect("ram bytes"),
            "ByteBuffersDataOutput.ramBytesUsed() after {written} bytes"
        );
    }

    // -----------------------------------------------------------------
    // 3. WordlistLoader.getLines.
    // -----------------------------------------------------------------
    let corpus = unb64(&measured.one("corpus"));
    let expected_lines = measured.indexed_text("getlines");
    assert_eq!(
        expected_lines.len(),
        measured
            .one("getlines_count")
            .parse::<usize>()
            .expect("count"),
    );
    let actual_lines =
        get_lines(corpus.as_bytes(), rucene::util::IOUtils::UTF_8).expect("getLines");
    assert_eq!(
        actual_lines, expected_lines,
        "WordlistLoader.getLines over the rule corpus"
    );

    // The BOM strip is guarded by "nothing kept yet", not by "first line":
    // here the first line is dropped as a comment, so the mark is taken off
    // the second one instead.
    let bom_late = unb64(&measured.one("bomlate"));
    let expected_bom = measured.indexed_text("bomlate_line");
    let actual_bom =
        get_lines(bom_late.as_bytes(), rucene::util::IOUtils::UTF_8).expect("getLines");
    assert_eq!(
        actual_bom, expected_bom,
        "the BOM must be stripped from the first line that survives, not the first line read"
    );

    // -----------------------------------------------------------------
    // 4. String.trim(), through the loader that applies it.
    // -----------------------------------------------------------------
    for row in measured.all("trim") {
        let (probe, expected) = row.split_once(':').expect("<b64>:<b64>");
        let (probe, expected) = (unb64(probe), unb64(expected));
        let set = get_word_set([probe.as_str()]);
        if expected.is_empty() {
            assert!(
                set.is_empty(),
                "{probe:?} trims to nothing in Java, so no word may be added"
            );
        } else {
            let words: Vec<String> = set.iter().map(|w| w.iter().collect()).collect();
            assert_eq!(
                words,
                vec![expected.clone()],
                "String.trim() of {probe:?} is {expected:?}"
            );
        }
    }

    // -----------------------------------------------------------------
    // 5. Snowball: Java splits on the ASCII \s+ class, so a no-break space
    //    stays inside a word and a vertical tab does not.
    // -----------------------------------------------------------------
    let snowball = unb64(&measured.one("snowball"));
    let mut expected_words = measured.indexed_text("snowball_word");
    expected_words.sort();
    let mut reader = StringCharReader::new(snowball);
    let lines = get_buffered_reader(&mut reader).expect("lines");
    let set = get_snowball_word_set(lines);
    let mut actual_words: Vec<String> = set.iter().map(|w| w.iter().collect()).collect();
    actual_words.sort();
    assert_eq!(
        actual_words, expected_words,
        "WordlistLoader.getSnowballWordSet over the whitespace corpus"
    );

    // -----------------------------------------------------------------
    // 6. Tokenizer.correctOffset.
    // -----------------------------------------------------------------
    let unset = Tokenizer::new(ProbeLogic);
    assert_eq!(
        unset.correct_offset(7),
        measured
            .one("correctoffset_unset")
            .parse::<i32>()
            .expect("int"),
        "with no reader the offset comes back unchanged"
    );

    let mut plain = Tokenizer::new(ProbeLogic);
    plain
        .set_reader(Box::new(StringCharReader::new("hello")))
        .expect("set_reader");
    rucene::analysis::TokenStream::reset(&mut plain).expect("reset");
    assert_eq!(
        plain.correct_offset(7),
        measured
            .one("correctoffset_plain")
            .parse::<i32>()
            .expect("int"),
        "a reader that is not a CharFilter leaves the offset alone"
    );

    let mut filtered = Tokenizer::new(ProbeLogic);
    filtered
        .set_reader(doubling_filter(Box::new(StringCharReader::new("hello"))))
        .expect("set_reader");
    rucene::analysis::TokenStream::reset(&mut filtered).expect("reset");
    assert_eq!(
        filtered.correct_offset(7),
        measured
            .one("correctoffset_filter")
            .parse::<i32>()
            .expect("int"),
        "one CharFilter corrects the offset once"
    );

    let mut nested = Tokenizer::new(ProbeLogic);
    nested
        .set_reader(doubling_filter(doubling_filter(Box::new(
            StringCharReader::new("hello"),
        ))))
        .expect("set_reader");
    rucene::analysis::TokenStream::reset(&mut nested).expect("reset");
    assert_eq!(
        nested.correct_offset(7),
        measured
            .one("correctoffset_nested")
            .parse::<i32>()
            .expect("int"),
        "nested CharFilters chain, innermost last"
    );

    // -----------------------------------------------------------------
    // 7b. The file Rucene wrote, as Lucene decoded it.
    // -----------------------------------------------------------------
    assert_eq!(
        fs::metadata(&rust_file).expect("metadata").len(),
        measured
            .one("rust_file_len")
            .parse::<u64>()
            .expect("file len"),
        "Lucene and the filesystem must agree on the length of the file Rucene closed"
    );
    assert_eq!(
        unb64(&measured.one("rust_file_values")),
        "66,4660,-559038737,1311768467294899695,16384,1000000000000,stream test",
        "InputStreamDataInput on the Java side decodes what Rucene wrote"
    );

    // -----------------------------------------------------------------
    // 7c. The file Lucene wrote, read back by Rucene, then closed.
    // -----------------------------------------------------------------
    let java_file = dir.join("java_out.bin");
    let fds_before = open_fds();
    let mut input = InputStreamDataInput::new(fs::File::open(&java_file).expect("open"));
    assert_eq!(
        open_fds(),
        fds_before + 1,
        "the open file must be visible as one more descriptor"
    );
    assert_eq!(input.read_byte().expect("byte"), 0x42);
    assert_eq!(input.read_short().expect("short"), 0x1234);
    assert_eq!(input.read_int().expect("int"), 0xDEADBEEF_u32 as i32);
    assert_eq!(input.read_long().expect("long"), 0x1234567890ABCDEF_i64);
    assert_eq!(input.read_v_int().expect("vint"), 16384);
    assert_eq!(input.read_v_long().expect("vlong"), 1_000_000_000_000);
    assert_eq!(input.read_string().expect("string"), "stream test");
    input.close().expect("close");
    assert_eq!(
        open_fds(),
        fds_before,
        "close() must release the underlying file, not just return Ok"
    );

    // -----------------------------------------------------------------
    // 7d. The strongest form: the two files are the same bytes.
    // -----------------------------------------------------------------
    let mut rust_bytes = Vec::new();
    fs::File::open(&rust_file)
        .expect("open")
        .read_to_end(&mut rust_bytes)
        .expect("read");
    let mut java_bytes = Vec::new();
    fs::File::open(&java_file)
        .expect("open")
        .read_to_end(&mut java_bytes)
        .expect("read");
    assert_eq!(
        rust_bytes, java_bytes,
        "OutputStreamDataOutput must lay the same sequence down as the same bytes"
    );
    assert_eq!(
        java_bytes.len() as u64,
        measured
            .one("java_file_len")
            .parse::<u64>()
            .expect("file len"),
    );
}
