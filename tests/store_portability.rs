//! Store-layer portability tests against Apache Lucene Core 10.5.0.
//!
//! These tests verify that Rucene's in-heap and filesystem-backed Directory
//! implementations produce byte-identical output and matching CRC-32 checksums
//! for the same payload written by Java Lucene 10.5.0.
//!
//! The reference bytes and checksums below were generated with the Java fixture
//! generator in `tests/fixtures/GenerateStoreFixtures.java`, compiled with the
//! Eclipse ECJ compiler against `lucene-core-10.5.0.jar`.

use std::collections::HashSet;

use rucene::store::{
    BufferedChecksum, ByteArrayDataInput, DataInput, Directory, FSDirectory, IndexInput,
    IndexOutput, RamDirectory, DEFAULT_IO_CONTEXT,
};
use rucene::Result;

/// Base64-encoded payload produced by Java Lucene 10.5.0 `ByteBuffersDirectory`.
const JAVA_PAYLOAD_B64: &str =
    "ATQS776t3u/Nq5B4VjQSgIABgKCUpY0dF1J1Y2VuZSBwb3J0YWJpbGl0eSB0ZXN0CgsMDQ==";

/// Expected length of the Java payload.
const JAVA_PAYLOAD_LEN: usize = 52;

/// CRC-32 of the Java payload, computed with `java.util.zip.CRC32`.
const JAVA_PAYLOAD_CRC32: i64 = 1_775_344_477;

/// Base64-encoded payload for the simple "hello" fixture.
const JAVA_HELLO_B64: &str = "DEhlbGxvIEx1Y2VuZQ==";

/// CRC-32 of the "hello" fixture.
const JAVA_HELLO_CRC32: i64 = 3_713_613_496;

/// Writes the standard portability payload through the supplied output.
fn write_portability_payload(out: &mut dyn IndexOutput) -> Result<()> {
    out.write_byte(0x01)?;
    out.write_short(0x1234)?;
    out.write_int(-559038737_i32)?;
    out.write_long(0x1234_5678_90AB_CDEF_i64)?;
    out.write_v_int(16_384)?;
    out.write_v_long(1_000_000_000_000_i64)?;
    out.write_string("Rucene portability test")?;
    out.write_bytes(&[0x0A, 0x0B, 0x0C, 0x0D], 0, 4)?;
    Ok(())
}

/// Reads the standard portability payload from the supplied input and asserts
/// it matches the values written by the Java fixture generator.
fn assert_portability_payload(input: &mut dyn IndexInput) -> Result<()> {
    assert_eq!(input.read_byte()?, 0x01);
    assert_eq!(input.read_short()?, 0x1234);
    assert_eq!(input.read_int()?, -559038737_i32);
    assert_eq!(input.read_long()?, 0x1234_5678_90AB_CDEF_i64);
    assert_eq!(input.read_v_int()?, 16_384);
    assert_eq!(input.read_v_long()?, 1_000_000_000_000_i64);
    assert_eq!(input.read_string()?, "Rucene portability test");
    let mut tail = [0u8; 4];
    input.read_bytes(&mut tail, 0, 4)?;
    assert_eq!(tail, [0x0A, 0x0B, 0x0C, 0x0D]);
    Ok(())
}

/// Returns the reference Java payload bytes.
fn java_payload_bytes() -> Vec<u8> {
    let bytes = base64::decode(JAVA_PAYLOAD_B64).expect("embedded base64 is valid");
    assert_eq!(bytes.len(), JAVA_PAYLOAD_LEN);
    bytes
}

/// Returns the "hello" fixture bytes.
fn java_hello_bytes() -> Vec<u8> {
    base64::decode(JAVA_HELLO_B64).expect("embedded base64 is valid")
}

/// CRC-32 of `data`, matching `java.util.zip.CRC32`.
fn crc32_of(data: &[u8]) -> i64 {
    let mut hasher = BufferedChecksum::new();
    hasher
        .update_bytes(data, 0, data.len())
        .expect("valid range");
    hasher.get_value()
}

#[test]
fn java_payload_bytes_match_reference_crc32() {
    let payload = java_payload_bytes();
    assert_eq!(crc32_of(&payload), JAVA_PAYLOAD_CRC32);
}

#[test]
fn java_hello_payload_bytes_match_reference_crc32() {
    let payload = java_hello_bytes();
    assert_eq!(crc32_of(&payload), JAVA_HELLO_CRC32);
}

#[test]
fn ram_directory_writes_java_compatible_payload() -> Result<()> {
    let dir = RamDirectory::new();
    {
        let mut out = dir.create_output("test.bin", &*DEFAULT_IO_CONTEXT)?;
        write_portability_payload(out.as_mut())?;
        assert_eq!(out.checksum()?, JAVA_PAYLOAD_CRC32);
        out.close()?;
    }

    let mut input = dir.open_input("test.bin", &*DEFAULT_IO_CONTEXT)?;
    assert_eq!(input.length(), JAVA_PAYLOAD_LEN as i64);
    assert_portability_payload(input.as_mut())?;

    // Read back the full bytes to prove byte equality with the Java fixture.
    input.seek(0)?;
    let mut rust_bytes = vec![0u8; JAVA_PAYLOAD_LEN];
    input.read_bytes(&mut rust_bytes, 0, JAVA_PAYLOAD_LEN)?;
    assert_eq!(rust_bytes, java_payload_bytes());
    Ok(())
}

