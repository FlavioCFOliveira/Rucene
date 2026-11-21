//! Port of `org.apache.lucene.internal.hppc.LongObjectHashMap`.

use super::macros::define_object_hash_map;

define_object_hash_map! {
    map = LongObjectHashMap,
    cursor = LongObjectCursor,
    key = i64,
    key_cursor = LongCursor,
    key_zero = 0,
    hash_key = BitMixer::mix_phi_i64,
    mix_key = BitMixer::mix_i64,
    size_of_keys = super::support::size_of_long_array,
    base_ram_bytes_used = 48,
    java_class = "LongObjectHashMap",
    java_key = "long",
}
