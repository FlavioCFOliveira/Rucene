//! `CodecUtil.checkHeaderNoMagic` raises Lucene's exceptions.
//!
//! A header whose version is below the accepted range raises
//! `IndexFormatTooOldException(in, version, minVersion, maxVersion)`, one above
//! it `IndexFormatTooNewException(in, version, minVersion, maxVersion)`, and a
//! codec name that differs `CorruptIndexException(message, in)` — each naming
//! the input, `Objects.toString(in)`, as its resource
//! (`CodecUtil.java:201-218`). Rucene writes a header and an index header to
//! a directory; `CodecHeaderFixture` (`tests/fixtures/java-codec-harness`, run
//! with `java -cp lucene-core-10.5.0.jar CodecHeaderFixture.java`) reads them
//! through an `NIOFSDirectory`, with a plain and a checksum input, and prints
//! every exception; Rucene, reading them the same way, must print the same.
//!
//! The other header checks are read the same way: a file without the codec
//! magic (`CodecUtil.java:182-194`), an index header against another id and an
//! id of the wrong length (`CodecUtil.java:363-375`), against another suffix
//! and with a suffix that is not valid UTF-8 (`CodecUtil.java:378-389`); and a
//! header followed by one mebibyte in a `ByteBuffersDirectory`, whose inputs
//! describe themselves with the data input's `toString()`
//! (`ByteBuffersDirectory.java:51-78`).
//!
//! `verifyAndCopyIndexHeader` (`CodecUtil.java:274-314`) and `readIndexHeader`
//! (`CodecUtil.java:320-339`) raise `CorruptIndexException(String, DataInput)`
//! too, naming the input: a file too small for a header and a footer, and
//! files without the codec magic. A header whose codec name is not valid UTF-8
//! is read by `checkHeader`, whose `DataInput.readString` decodes it as the
//! JDK does, never failing (`DataInput.java:230-235`); the codec mismatch it
//! then raises carries the decoded name.
//!
//! The footer checks raise `CorruptIndexException(String, DataInput)` naming
//! their input too: `readFooter` (`CodecUtil.java:345-360`),
//! `retrieveChecksum` with and without an expected length
//! (`CodecUtil.java:525-558`, whose messages print `expectedLength==`),
//! `validateFooter` (`CodecUtil.java:560-598`), `checksumEntireFile`
//! (`CodecUtil.java:606-621`, naming the input it was given, not its clone),
//! `readCRC` (`CodecUtil.java:629-635`) and `checkFooter`
//! (`CodecUtil.java:432-445`).

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use rucene::codecs::codec_util;
use rucene::store::{
    BufferedChecksumIndexInput, ByteBuffersDataOutput, ByteBuffersDirectory, Directory, IndexInput,
    NIOFSDirectory, DEFAULT_IO_CONTEXT,
};
use rucene::{LuceneError, LuceneErrorKind};

/// `CodecHeaderFixture.ID`.
const ID: [u8; 16] = [0; 16];

/// `CodecHeaderFixture.OTHER_ID`.
const OTHER_ID: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// `CodecHeaderFixture.SHORT_ID`.
const SHORT_ID: [u8; 15] = [0; 15];

/// `CodecHeaderFixture.BAD_SUFFIX`.
const BAD_SUFFIX: [u8; 14] = [
    0x61, 0xFF, 0xC3, 0x28, 0xE2, 0x82, 0x41, 0xF0, 0x9F, 0x98, 0x80, 0xED, 0xA0, 0x80,
];

/// `CodecHeaderFixture.escaped(s)`: every UTF-16 unit outside printable
/// ASCII as `\uXXXX`.
fn escaped(s: &str) -> String {
    let mut out = String::new();
    for unit in s.encode_utf16() {
        if (0x20..=0x7E).contains(&unit) {
            out.push(char::from(unit as u8));
        } else {
            write!(out, "\\u{unit:04X}").unwrap();
        }
    }
    out
}

