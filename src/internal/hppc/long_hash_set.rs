//! Port of `org.apache.lucene.internal.hppc.LongHashSet`.

use super::macros::define_hash_set;

define_hash_set! {
    set = LongHashSet,
    key = i64,
    cursor = LongCursor,
    key_zero = 0,
    hash_key = BitMixer::mix_phi_i64,
    mix_key = BitMixer::mix_i64,
    size_of_keys = super::support::size_of_long_array,
    base_ram_bytes_used = 48,
    java_class = "LongHashSet",
    java_key = "long",
}
