//! Defensive fuzz-style tests for index file-name parsing and `IndexFileDeleter`.
//!
//! Unlike the codec fuzz suites, the untrusted input here is not a stream of
//! bytes — it is a set of **file names**. `IndexFileDeleter` learns the state of
//! an index by listing the directory and parsing what it finds
//! (`IndexFileDeleter.java:143-176` and `inflateGens`,
//! `IndexFileDeleter.java:257-380`), and it sizes the index's future generation
//! and segment counter from the names it parses. A directory is not a trusted
//! input: an operator can put anything in it, a crashed writer can leave debris
//! behind, and a restored backup can mix names from two indexes.
//!
//! Java's behaviour on a name it cannot make sense of is either to ignore it
//! (the `NumberFormatException` swallowed around `generationFromSegmentsFileName`
//! and `parseGeneration`) or to let an unchecked exception escape (the
//! `Long.parseLong` at `IndexFileDeleter.java:303-304`, which is deliberately
//! *not* inside a `try`). Neither aborts the JVM. A Rust port must match that:
//! `Ok`, or `Err`, but never a panic.
//!
//! # Why this suite exists
//!
//! It is not speculative. Writing the port surfaced a real panic: both
//! `IndexFileNames.index_of_segment_name` and `inflate_gens` reproduced Java's
//! `substring(1)` as the Rust slice `name[1..]`. Java's `substring` counts
//! *characters*; Rust's slice counts *bytes*, so any name beginning with a
//! multi-byte UTF-8 character — `é.si`, say — made the slice land inside a
//! character and abort the process. Java throws at worst. The names that reach
//! this code are not all filtered: `IndexFileDeleter` folds
//! `Directory::get_pending_deletions()` into the set it parses
//! (`IndexFileDeleter.java:211-217`), and that set carries whatever name a
//! failed delete left behind.
//!
//! Both sites now step over one character rather than one byte, and the sweeps
//! below pin that down.
//!
//! # What is swept
//!
//! * every file-name helper, over a corpus built to hit the boundaries the
//!   parsers actually branch on: the leading `_`, the `_` separator, the `.`
//!   extension mark, the radix-36 digit set, and the char/byte distinction;
//! * the same corpus injected at each position of a multi-name set handed to
//!   `inflate_gens`;
//! * a whole directory of hostile names handed to `IndexFileDeleter::new`.
//!
//! Names that do not begin with `_` trip a `debug_assert` inside `inflate_gens`.
//! That is deliberate and faithful: Java asserts the same thing at the same
//! place (`assert segmentName.startsWith("_")`, `IndexFileDeleter.java:296`) and
//! Lucene's own tests run with assertions enabled. The sweeps that reach
//! `inflate_gens` therefore use `_`-leading names, which is also the interesting
//! case — a name that passes Java's assertion and is still not ASCII.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::index_file_names::{
    file_name_from_generation, get_extension, is_codec_file, matches_extension, parse_generation,
    parse_segment_name, segment_file_name, strip_extension, strip_segment_name,
};
use rucene::index::{
    inflate_gens, IndexFileDeleter, KeepOnlyLastCommitDeletionPolicy, SegmentInfos,
};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::NoOutputInfoStream;

// -----------------------------------------------------------------------------
// Corpus
// -----------------------------------------------------------------------------

/// Fragments chosen to sit on the boundaries the file-name parsers branch on.
const FRAGMENTS: &[&str] = &[
    "",
    "_",
    "__",
    "___",
    ".",
    "..",
    "_.",
    "._",
    "0",
    "z",
    "Z",
    "!",
    "-",
    " ",
    "\t",
    "\n",
    "\0",
    // Multi-byte UTF-8 of every width: 2, 3 and 4 bytes. These are what broke
    // the byte-indexed slice.
    "é",
    "€",
    "\u{10348}",
    // A combining mark, whose first byte is a continuation byte.
    "\u{0301}",
    // Radix-36 values at and beyond the i64 boundary.
    "1y2p0ij32e8e7",  // i64::MAX in radix 36
    "1y2p0ij32e8e8",  // i64::MAX + 1
    "zzzzzzzzzzzzzz", // far beyond
    "00000000000000000000",
    // Names the real codec produces, so the corpus is not all garbage.
    "segments",
    "segments_1",
    "pending_segments_1",
    "write.lock",
    "_0.si",
    "_0_Lucene104_0.doc",
    "_0_1.liv",
    "_z_4.dvd",
    "_0.tmp",
    "_0.TMP",
];

