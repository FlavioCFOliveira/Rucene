//! Defensive fuzz-style tests for codec header/footer validation.
//!
//! These tests verify that `CodecUtil` rejects malformed headers, footers,
//! checksum mismatches and truncated files without panicking.

use rucene::codecs::codec_util::{
    check_footer, check_header, check_index_header, write_footer, write_header, write_index_header,
};
use rucene::store::{
    BufferedChecksumIndexInput, DataOutput, IndexInput, MockIndexInput, MockIndexOutput,
};
use rucene::util::string_helper::ID_LENGTH;

fn valid_header_bytes(codec: &str, version: i32) -> Vec<u8> {
    let mut out = MockIndexOutput::new("header", "test");
    write_header(&mut out, codec, version).unwrap();
    out.into_inner()
}

fn valid_index_header_bytes(codec: &str, version: i32, id: &[u8], suffix: &str) -> Vec<u8> {
    let mut out = MockIndexOutput::new("index_header", "test");
    write_index_header(&mut out, codec, version, id, suffix).unwrap();
    out.into_inner()
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = valid_header_bytes("TestCodec", 0);
    bytes[0] ^= 0xFF;
    let mut input = MockIndexInput::new(bytes, "bad_magic");
    assert!(check_header(&mut input, "TestCodec", 0, 0).is_err());
}

#[test]
fn wrong_codec_name_is_rejected() {
    let bytes = valid_header_bytes("TestCodec", 0);
    let mut input = MockIndexInput::new(bytes, "wrong_name");
    assert!(check_header(&mut input, "OtherCodec", 0, 0).is_err());
}

#[test]
fn version_too_old_is_rejected() {
    let bytes = valid_header_bytes("TestCodec", 5);
    let mut input = MockIndexInput::new(bytes, "old_version");
    assert!(check_header(&mut input, "TestCodec", 6, 10).is_err());
}

#[test]
fn version_too_new_is_rejected() {
    let bytes = valid_header_bytes("TestCodec", 11);
    let mut input = MockIndexInput::new(bytes, "new_version");
    assert!(check_header(&mut input, "TestCodec", 6, 10).is_err());
}

#[test]
fn truncated_header_is_rejected() {
    let bytes = valid_header_bytes("TestCodec", 0);
    for len in 1..bytes.len() {
        let mut input = MockIndexInput::new(bytes[..len].to_vec(), "truncated");
        assert!(check_header(&mut input, "TestCodec", 0, 0).is_err());
    }
}

#[test]
fn wrong_id_in_index_header_is_rejected() {
    let id = [0u8; ID_LENGTH];
    let mut wrong_id = id;
    wrong_id[0] = 0xFF;
    let bytes = valid_index_header_bytes("TestCodec", 0, &id, "");
    let mut input = MockIndexInput::new(bytes, "wrong_id");
    assert!(check_index_header(&mut input, "TestCodec", 0, 0, &wrong_id, "").is_err());
}

#[test]
fn wrong_suffix_in_index_header_is_rejected() {
    let id = [0u8; ID_LENGTH];
    let bytes = valid_index_header_bytes("TestCodec", 0, &id, "suffix");
    let mut input = MockIndexInput::new(bytes, "wrong_suffix");
    assert!(check_index_header(&mut input, "TestCodec", 0, 0, &id, "other").is_err());
}

fn make_file_with_footer() -> Vec<u8> {
    let mut out = MockIndexOutput::new("footer", "test");
    out.write_byte(0).unwrap();
    write_footer(&mut out).unwrap();
    out.into_inner()
}

#[test]
fn truncated_footer_is_rejected() {
    let bytes = make_file_with_footer();
    for len in 0..bytes.len() {
        let input = MockIndexInput::new(bytes[..len].to_vec(), "truncated");
        let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
        IndexInput::seek(&mut checksum_input, 0i64).unwrap();
        assert!(check_footer(&mut checksum_input).is_err());
    }
}

#[test]
fn bad_footer_magic_is_rejected() {
    let mut bytes = make_file_with_footer();
    let footer_start = bytes.len() - 16;
    bytes[footer_start] ^= 0xFF;
    let input = MockIndexInput::new(bytes, "bad_magic");
    let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
    IndexInput::seek(&mut checksum_input, footer_start as i64).unwrap();
    assert!(check_footer(&mut checksum_input).is_err());
}

#[test]
fn checksum_mismatch_is_rejected() {
    let mut bytes = make_file_with_footer();
    let len = bytes.len();
    bytes[len - 1] ^= 0xFF;
    let input = MockIndexInput::new(bytes, "checksum");
    let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
    IndexInput::seek(&mut checksum_input, (len - 16) as i64).unwrap();
    assert!(check_footer(&mut checksum_input).is_err());
}

#[test]
fn algorithm_id_corruption_is_rejected() {
    let mut bytes = make_file_with_footer();
    let footer_start = bytes.len() - 16;
    bytes[footer_start + 4] = 1;
    let input = MockIndexInput::new(bytes, "algorithm");
    let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
    IndexInput::seek(&mut checksum_input, footer_start as i64).unwrap();
    assert!(check_footer(&mut checksum_input).is_err());
}
