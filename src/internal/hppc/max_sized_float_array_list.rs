//! Port of `org.apache.lucene.internal.hppc.MaxSizedFloatArrayList`.

use super::macros::define_max_sized_array_list;

define_max_sized_array_list! {
    list = MaxSizedFloatArrayList,
    base = FloatArrayList,
    element = f32,
    cursor = FloatCursor,
    bytes_per_element = 4,
    mix = super::bit_mixer::BitMixer::mix_f32,
    size_of_elements = super::support::size_of_float_array,
    base_ram_bytes_used = 24,
    java_class = "MaxSizedFloatArrayList",
    java_base = "FloatArrayList",
    element_fmt = ":?",
}