#[test]
fn fs_directory_writes_java_compatible_payload() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = FSDirectory::open(tmp.path())?;
    {
        let mut out = dir.create_output("test.bin", &*DEFAULT_IO_CONTEXT)?;
        write_portability_payload(out.as_mut())?;
        assert_eq!(out.checksum()?, JAVA_PAYLOAD_CRC32);
        out.close()?;
    }

    // The file on disk must be byte-identical to the Java fixture.
    let rust_bytes = std::fs::read(tmp.path().join("test.bin"))?;
    assert_eq!(rust_bytes, java_payload_bytes());

    let mut input = dir.open_input("test.bin", &*DEFAULT_IO_CONTEXT)?;
    assert_portability_payload(input.as_mut())?;
    Ok(())
}

#[test]
fn rucene_reads_java_hello_fixture() -> Result<()> {
    let bytes = java_hello_bytes();
    let mut input = ByteArrayDataInput::new(bytes);
    assert_eq!(input.read_string()?, "Hello Lucene");
    Ok(())
}

#[test]
fn rucene_crc32_matches_java_reference_for_known_payloads() -> Result<()> {
    // Recompute the CRC-32 by writing through Rucene's BufferedChecksum and
    // verify parity with the Java reference values.
    let payload = java_payload_bytes();
    assert_eq!(crc32_of(&payload), JAVA_PAYLOAD_CRC32);

    let hello = java_hello_bytes();
    assert_eq!(crc32_of(&hello), JAVA_HELLO_CRC32);

    // Ensure a fresh payload written through a Rucene output also matches.
    let dir = RamDirectory::new();
    {
        let mut out = dir.create_output("crc.bin", &*DEFAULT_IO_CONTEXT)?;
        out.write_bytes(&payload, 0, payload.len())?;
        assert_eq!(out.checksum()?, JAVA_PAYLOAD_CRC32);
        out.close()?;
    }

    // The on-disk (in-heap) bytes must be identical.
    let mut input = dir.open_input("crc.bin", &*DEFAULT_IO_CONTEXT)?;
    let mut rust_bytes = vec![0u8; payload.len()];
    input.read_bytes(&mut rust_bytes, 0, payload.len())?;
    assert_eq!(rust_bytes, payload);

    Ok(())
}

#[test]
fn fs_directory_lists_and_deletes_files_like_java_directory() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = FSDirectory::open(tmp.path())?;

    {
        let mut out = dir.create_output("a.bin", &*DEFAULT_IO_CONTEXT)?;
        out.write_byte(0x01)?;
        out.close()?;
    }
    {
        let mut out = dir.create_output("b.bin", &*DEFAULT_IO_CONTEXT)?;
        out.write_byte(0x02)?;
        out.close()?;
    }

    let mut names: HashSet<String> = dir.list_all()?.into_iter().collect();
    assert_eq!(
        names,
        ["a.bin".to_string(), "b.bin".to_string()]
            .into_iter()
            .collect()
    );

    dir.delete_file("a.bin")?;
    names = dir.list_all()?.into_iter().collect();
    assert_eq!(names, ["b.bin".to_string()].into_iter().collect());

    Ok(())
}

// Minimal base64 decoder so the tests have no extra dev-dependencies.
mod base64 {
    pub fn decode(input: &str) -> Option<Vec<u8>> {
        let mut output = Vec::with_capacity(input.len() * 3 / 4);
        let mut buf = 0u32;
        let mut bits = 0u32;
        let mut padding_seen = false;

        for ch in input.bytes() {
            if ch == b'=' {
                padding_seen = true;
                continue;
            }

            // Data characters are not allowed after padding starts.
            if padding_seen {
                return None;
            }

            let value = match ch {
                b'A'..=b'Z' => ch - b'A',
                b'a'..=b'z' => ch - b'a' + 26,
                b'0'..=b'9' => ch - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return None,
            } as u32;

            buf = (buf << 6) | value;
            bits += 6;

            if bits >= 8 {
                bits -= 8;
                output.push(((buf >> bits) & 0xFF) as u8);
            }
        }

        Some(output)
    }

    #[test]
    fn decoder_round_trips_known_values() {
        assert_eq!(decode(""), Some(vec![]));
        assert_eq!(decode("QQ=="), Some(vec![b'A']));
        assert_eq!(decode("QUI="), Some(vec![b'A', b'B']));
        assert_eq!(decode("QUJD"), Some(vec![b'A', b'B', b'C']));
    }
}
