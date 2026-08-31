//! Port of `org.apache.lucene.internal.hppc.CharHashSet`.

use super::macros::define_hash_set;

define_hash_set! {
    set = CharHashSet,
    key = u16,
    cursor = CharCursor,
    key_zero = 0,
    hash_key = BitMixer::mix_phi_u16,
    mix_key = BitMixer::mix_u16,
    size_of_keys = super::support::size_of_char_array,
    base_ram_bytes_used = 48,
    java_class = "CharHashSet",
    java_key = "char",
}
