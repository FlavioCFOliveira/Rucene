//! Port of `org.apache.lucene.internal.hppc.IntDoubleHashMap`.

use super::macros::define_primitive_hash_map;

define_primitive_hash_map! {
    map = IntDoubleHashMap,
    cursor = IntDoubleCursor,
    key = i32,
    value = f64,
    key_cursor = IntCursor,
    value_cursor = DoubleCursor,
    key_zero = 0,
    value_zero = 0.0,
    hash_key = BitMixer::mix_phi_i32,
    mix_key = BitMixer::mix_i32,
    mix_value = BitMixer::mix_f64,
    eq_value = super::support::eq_f64_bits,
    add_value = super::support::add_f64,
    size_of_keys = super::support::size_of_int_array,
    size_of_values = super::support::size_of_double_array,
    base_ram_bytes_used = 48,
    java_class = "IntDoubleHashMap",
    java_key = "int",
    java_value = "double",
    value_fmt = ":?",
}
