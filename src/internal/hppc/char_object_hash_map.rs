//! Port of `org.apache.lucene.internal.hppc.CharObjectHashMap`.

use super::macros::define_object_hash_map;

define_object_hash_map! {
    map = CharObjectHashMap,
    cursor = CharObjectCursor,
    key = u16,
    key_cursor = CharCursor,
    key_zero = 0,
    hash_key = BitMixer::mix_phi_u16,
    mix_key = BitMixer::mix_u16,
    size_of_keys = super::support::size_of_char_array,
    base_ram_bytes_used = 48,
    java_class = "CharObjectHashMap",
    java_key = "char",
}