fn report(
    out: &mut String,
    dir: &dyn Directory,
    file: &str,
    checksum: bool,
    label: &str,
    check: impl Fn(&mut dyn IndexInput) -> rucene::Result<i32>,
) {
    let mut input: Box<dyn IndexInput> = if checksum {
        dir.open_checksum_input(file).unwrap()
    } else {
        dir.open_input(file, &*DEFAULT_IO_CONTEXT).unwrap()
    };
    let outcome = check(input.as_mut());
    input.close().unwrap();
    match outcome {
        Ok(_) => writeln!(out, "{label}: ok").unwrap(),
        Err(
            error @ LuceneError {
                kind: LuceneErrorKind::IndexFormatTooOld { .. },
                ..
            },
        ) => {
            writeln!(out, "{label}: IndexFormatTooOldException: {error}").unwrap();
            writeln!(
                out,
                "{label}  resource={} version={} min={} max={} reason={}",
                error.resource_description().unwrap(),
                error.version().unwrap(),
                error.min_version().unwrap(),
                error.max_version().unwrap(),
                error.reason().unwrap_or("null")
            )
            .unwrap();
        }
        Err(
            error @ LuceneError {
                kind: LuceneErrorKind::IndexFormatTooNew { .. },
                ..
            },
        ) => {
            writeln!(out, "{label}: IndexFormatTooNewException: {error}").unwrap();
            writeln!(
                out,
                "{label}  resource={} version={} min={} max={}",
                error.resource_description().unwrap(),
                error.version().unwrap(),
                error.min_version().unwrap(),
                error.max_version().unwrap()
            )
            .unwrap();
        }
        Err(LuceneError {
            kind: LuceneErrorKind::CorruptIndex { message, .. },
            ..
        }) => {
            writeln!(out, "{label}: CorruptIndexException: {}", escaped(&message)).unwrap();
        }
        Err(other) => panic!("{label}: unexpected {other:?}"),
    }
}

/// [`report`] over a checksum input kept as one, for `checkFooter`.
fn report_checksum(
    out: &mut String,
    dir: &dyn Directory,
    file: &str,
    label: &str,
    check: impl Fn(&mut BufferedChecksumIndexInput) -> rucene::Result<i64>,
) {
    let mut input = dir.open_checksum_input(file).unwrap();
    let outcome = check(&mut input);
    input.close().unwrap();
    match outcome {
        Ok(_) => writeln!(out, "{label}: ok").unwrap(),
        Err(LuceneError {
            kind: LuceneErrorKind::CorruptIndex { message, .. },
            ..
        }) => {
            writeln!(out, "{label}: CorruptIndexException: {}", escaped(&message)).unwrap();
        }
        Err(other) => panic!("{label}: unexpected {other:?}"),
    }
}

/// `CodecHeaderFixture.writeFooters(dir)`.
fn write_footers(dir: &dyn Directory) {
    let write = |name: &str, fill: &dyn Fn(&mut dyn rucene::store::IndexOutput)| {
        let mut output = dir.create_output(name, &*DEFAULT_IO_CONTEXT).unwrap();
        fill(output.as_mut());
        output.close().unwrap();
    };
    write("tiny.bin", &|out| {
        out.write_bytes(&[1, 2, 3, 4], 0, 4).unwrap()
    });
    write("footer.bin", &|out| {
        codec_util::write_header(out, "TestCodec", 5).unwrap();
        out.write_bytes(&[7, 7, 7], 0, 3).unwrap();
        codec_util::write_footer(out).unwrap();
    });
    write("badfooter.bin", &|out| {
        out.write_bytes(&[0; 20], 0, 20).unwrap()
    });
    let footer = |out: &mut dyn rucene::store::IndexOutput, algorithm: i32, crc: i64| {
        out.write_bytes(&[0; 4], 0, 4).unwrap();
        codec_util::write_be_int(out, codec_util::FOOTER_MAGIC).unwrap();
        codec_util::write_be_int(out, algorithm).unwrap();
        codec_util::write_be_long(out, crc).unwrap();
    };
    write("badalgo.bin", &|out| footer(out, 1, 0));
    write("badcrc.bin", &|out| footer(out, 0, 0x1_0000_0000));
    write("badsum.bin", &|out| footer(out, 0, 12345));
}

