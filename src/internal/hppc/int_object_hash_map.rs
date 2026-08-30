//! Port of `org.apache.lucene.internal.hppc.IntObjectHashMap`.

use super::macros::define_object_hash_map;

define_object_hash_map! {
    map = IntObjectHashMap,
    cursor = IntObjectCursor,
    key = i32,
    key_cursor = IntCursor,
    key_zero = 0,
    hash_key = BitMixer::mix_phi_i32,
    mix_key = BitMixer::mix_i32,
    size_of_keys = super::support::size_of_int_array,
    base_ram_bytes_used = 48,
    java_class = "IntObjectHashMap",
    java_key = "int",
}
