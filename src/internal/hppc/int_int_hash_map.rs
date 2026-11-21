//! Port of `org.apache.lucene.internal.hppc.IntIntHashMap`.

use super::macros::define_primitive_hash_map;

define_primitive_hash_map! {
    map = IntIntHashMap,
    cursor = IntIntCursor,
    key = i32,
    value = i32,
    key_cursor = IntCursor,
    value_cursor = IntCursor,
    key_zero = 0,
    value_zero = 0,
    hash_key = BitMixer::mix_phi_i32,
    mix_key = BitMixer::mix_i32,
    mix_value = BitMixer::mix_i32,
    eq_value = super::support::eq_i32,
    add_value = super::support::add_i32,
    size_of_keys = super::support::size_of_int_array,
    size_of_values = super::support::size_of_int_array,
    base_ram_bytes_used = 48,
    java_class = "IntIntHashMap",
    java_key = "int",
    java_value = "int",
    value_fmt = "",
}