/// `CodecHeaderFixture.readFooters(dir)`.
fn read_footers(out: &mut String, dir: &dyn Directory) {
    report(out, dir, "tiny.bin", false, "read footer short", |input| {
        codec_util::read_footer(input).map(|_| 0)
    });
    report(out, dir, "tiny.bin", false, "retrieve short", |input| {
        codec_util::retrieve_checksum(input).map(|_| 0)
    });
    report(out, dir, "tiny.bin", false, "entire short", |input| {
        codec_util::checksum_entire_file(input).map(|_| 0)
    });
    report(out, dir, "footer.bin", false, "expected longer", |input| {
        let length = input.length();
        codec_util::retrieve_checksum_expected_length(input, length + 1).map(|_| 0)
    });
    report(out, dir, "footer.bin", false, "expected shorter", |input| {
        let length = input.length();
        codec_util::retrieve_checksum_expected_length(input, length - 1).map(|_| 0)
    });
    report(out, dir, "badfooter.bin", false, "footer magic", |input| {
        codec_util::retrieve_checksum(input).map(|_| 0)
    });
    report(
        out,
        dir,
        "badalgo.bin",
        false,
        "footer algorithm",
        |input| codec_util::read_footer(input).map(|_| 0),
    );
    report(out, dir, "badcrc.bin", false, "footer crc", |input| {
        codec_util::retrieve_checksum(input).map(|_| 0)
    });
    report(out, dir, "badsum.bin", false, "entire checksum", |input| {
        codec_util::checksum_entire_file(input).map(|_| 0)
    });
    report_checksum(out, dir, "footer.bin", "check footer extended", |input| {
        codec_util::check_footer(input)
    });
    report_checksum(out, dir, "footer.bin", "check footer truncated", |input| {
        let target = IndexInput::length(input) - 10;
        rucene::store::ChecksumIndexInput::seek(input, target)?;
        codec_util::check_footer(input)
    });
}