/// Builds the full corpus: every fragment, every pair, and every fragment with a
/// codec-like skeleton wrapped round it.
fn corpus() -> Vec<String> {
    let mut names: Vec<String> = FRAGMENTS.iter().map(|s| s.to_string()).collect();

    for a in FRAGMENTS {
        for b in FRAGMENTS {
            names.push(format!("{a}{b}"));
            names.push(format!("_{a}{b}"));
            names.push(format!("_{a}_{b}.si"));
            names.push(format!("_0_{a}.{b}"));
            names.push(format!("segments_{a}{b}"));
            names.push(format!("pending_segments_{a}{b}"));
        }
    }

    // A pathologically long name, to catch anything that indexes by a constant.
    names.push(format!("_{}.si", "z".repeat(4096)));
    names.push("_".repeat(4096));

    names
}

/// The subset of the corpus that begins with `_`, i.e. the names that satisfy
/// the assertion `inflate_gens` inherits from Java.
fn underscore_corpus() -> Vec<String> {
    corpus()
        .into_iter()
        .filter(|n| n.starts_with('_'))
        .collect()
}

// -----------------------------------------------------------------------------
// File-name helpers
// -----------------------------------------------------------------------------

/// Every file-name helper must survive every name in the corpus.
///
/// The helpers have no notion of a trusted caller: `IndexFileDeleter` hands them
/// raw directory entries. Returning a wrong answer for garbage is acceptable —
/// Java does too — but aborting is not.
#[test]
fn file_name_helpers_survive_every_hostile_name() {
    for name in corpus() {
        // Each of these must return, not abort. The results are deliberately
        // unconstrained: garbage in, garbage out is Java's contract too.
        let segment = parse_segment_name(&name);
        let stripped = strip_segment_name(&name);
        let no_ext = strip_extension(&name);
        let ext = get_extension(&name);
        let _ = is_codec_file(&name);
        let _ = matches_extension(&name, "si");
        let _ = parse_generation(&name);

        // Whatever they return must be a real slice of the input, so that a
        // caller re-slicing it cannot land mid-character.
        assert!(
            name.contains(segment) && name.contains(stripped) && name.contains(no_ext),
            "helpers must return substrings of the input, got {segment:?} / {stripped:?} / \
             {no_ext:?} for {name:?}"
        );
        if let Some(ext) = ext {
            assert!(name.contains(ext));
        }
    }
}

/// The results of the helpers must be safely re-sliceable by one character,
/// which is exactly what `inflate_gens` does to recover a segment ordinal.
#[test]
fn a_parsed_segment_name_can_be_advanced_by_one_character() {
    for name in corpus() {
        let segment = parse_segment_name(&name);
        let mut chars = segment.chars();
        chars.next();
        let rest = chars.as_str();
        // Must not panic, and must stay a suffix of the segment name.
        assert!(segment.ends_with(rest));
    }
}

/// A generation that round-trips through `file_name_from_generation` must parse
/// back to itself, and hostile generations must not abort.
#[test]
fn generation_round_trips_and_extremes_do_not_abort() {
    for gen in [0i64, 1, 2, 35, 36, 1295, 1296, i64::MAX, -1] {
        let Some(name) = file_name_from_generation("_0", "liv", gen) else {
            assert_eq!(gen, -1, "only -1 yields no name");
            continue;
        };
        let parsed = parse_generation(&name);
        if gen == 0 {
            // Generation 0 is encoded by *omitting* the suffix, so it parses
            // back as 0 through the "no generation" branch.
            assert_eq!(parsed.unwrap(), 0, "{name}");
        } else {
            assert_eq!(parsed.unwrap(), gen, "{name}");
        }
    }

    // A hostile suffix must be an error, never an abort.
    for name in corpus() {
        let _ = parse_generation(&segment_file_name("_0", &name, "si"));
    }
}

