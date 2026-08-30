//! Port of `org.apache.lucene.internal.hppc`.
//!
//! Specialised primitive collections, forked by Lucene from
//! [HPPC](https://github.com/carrotsearch/hppc) 0.10.0 to avoid boxing every
//! `int`, `long`, `float`, `double` and `char` that Lucene stores in a map, a
//! set or a list. They are Lucene-internal (`@lucene.internal`): they exist to
//! serve the rest of the library, not as a public collections API.
//!
//! The port keeps every container monomorphic and named exactly as in Lucene —
//! [`IntIntHashMap`], [`LongFloatHashMap`], [`CharHashSet`] and so on — rather
//! than collapsing them into one generic `HashMap<K, V>`, because that
//! specialisation is the whole point of the package. The bodies are generated
//! from the templates in the private `macros` module, which plays the role of
//! the HPPC code generator that produced Lucene's own files.
//!
//! # Port-wide adaptations
//!
//! * **Java `char` is a `u16`.** A Java `char` is a UTF-16 code unit, not a
//!   Unicode scalar value, so [`CharHashSet`], [`CharObjectHashMap`] and
//!   [`CharCursor`] are keyed on [`u16`]. A Rust `char` would reject unpaired
//!   surrogates and would hash differently outside the Basic Multilingual
//!   Plane. As a consequence, `Display` renders such a key as a number rather
//!   than as the character Java's string concatenation would print.
//! * **Unchecked exceptions become panics.**
//!   [`BufferAllocationException`] is a Java `RuntimeException` that no Lucene
//!   caller catches and that no Java signature declares, so the containers
//!   panic with it rather than returning `Result` — which would otherwise
//!   change every constructor, `put` and `add` signature in the package. The
//!   type itself is a real [`std::error::Error`] and converts into
//!   [`LuceneError`](crate::error::LuceneError).
//! * **Java assertions become debug assertions.** Lucene guards its
//!   index-based API and internal invariants with `assert`, which is off by
//!   default in production and on under `-ea`; [`debug_assert!`] has exactly
//!   that behaviour.
//! * **Cursors are yielded by value.** Java reuses one mutable cursor object
//!   for a whole iteration; the ported cursors are [`Copy`] and are returned by
//!   value, which no caller can distinguish except by holding on to a cursor
//!   across a step — which Java forbids anyway.
//! * **Nested classes become sibling items.** Java nests `IntIntCursor`,
//!   `KeysContainer`, `EntryIterator` and friends inside their container; Rust
//!   has no nested types, so each lives at the top of the module named after
//!   its Lucene class, e.g. [`int_int_hash_map::KeysContainer`].
//! * **Iteration order is randomised**, exactly as in Lucene: every container
//!   takes a distinct seed from a process-wide counter and every iterator
//!   advances it, so that no caller can come to depend on a fixed order.

#![deny(unsafe_code)]

pub mod abstract_iterator;
pub mod bit_mixer;
pub mod buffer_allocation_exception;
pub mod char_cursor;
pub mod double_cursor;
pub mod float_cursor;
pub mod hash_containers;
pub mod int_cursor;
pub mod long_cursor;
pub mod object_cursor;

pub mod char_hash_set;
pub mod char_object_hash_map;
pub mod float_array_list;
pub mod int_array_list;
pub mod int_double_hash_map;
pub mod int_float_hash_map;
pub mod int_hash_set;
pub mod int_int_hash_map;
pub mod int_long_hash_map;
pub mod int_object_hash_map;
pub mod long_array_list;
pub mod long_float_hash_map;
pub mod long_hash_set;
pub mod long_int_hash_map;
pub mod long_object_hash_map;
pub mod max_sized_float_array_list;
pub mod max_sized_int_array_list;

mod macros;
mod support;

pub use abstract_iterator::AbstractIterator;
pub use bit_mixer::BitMixer;
pub use buffer_allocation_exception::BufferAllocationException;
pub use char_cursor::CharCursor;
pub use double_cursor::DoubleCursor;
pub use float_cursor::FloatCursor;
pub use int_cursor::IntCursor;
pub use long_cursor::LongCursor;
pub use object_cursor::ObjectCursor;

pub use char_hash_set::CharHashSet;
pub use char_object_hash_map::{CharObjectCursor, CharObjectHashMap};
pub use float_array_list::FloatArrayList;
pub use int_array_list::IntArrayList;
pub use int_double_hash_map::{IntDoubleCursor, IntDoubleHashMap};
pub use int_float_hash_map::{IntFloatCursor, IntFloatHashMap};
pub use int_hash_set::IntHashSet;
pub use int_int_hash_map::{IntIntCursor, IntIntHashMap};
pub use int_long_hash_map::{IntLongCursor, IntLongHashMap};
pub use int_object_hash_map::{IntObjectCursor, IntObjectHashMap};
pub use long_array_list::LongArrayList;
pub use long_float_hash_map::{LongFloatCursor, LongFloatHashMap};
pub use long_hash_set::LongHashSet;
pub use long_int_hash_map::{LongIntCursor, LongIntHashMap};
pub use long_object_hash_map::{LongObjectCursor, LongObjectHashMap};
pub use max_sized_float_array_list::MaxSizedFloatArrayList;
pub use max_sized_int_array_list::MaxSizedIntArrayList;
