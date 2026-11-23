// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.IterCollationIterator`.

use std::sync::Arc;

use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_iterator::{is_trail_surrogate, CollationIterator, CollationIteratorBase, NO_CP_AND_CE32};
use crate::text::u_character_iterator::UCharacterIterator;

/// UCharIterator-based collation element and character iterator. Handles
/// normalized text, with length or NUL-terminated. Unnormalized text is
/// handled by a subclass.
///
/// Equivalent to `com.ibm.icu.impl.coll.IterCollationIterator`; the
/// iterator owns the `UCharacterIterator` it is given.
#[derive(Debug)]
pub struct IterCollationIterator {
    base: CollationIteratorBase,
    /// `iter`.
    pub(crate) iter: Box<dyn UCharacterIterator>,
}

impl IterCollationIterator {
    /// `IterCollationIterator(CollationData, boolean, UCharacterIterator)`.
    pub fn new(d: Arc<CollationData>, numeric: bool, ui: Box<dyn UCharacterIterator>) -> Self {
        Self { base: CollationIteratorBase::with_numeric(d, numeric), iter: ui }
    }
}

impl CollationIterator for IterCollationIterator {
    fn base(&self) -> &CollationIteratorBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut CollationIteratorBase {
        &mut self.base
    }

    fn reset_to_offset(&mut self, new_offset: i32) {
        self.base.reset();
        self.iter.set_index(new_offset);
    }

    fn get_offset(&self) -> i32 {
        self.iter.get_index()
    }

    fn next_code_point(&mut self) -> i32 {
        self.iter.next_code_point()
    }

    fn previous_code_point(&mut self) -> i32 {
        self.iter.previous_code_point()
    }

    fn handle_next_ce32(&mut self) -> i64 {
        let c = self.iter.next();
        if c < 0 {
            return NO_CP_AND_CE32;
        }
        let trie = self.base.trie.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        CollationIteratorBase::make_code_point_and_ce32_pair(c, trie.get_from_u16_single_lead(c as u16))
    }

    fn handle_get_trail_surrogate(&mut self) -> u16 {
        let trail = self.iter.next();
        if !is_trail_surrogate(trail) && trail >= 0 {
            self.iter.previous();
        }
        trail as u16
    }

    fn forward_num_code_points(&mut self, num: i32) {
        self.iter.move_code_point_index(num);
    }

    fn backward_num_code_points(&mut self, num: i32) {
        self.iter.move_code_point_index(-num);
    }
}
