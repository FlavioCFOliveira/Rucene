//! Port of `org.apache.lucene.internal.hppc.IntFloatHashMap`.

use super::macros::define_primitive_hash_map;

define_primitive_hash_map! {
    map = IntFloatHashMap,
    cursor = IntFloatCursor,
    key = i32,
    value = f32,
    key_cursor = IntCursor,
    value_cursor = FloatCursor,
    key_zero = 0,
    value_zero = 0.0,
    hash_key = BitMixer::mix_phi_i32,
    mix_key = BitMixer::mix_i32,
    mix_value = BitMixer::mix_f32,
    eq_value = super::support::eq_f32_bits,
    add_value = super::support::add_f32,
    size_of_keys = super::support::size_of_int_array,
    size_of_values = super::support::size_of_float_array,
    base_ram_bytes_used = 48,
    java_class = "IntFloatHashMap",
    java_key = "int",
    java_value = "float",
    value_fmt = ":?",
}
