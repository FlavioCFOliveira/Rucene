//! Port of `org.apache.lucene.internal.hppc.FloatArrayList`.

use super::macros::define_array_list;

define_array_list! {
    list = FloatArrayList,
    element = f32,
    cursor = FloatCursor,
    zero = 0.0,
    bytes_per_element = 4,
    mix = super::bit_mixer::BitMixer::mix_f32,
    eq = super::support::eq_f32,
    sort = super::support::sort_f32,
    size_of_elements = super::support::size_of_float_array,
    base_ram_bytes_used = 24,
    java_class = "FloatArrayList",
    java_element = "float",
    element_fmt = ":?",
}

// `FloatArrayList` deliberately has no `Eq` implementation and no `stream()`.
//
// Lucene's `equalElements` compares `float` values with `==`, which is not an
// equivalence relation (`NaN != NaN`), so only `PartialEq` can be honoured.
// Note that its `hashCode` nevertheless mixes `Float.floatToIntBits`, so `-0.0`
// and `0.0` compare equal while hashing differently -- the port reproduces both
// halves of that Lucene behaviour rather than reconciling them.
//
// `stream()` is absent because Java has no `FloatStream`, and `IntArrayList`
// and `LongArrayList` only have the method because `IntStream` and `LongStream`
// exist.