// -----------------------------------------------------------------------------
// inflate_gens
// -----------------------------------------------------------------------------

/// `inflate_gens` must survive any set of names, one at a time.
///
/// Each name is offered alone so that a failure names the culprit.
#[test]
fn inflate_gens_survives_each_hostile_name_alone() {
    for name in underscore_corpus() {
        let mut infos = SegmentInfos::new(10).unwrap();
        let files = HashSet::from([name.clone()]);
        // Ok or Err; never a panic.
        let _: Result<()> = inflate_gens(&mut infos, &files, &NoOutputInfoStream);
    }
}

/// `inflate_gens` must survive names that look like commit points.
///
/// These take the `segments` / `pending_segments` branches, where Java swallows
/// the `NumberFormatException` and carries on, so an unparsable name must leave
/// the generation untouched rather than failing.
#[test]
fn inflate_gens_ignores_unparsable_commit_names() {
    for fragment in FRAGMENTS {
        for prefix in ["segments_", "pending_segments_"] {
            let mut infos = SegmentInfos::new(10).unwrap();
            let before = infos.generation();
            let files = HashSet::from([format!("{prefix}{fragment}")]);

            inflate_gens(&mut infos, &files, &NoOutputInfoStream)
                .expect("a commit-shaped name must never fail inflate_gens");

            // The generation may only ever move forward.
            assert!(
                infos.generation() >= before,
                "{prefix}{fragment}: generation went backwards"
            );
        }
    }
}

/// The whole corpus at once, so that names interact.
///
/// Ordering matters here: `max_segment_name` and the per-segment generation map
/// accumulate across names, and a `HashSet` gives no ordering guarantee, so this
/// exercises many orders across runs.
#[test]
fn inflate_gens_survives_the_whole_corpus_at_once() {
    let mut infos = SegmentInfos::new(10).unwrap();
    let files: HashSet<String> = underscore_corpus().into_iter().collect();
    let _: Result<()> = inflate_gens(&mut infos, &files, &NoOutputInfoStream);

    // Whatever it decided, the counter must never be left negative: it names the
    // next segment, and a negative name is unwritable.
    if infos.counter < 0 {
        panic!(
            "inflate_gens left a negative segment counter: {}",
            infos.counter
        );
    }
}

/// A name that encodes a segment ordinal too large for `i64` must be reported,
/// not silently wrapped into a counter that would then name a colliding segment.
#[test]
fn an_out_of_range_segment_ordinal_is_an_error_not_a_wrap() {
    let mut infos = SegmentInfos::new(10).unwrap();
    // `i64::MAX + 1` in radix 36.
    let files = HashSet::from(["_1y2p0ij32e8e8.si".to_string()]);

    let result = inflate_gens(&mut infos, &files, &NoOutputInfoStream);

    assert!(
        result.is_err(),
        "an unrepresentable ordinal must be an error; got counter={}",
        infos.counter
    );
}

// -----------------------------------------------------------------------------
// IndexFileDeleter over a hostile directory
// -----------------------------------------------------------------------------

/// Building an `IndexFileDeleter` over a directory full of hostile names must
/// never abort.
///
/// The directory holds no valid commit, so the constructor is expected to fail;
/// the property under test is *how* it fails.
#[test]
fn deleter_construction_over_a_hostile_directory_does_not_abort() {
    let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    // Only names a directory can actually hold: no separators, no empties.
    let names: Vec<String> = corpus()
        .into_iter()
        .filter(|n| !n.is_empty() && !n.contains('/') && !n.contains('\0') && n.len() < 200)
        .collect();
    for name in &names {
        if let Ok(mut out) = directory.create_output(name, &*rucene::store::DEFAULT_IO_CONTEXT) {
            let _ = out.close();
        }
    }

    let mut infos = SegmentInfos::new(10).unwrap();
    let files = directory.list_all().unwrap();

    // Ok or Err; never a panic, and never a deletion of a file it does not
    // understand.
    let result = IndexFileDeleter::new(
        &files,
        Arc::clone(&directory),
        Arc::clone(&directory),
        Arc::new(KeepOnlyLastCommitDeletionPolicy),
        &mut infos,
        Arc::new(NoOutputInfoStream),
        false,
        false,
    );

    if result.is_ok() {
        // If it did succeed, a name that is neither a codec file nor a commit
        // file must still be on disk: the deleter has no claim on it.
        let remaining: HashSet<String> = directory.list_all().unwrap().into_iter().collect();
        for name in &names {
            if !is_codec_file(name)
                && !name.starts_with("segments")
                && !name.starts_with("pending_segments")
            {
                assert!(
                    remaining.contains(name),
                    "{name} is not an index file and must not be deleted"
                );
            }
        }
    }
}

