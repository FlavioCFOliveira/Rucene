//! Port of `org.apache.lucene.internal.hppc.IntLongHashMap`.

use super::macros::define_primitive_hash_map;

define_primitive_hash_map! {
    map = IntLongHashMap,
    cursor = IntLongCursor,
    key = i32,
    value = i64,
    key_cursor = IntCursor,
    value_cursor = LongCursor,
    key_zero = 0,
    value_zero = 0,
    hash_key = BitMixer::mix_phi_i32,
    mix_key = BitMixer::mix_i32,
    mix_value = BitMixer::mix_i64,
    eq_value = super::support::eq_i64,
    add_value = super::support::add_i64,
    size_of_keys = super::support::size_of_int_array,
    size_of_values = super::support::size_of_long_array,
    base_ram_bytes_used = 48,
    java_class = "IntLongHashMap",
    java_key = "int",
    java_value = "long",
    value_fmt = "",
}