fn rucene_report(path: &Path) -> String {
    let dir = NIOFSDirectory::open(path).unwrap();
    let mut output = dir
        .create_output("plain.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    codec_util::write_header(output.as_mut(), "TestCodec", 5).unwrap();
    output.close().unwrap();
    let mut output = dir
        .create_output("index.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    codec_util::write_index_header(output.as_mut(), "TestCodec", 5, &ID, "sfx").unwrap();
    output.close().unwrap();
    // `CodecHeaderFixture.writeExtra(dir)`.
    let mut output = dir
        .create_output("nomagic.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    output.write_bytes(&[0x12, 0x34, 0x56, 0x78], 0, 4).unwrap();
    output.close().unwrap();
    let mut output = dir
        .create_output("badsuffix.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    codec_util::write_header(output.as_mut(), "TestCodec", 5).unwrap();
    output.write_bytes(&ID, 0, ID.len()).unwrap();
    output.write_byte(BAD_SUFFIX.len() as u8).unwrap();
    output
        .write_bytes(&BAD_SUFFIX, 0, BAD_SUFFIX.len())
        .unwrap();
    output.close().unwrap();
    let mut output = dir
        .create_output("nomagiclong.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    output.write_bytes(&[0x12, 0x34, 0x56, 0x78], 0, 4).unwrap();
    output.write_bytes(&[0u8; 28], 0, 28).unwrap();
    output.close().unwrap();
    let mut output = dir
        .create_output("badcodec.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    codec_util::write_be_int(output.as_mut(), codec_util::CODEC_MAGIC).unwrap();
    output.write_v_int(BAD_SUFFIX.len() as i32).unwrap();
    output
        .write_bytes(&BAD_SUFFIX, 0, BAD_SUFFIX.len())
        .unwrap();
    codec_util::write_be_int(output.as_mut(), 5).unwrap();
    output.close().unwrap();
    write_footers(&dir);

    let mut out = String::new();
    read_extra(&mut out, &dir);
    let memory = ByteBuffersDirectory::new();
    let mut output = memory
        .create_output("big.bin", &*DEFAULT_IO_CONTEXT)
        .unwrap();
    codec_util::write_header(output.as_mut(), "TestCodec", 5).unwrap();
    output.write_bytes(&vec![0u8; 1 << 20], 0, 1 << 20).unwrap();
    output.close().unwrap();
    for checksum in [false, true] {
        let p = if checksum {
            "memory checksum "
        } else {
            "memory plain "
        };
        report(
            &mut out,
            &memory,
            "big.bin",
            checksum,
            &format!("{p}too-old"),
            |input| codec_util::check_header(input, "TestCodec", 6, 10),
        );
        report(
            &mut out,
            &memory,
            "big.bin",
            checksum,
            &format!("{p}codec"),
            |input| codec_util::check_header(input, "OtherCodec", 1, 10),
        );
    }
    read_footers(&mut out, &dir);
    for checksum in [false, true] {
        let p = if checksum { "checksum " } else { "plain " };
        report(
            &mut out,
            &dir,
            "plain.bin",
            checksum,
            &format!("{p}too-old"),
            |input| codec_util::check_header(input, "TestCodec", 6, 10),
        );
        report(
            &mut out,
            &dir,
            "plain.bin",
            checksum,
            &format!("{p}too-new"),
            |input| codec_util::check_header(input, "TestCodec", 1, 4),
        );
        report(
            &mut out,
            &dir,
            "plain.bin",
            checksum,
            &format!("{p}codec"),
            |input| codec_util::check_header(input, "OtherCodec", 1, 10),
        );
        report(
            &mut out,
            &dir,
            "plain.bin",
            checksum,
            &format!("{p}in-range"),
            |input| codec_util::check_header(input, "TestCodec", 5, 5),
        );
        report(
            &mut out,
            &dir,
            "index.bin",
            checksum,
            &format!("{p}index too-new"),
            |input| codec_util::check_index_header(input, "TestCodec", 1, 4, &ID, "sfx"),
        );
        report(
            &mut out,
            &dir,
            "index.bin",
            checksum,
            &format!("{p}index codec"),
            |input| codec_util::check_index_header(input, "OtherCodec", 1, 10, &ID, "sfx"),
        );
    }
    out
}

/// `CodecHeaderFixture.readExtra(dir)`.
fn read_extra(out: &mut String, dir: &dyn Directory) {
    for checksum in [false, true] {
        let p = if checksum { "checksum " } else { "plain " };
        report(
            out,
            dir,
            "nomagic.bin",
            checksum,
            &format!("{p}magic"),
            |input| codec_util::check_header(input, "TestCodec", 1, 10),
        );
        report(
            out,
            dir,
            "index.bin",
            checksum,
            &format!("{p}index id"),
            |input| codec_util::check_index_header(input, "TestCodec", 1, 10, &OTHER_ID, "sfx"),
        );
        report(
            out,
            dir,
            "index.bin",
            checksum,
            &format!("{p}index short id"),
            |input| codec_util::check_index_header(input, "TestCodec", 1, 10, &SHORT_ID, "sfx"),
        );
        report(
            out,
            dir,
            "index.bin",
            checksum,
            &format!("{p}index suffix"),
            |input| codec_util::check_index_header(input, "TestCodec", 1, 10, &ID, "xfs"),
        );
        report(
            out,
            dir,
            "badsuffix.bin",
            checksum,
            &format!("{p}index bad suffix"),
            |input| codec_util::check_index_header(input, "TestCodec", 1, 10, &ID, "sfx"),
        );
        for (file, label, id) in [
            ("nomagic.bin", "copy small", &ID),
            ("nomagiclong.bin", "copy magic", &ID),
            ("index.bin", "copy id", &OTHER_ID),
            ("index.bin", "copy ok", &ID),
        ] {
            report(out, dir, file, checksum, &format!("{p}{label}"), |input| {
                let mut copy = ByteBuffersDataOutput::new();
                codec_util::verify_and_copy_index_header(input, &mut copy, id).map(|()| 0)
            });
        }
        report(
            out,
            dir,
            "nomagic.bin",
            checksum,
            &format!("{p}read header magic"),
            |input| codec_util::read_index_header(input).map(|header| header.len() as i32),
        );
        report(
            out,
            dir,
            "badcodec.bin",
            checksum,
            &format!("{p}bad codec"),
            |input| codec_util::check_header(input, "TestCodec", 1, 10),
        );
    }
    // A checksum input cannot seek back to the start once the header was read
    // (`ChecksumIndexInput.java:57-65`).
    report(out, dir, "index.bin", false, "plain read header", |input| {
        codec_util::read_index_header(input).map(|header| header.len() as i32)
    });
}

fn java_report(path: &Path) -> String {
    let jar = Path::new(&std::env::var("HOME").unwrap())
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         CodecHeaderFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(jar)
        .arg(fixture)
        .arg("read")
        .arg(path)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "CodecHeaderFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Too old, too new, a wrong codec name, a missing magic, a wrong id and a
/// wrong suffix raise Lucene's exception classes, with Lucene's messages,
/// resources and version triples, through file and in-memory inputs.
#[test]
fn header_failures_raise_lucenes_exceptions() {
    let dir = tempfile::tempdir().unwrap();
    let rust = rucene_report(dir.path());
    assert_eq!(rust, java_report(dir.path()));
}