/// A corrupt `segments_N` found by the constructor's directory scan must be
/// reported, not abort, and must not take the healthy commit down with it.
///
/// This is the real path: `IndexFileDeleter` reads **every** `segments*` file in
/// the listing to rebuild past commit points (`IndexFileDeleter.java:161`), not
/// just the newest, so a single unreadable generation is met head-on.
#[test]
fn a_corrupt_older_commit_is_an_error_not_an_abort() {
    for body in [
        Vec::new(),
        vec![0u8; 1],
        vec![0xffu8; 64],
        b"segments".to_vec(),
        vec![0x3f, 0xd7, 0x6c, 0x17], // a plausible-looking codec magic
    ] {
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

        // Two healthy, empty commits: `segments_1` and `segments_2`.
        let mut infos = SegmentInfos::new(10).unwrap();
        infos.changed();
        infos.commit(directory.as_ref()).unwrap();
        infos.changed();
        infos.commit(directory.as_ref()).unwrap();

        // Corrupt the older one, leaving the newest readable.
        directory.delete_file("segments_1").unwrap();
        {
            let mut out = directory
                .create_output("segments_1", &*rucene::store::DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_bytes(&body, 0, body.len()).unwrap();
            out.close().unwrap();
        }

        // The newest commit is still readable, so a writer would get this far.
        let mut latest = SegmentInfos::read_latest_commit(directory.as_ref())
            .expect("the healthy newest commit must still be readable");
        assert_eq!(latest.segments_file_name().as_deref(), Some("segments_2"));

        let files = directory.list_all().unwrap();
        let result = IndexFileDeleter::new(
            &files,
            Arc::clone(&directory),
            Arc::clone(&directory),
            Arc::new(KeepOnlyLastCommitDeletionPolicy),
            &mut latest,
            Arc::new(NoOutputInfoStream),
            true,
            false,
        );

        assert!(
            result.is_err(),
            "an unreadable commit point must be reported, not silently skipped \
             (body of {} bytes)",
            body.len()
        );

        // A failed init must not have destroyed the evidence.
        let remaining: HashSet<String> = directory.list_all().unwrap().into_iter().collect();
        assert!(
            remaining.contains("segments_1") && remaining.contains("segments_2"),
            "a failed init must delete nothing; left {remaining:?}"
        );
    }
}

/// A `SegmentInfos` that names no commit file leaves the deleter with nothing to
/// adopt, and that must be a clean no-op rather than a failure.
///
/// This is the fresh-index case: `IndexWriter` on an empty directory builds the
/// deleter before any `segments_N` exists.
#[test]
fn a_segment_infos_naming_no_commit_file_is_a_clean_no_op() {
    let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    {
        let mut out = directory
            .create_output("_0.fdt", &*rucene::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        out.close().unwrap();
    }

    let mut infos = SegmentInfos::new(10).unwrap();
    assert_eq!(infos.segments_file_name(), None, "premise: no commit yet");

    let files = directory.list_all().unwrap();
    let deleter = IndexFileDeleter::new(
        &files,
        Arc::clone(&directory),
        Arc::clone(&directory),
        Arc::new(KeepOnlyLastCommitDeletionPolicy),
        &mut infos,
        Arc::new(NoOutputInfoStream),
        false,
        false,
    )
    .expect("an index with no commit yet must not fail the deleter");

    assert!(deleter.commits().is_empty());
    // Nothing was adopted, so nothing may have been deleted either.
    assert!(directory
        .list_all()
        .unwrap()
        .contains(&"_0.fdt".to_string()));
}
