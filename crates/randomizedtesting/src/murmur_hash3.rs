//! Port of `com.carrotsearch.randomizedtesting.MurmurHash3`
//! (randomizedtesting 2.8.1, `MurmurHash3.java`).

/// Hash routines for primitive types: the finalization step of Austin
/// Appleby's MurmurHash3. Java's `MurmurHash3` is package-private.
pub(crate) struct MurmurHash3;

impl MurmurHash3 {
    /// `MurmurHash3.hash(int)`: hashes a 4-byte sequence.
    #[allow(dead_code)] // Java's class carries it; no 2.8.1 caller is ported.
    pub(crate) fn hash_int(k: i32) -> i32 {
        let mut k = k as u32;
        k ^= k >> 16;
        k = k.wrapping_mul(0x85eb_ca6b);
        k ^= k >> 13;
        k = k.wrapping_mul(0xc2b2_ae35);
        k ^= k >> 16;
        k as i32
    }

    /// `MurmurHash3.hash(long)`: hashes an 8-byte sequence.
    pub(crate) fn hash_long(k: i64) -> i64 {
        let mut k = k as u64;
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        k ^= k >> 33;
        k as i64
    }
}
