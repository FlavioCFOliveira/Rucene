//! `FieldUpdatesBuffer` ported from `org.apache.lucene.index`.
//!
//! Buffers a run of doc-values updates compactly, so the delete queue can carry
//! many of them without a per-update object.

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue};
use crate::index::Term;
use crate::util::bytes_ref_array::{
    BytesRefArrayIterator, IndexedBytesRefIterator, SortState, SortableBytesRefArray,
};
use crate::util::sorter::{NaturalBytesRefComparator, StringSorterComparator};
use crate::util::{
    Accountable, ArrayUtil, BytesRef, BytesRefArray, BytesRefIterator, Counter, FixedBitSet,
    RamUsageEstimator,
};

/// `RamUsageEstimator.shallowSizeOfInstance(FieldUpdatesBuffer.class)`: an
/// object header, eight references, an `int`, two `long`s and two `boolean`s,
/// aligned — 72 bytes with compressed references, measured against
/// `lucene-core-10.5.0.jar`.
///
/// Equivalent to `FieldUpdatesBuffer.SELF_SHALLOW_SIZE`
/// (`FieldUpdatesBuffer.java:46-47`).
const SELF_SHALLOW_SIZE: i64 = RamUsageEstimator::align_object_size(
    RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
        + 8 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
        + 4
        + 2 * 8
        + 2,
);

/// `RamUsageEstimator.shallowSizeOfInstance(String.class)`: an object header,
/// the `value` reference, the `int hash`, the `byte coder` and the `boolean
/// hashIsZero`, aligned — 24 bytes, measured against the JDK `lucene-core`
/// runs on.
///
/// Equivalent to `FieldUpdatesBuffer.STRING_SHALLOW_SIZE`
/// (`FieldUpdatesBuffer.java:48-49`).
const STRING_SHALLOW_SIZE: i64 = 24;

/// A run of updates to one kind of doc-values field.
///
/// Equivalent to `org.apache.lucene.index.FieldUpdatesBuffer`
/// (`index/FieldUpdatesBuffer.java:43-410`): the update terms are kept in a
/// `BytesRefArray`, the binary values in another, and the term fields, the
/// `docUpTo` limits and the numeric values in parallel arrays that stay a
/// single slot long for as long as every update agrees with the first one, so
/// a long run of updates to the same field, applied to every document, costs
/// almost nothing. A `FixedBitSet` records which updates carry a value; it
/// only exists once one does not.
///
/// The RAM the buffer occupies is charged to its own [`Counter`], which the
/// term and value arrays share, exactly as Java charges the
/// `BufferedUpdates.fieldUpdatesBytesUsed` counter it is given;
/// [`ram_bytes_used`](Accountable::ram_bytes_used) reads it back, and
/// `BufferedUpdates` adds the deltas to its own total.
///
/// **Divergence from Lucene 10.5.0.** Java charges the term field of an update
/// only when the `String` is not the very object the first update used
/// (`field != fields[0]`, a reference comparison). Rust has no object identity
/// for strings, so the port compares the names: an equal name held by a
/// different Java `String` object is charged by Lucene and not by this port.
/// Field names built from literals are interned, and so the same object, in
/// Java, which is the case the comparison is for.
#[derive(Debug)]
pub struct FieldUpdatesBuffer {
    bytes_used: Arc<Counter>,
    num_updates: i32,
    // we use a very simple approach and store the update term values without
    // de-duplication which is also not a common case to keep updating the same
    // value more than once... we might pay a higher price in terms of memory in
    // certain cases but will gain on CPU for those. We also use a stable sort
    // to sort in order to apply the terms in order since by definition we
    // store them in order.
    term_values: BytesRefArray,
    term_sort_state: Option<SortState>,
    /// `None` if we are buffering numerics.
    byte_values: Option<BytesRefArray>,
    docs_up_to: Vec<i32>,
    /// `None` if we are buffering binaries.
    numeric_values: Option<Vec<i64>>,
    has_values: Option<FixedBitSet>,
    max_numeric: i64,
    min_numeric: i64,
    /// Java's `String[] fields`; a slot past the last update is `None`, as
    /// Java's `ArrayUtil.grow` leaves it `null`.
    fields: Vec<Option<String>>,
    is_numeric: bool,
    finished: bool,
}

impl FieldUpdatesBuffer {
    /// Creates a buffer seeded with `initial_value`, the first update.
    ///
    /// Equivalent to the two public constructors
    /// `FieldUpdatesBuffer(Counter, NumericDocValuesUpdate, int)` and
    /// `FieldUpdatesBuffer(Counter, BinaryDocValuesUpdate, int)`
    /// (`FieldUpdatesBuffer.java:82-107`) over the private common one
    /// (`FieldUpdatesBuffer.java:63-78`). The kind of the update decides which:
    /// an update without a value is numeric unless its doc-values type says
    /// binary.
    ///
    /// # Errors
    ///
    /// Propagates whatever appending the term or the value raises.
    pub fn new(initial_value: DocValuesUpdate, doc_up_to: i32) -> Result<Self> {
        let is_numeric = match &initial_value.value {
            DocValuesUpdateValue::Numeric(_) => true,
            DocValuesUpdateValue::Binary(_) => false,
            DocValuesUpdateValue::None => {
                initial_value.doc_values_type != crate::index::DocValuesType::BINARY
            }
        };
        let has_value = !matches!(initial_value.value, DocValuesUpdateValue::None);
        let bytes_used = Arc::new(Counter::new_counter());
        bytes_used.add_and_get(SELF_SHALLOW_SIZE);
        let mut term_values = BytesRefArray::new(Arc::clone(&bytes_used));
        term_values.append(initial_value.term.bytes())?;
        // `fields = new String[] {initialValue.term.field}`
        // (`FieldUpdatesBuffer.java:73`): the field a buffered update records
        // is the field its *term* is looked up in, not the doc-values field it
        // updates.
        let fields = vec![Some(initial_value.term.field().to_string())];
        bytes_used.add_and_get(size_of_string(initial_value.term.field()));
        let docs_up_to = vec![doc_up_to];
        let has_values = if has_value {
            None
        } else {
            let has_values = FixedBitSet::new(1);
            bytes_used.add_and_get(has_values.ram_bytes_used());
            Some(has_values)
        };
        let byte_values = if is_numeric {
            None
        } else {
            Some(BytesRefArray::new(Arc::clone(&bytes_used)))
        };
        let mut buffer = Self {
            bytes_used,
            num_updates: 1,
            term_values,
            term_sort_state: None,
            byte_values,
            docs_up_to,
            numeric_values: None,
            has_values,
            max_numeric: i64::MIN,
            min_numeric: i64::MAX,
            fields,
            is_numeric,
            finished: false,
        };
        if is_numeric {
            match initial_value.value {
                DocValuesUpdateValue::Numeric(value) => {
                    buffer.numeric_values = Some(vec![value]);
                    buffer.max_numeric = value;
                    buffer.min_numeric = value;
                }
                _ => buffer.numeric_values = Some(vec![0]),
            }
            buffer.bytes_used.add_and_get(8);
        } else if let DocValuesUpdateValue::Binary(value) = &initial_value.value {
            buffer
                .byte_values
                .as_mut()
                .expect("INVARIANT: a binary buffer has byte values")
                .append(value)?;
        }
        Ok(buffer)
    }

    /// Returns the largest numeric value buffered, or `None` when no update
    /// carried one.
    ///
    /// Equivalent to `FieldUpdatesBuffer.getMaxNumeric()`
    /// (`FieldUpdatesBuffer.java:109-115`), which answers `0` when no update
    /// carried a value; `None` says that in the type, and the caller that needs
    /// Java's literal answer writes `unwrap_or(0)`.
    pub fn max_numeric_value(&self) -> Option<i64> {
        (!(self.min_numeric == i64::MAX && self.max_numeric == i64::MIN))
            .then_some(self.max_numeric)
    }

    /// Returns the smallest numeric value buffered, or `None` when no update
    /// carried one.
    ///
    /// Equivalent to `FieldUpdatesBuffer.getMinNumeric()`
    /// (`FieldUpdatesBuffer.java:117-123`); see
    /// [`max_numeric_value`](Self::max_numeric_value) for `None`.
    pub fn min_numeric_value(&self) -> Option<i64> {
        (!(self.min_numeric == i64::MAX && self.max_numeric == i64::MIN))
            .then_some(self.min_numeric)
    }

    fn check_not_finished(&self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "buffer was finished already".to_string(),
            ));
        }
        Ok(())
    }

    /// Records the field, the limit and whether a value is present for the
    /// update at `ord`, growing a parallel array only when the update differs
    /// from the first one.
    ///
    /// Equivalent to the package-private `FieldUpdatesBuffer.add(String, int,
    /// int, boolean)` (`FieldUpdatesBuffer.java:125-169`).
    fn add(&mut self, field: &str, doc_up_to: i32, ord: i32, has_value: bool) {
        debug_assert!(!self.finished, "buffer was finished already");
        let ord_index = ord as usize;
        if self.fields[0].as_deref() != Some(field) || self.fields.len() != 1 {
            if self.fields.len() <= ord_index {
                let previous_length = self.fields.len();
                let new_length = ArrayUtil::oversize(
                    ord_index + 1,
                    RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
                );
                self.fields.resize(new_length, None);
                if previous_length == 1 {
                    let first = self.fields[0].clone();
                    for slot in &mut self.fields[1..ord_index] {
                        *slot = first.clone();
                    }
                }
                self.bytes_used.add_and_get(
                    (new_length - previous_length) as i64 * RamUsageEstimator::NUM_BYTES_OBJECT_REF,
                );
            }
            // that's an easy win of not accounting if there is an outlier
            if self.fields[0].as_deref() != Some(field) {
                self.bytes_used.add_and_get(size_of_string(field));
            }
            self.fields[ord_index] = Some(field.to_string());
        }
        if self.docs_up_to[0] != doc_up_to || self.docs_up_to.len() != 1 {
            if self.docs_up_to.len() <= ord_index {
                let previous_length = self.docs_up_to.len();
                let new_length = ArrayUtil::oversize(ord_index + 1, 4);
                let first = self.docs_up_to[0];
                self.docs_up_to.resize(new_length, 0);
                if previous_length == 1 {
                    self.docs_up_to[1..ord_index].fill(first);
                }
                self.bytes_used
                    .add_and_get((new_length - previous_length) as i64 * 4);
            }
            self.docs_up_to[ord_index] = doc_up_to;
        }
        if !has_value || self.has_values.is_some() {
            match self.has_values.take() {
                None => {
                    let mut has_values = FixedBitSet::new(ord_index + 1);
                    has_values.set_range(0, ord_index);
                    self.bytes_used.add_and_get(has_values.ram_bytes_used());
                    self.has_values = Some(has_values);
                }
                Some(has_values) if has_values.length() <= ord_index => {
                    let old_bytes = has_values.ram_bytes_used();
                    let grown = FixedBitSet::ensure_capacity(
                        has_values,
                        ArrayUtil::oversize(ord_index + 1, 1),
                    );
                    self.bytes_used
                        .add_and_get(grown.ram_bytes_used() - old_bytes);
                    self.has_values = Some(grown);
                }
                Some(has_values) => self.has_values = Some(has_values),
            }
            if has_value {
                self.has_values
                    .as_mut()
                    .expect("INVARIANT: set just above")
                    .set(ord_index);
            }
        }
    }

    /// Buffers a numeric update.
    ///
    /// Equivalent to `FieldUpdatesBuffer.addUpdate(Term, long, int)`
    /// (`FieldUpdatesBuffer.java:171-189`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] for a binary buffer (Java asserts
    /// `isNumeric`) or a finished one, and propagates whatever appending the
    /// term raises.
    pub fn add_numeric_update(&mut self, term: &Term, value: i64, doc_up_to: i32) -> Result<()> {
        if !self.is_numeric {
            return Err(LuceneError::IllegalState(
                "cannot add a numeric update to a binary FieldUpdatesBuffer".to_string(),
            ));
        }
        self.check_not_finished()?;
        let ord = self.append(term)?;
        self.add(term.field(), doc_up_to, ord, true);
        self.min_numeric = self.min_numeric.min(value);
        self.max_numeric = self.max_numeric.max(value);
        let ord_index = ord as usize;
        let numeric_values = self
            .numeric_values
            .as_mut()
            .expect("INVARIANT: a numeric buffer has numeric values");
        if numeric_values[0] != value || numeric_values.len() != 1 {
            if numeric_values.len() <= ord_index {
                let previous_length = numeric_values.len();
                let new_length = ArrayUtil::oversize(ord_index + 1, 8);
                let first = numeric_values[0];
                numeric_values.resize(new_length, 0);
                if previous_length == 1 {
                    numeric_values[1..ord_index].fill(first);
                }
                self.bytes_used
                    .add_and_get((new_length - previous_length) as i64 * 8);
            }
            numeric_values[ord_index] = value;
        }
        Ok(())
    }

    /// Buffers an update that clears the field.
    ///
    /// Equivalent to `FieldUpdatesBuffer.addNoValue(Term, int)`
    /// (`FieldUpdatesBuffer.java:191-194`), which records `term.field`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] for a finished buffer, and
    /// propagates whatever appending the term raises.
    pub fn add_no_value(&mut self, term: &Term, doc_up_to: i32) -> Result<()> {
        self.check_not_finished()?;
        let ord = self.append(term)?;
        self.add(term.field(), doc_up_to, ord, false);
        Ok(())
    }

    /// Buffers a binary update.
    ///
    /// Equivalent to `FieldUpdatesBuffer.addUpdate(Term, BytesRef, int)`
    /// (`FieldUpdatesBuffer.java:196-201`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] for a numeric buffer (Java asserts
    /// `isNumeric == false`) or a finished one, and propagates whatever
    /// appending the term or the value raises.
    pub fn add_binary_update(
        &mut self,
        term: &Term,
        value: BytesRef,
        doc_up_to: i32,
    ) -> Result<()> {
        if self.is_numeric {
            return Err(LuceneError::IllegalState(
                "cannot add a binary update to a numeric FieldUpdatesBuffer".to_string(),
            ));
        }
        self.check_not_finished()?;
        let ord = self.append(term)?;
        self.byte_values
            .as_mut()
            .expect("INVARIANT: a binary buffer has byte values")
            .append(&value)?;
        self.add(term.field(), doc_up_to, ord, true);
        Ok(())
    }

    /// Appends the update's term and returns its ordinal.
    ///
    /// Equivalent to the private `FieldUpdatesBuffer.append(Term)`
    /// (`FieldUpdatesBuffer.java:203-206`).
    fn append(&mut self, term: &Term) -> Result<i32> {
        self.term_values.append(term.bytes())?;
        let ord = self.num_updates;
        self.num_updates += 1;
        Ok(ord)
    }

    /// Seals the buffer. When every update sets the same numeric value on
    /// documents selected by terms of one field, the terms are sorted (stably)
    /// so that they can be applied in term order.
    ///
    /// Equivalent to `FieldUpdatesBuffer.finish()` (`FieldUpdatesBuffer.java:208-219`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the buffer was finished
    /// already, with Java's message.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "buffer was finished already".to_string(),
            ));
        }
        self.finished = true;
        let sorted_terms =
            self.has_single_value() && self.has_values.is_none() && self.fields.len() == 1;
        if sorted_terms {
            let comparator = NaturalBytesRefComparator;
            let sort_state = self
                .term_values
                .sort(StringSorterComparator::Radix(&comparator), true);
            self.bytes_used.add_and_get(sort_state.ram_bytes_used());
            self.term_sort_state = Some(sort_state);
            debug_assert!(self.assert_term_and_doc_in_order());
        }
        Ok(())
    }

    /// Equivalent to `FieldUpdatesBuffer.assertTermAndDocInOrder()`
    /// (`FieldUpdatesBuffer.java:221-246`).
    fn assert_term_and_doc_in_order(&self) -> bool {
        let mut iterator = self
            .term_values
            .iterator_with_sort_state(self.term_sort_state.clone());
        let mut last: Option<BytesRef> = None;
        let mut last_ord = 0usize;
        while let Ok(Some(current)) = iterator.next() {
            if let Some(last) = last.as_ref() {
                let cmp = current.slice().cmp(last.slice());
                assert!(cmp != std::cmp::Ordering::Less, "term in reverse order");
                assert!(
                    cmp != std::cmp::Ordering::Equal
                        || self.docs_up_to[get_array_index(self.docs_up_to.len(), last_ord)]
                            <= self.docs_up_to
                                [get_array_index(self.docs_up_to.len(), iterator.ord())],
                    "doc id in reverse order"
                );
            }
            last = Some(current);
            last_ord = iterator.ord();
        }
        true
    }

    /// Returns an iterator over the buffered updates.
    ///
    /// Equivalent to `FieldUpdatesBuffer.iterator()` (`FieldUpdatesBuffer.java:248-253`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the buffer is not finished
    /// yet, with Java's message.
    pub fn iterator(&self) -> Result<BufferedUpdateIterator<'_>> {
        if !self.finished {
            return Err(LuceneError::IllegalState(
                "buffer is not finished yet".to_string(),
            ));
        }
        Ok(BufferedUpdateIterator::new(self))
    }

    /// Returns whether the buffer holds numeric updates.
    ///
    /// Equivalent to `FieldUpdatesBuffer.isNumeric()`.
    pub fn is_numeric(&self) -> bool {
        debug_assert!(self.is_numeric || self.byte_values.is_some());
        self.is_numeric
    }

    /// Returns how many updates are buffered.
    ///
    /// Equivalent to `FieldUpdatesBuffer.numUpdates`.
    pub fn num_updates(&self) -> usize {
        self.num_updates as usize
    }

    /// Returns whether every numeric update carries the same value.
    ///
    /// Equivalent to `FieldUpdatesBuffer.hasSingleValue()`
    /// (`FieldUpdatesBuffer.java:260-263`): `isNumeric && numericValues.length
    /// == 1`. We only do this optimization for numerics so far.
    pub fn has_single_value(&self) -> bool {
        self.is_numeric
            && self
                .numeric_values
                .as_ref()
                .is_some_and(|values| values.len() == 1)
    }

    /// Returns the numeric value of the update at `index`, or `0` when it
    /// carries none.
    ///
    /// Equivalent to `FieldUpdatesBuffer.getNumericValue(int)`
    /// (`FieldUpdatesBuffer.java:265-270`). A binary buffer answers `0`, where
    /// Java would dereference its null `numericValues`.
    pub fn get_numeric_value(&self, index: usize) -> i64 {
        if let Some(has_values) = self.has_values.as_ref() {
            if !has_values.get(index) {
                return 0;
            }
        }
        match self.numeric_values.as_ref() {
            Some(values) => values[get_array_index(values.len(), index)],
            None => 0,
        }
    }
}

/// `STRING_SHALLOW_SIZE + string.length() * Character.BYTES`, the heap a Java
/// `String` of `string`'s UTF-16 length is charged.
///
/// Equivalent to the private `FieldUpdatesBuffer.sizeOfString(String)`
/// (`FieldUpdatesBuffer.java:80-82`).
fn size_of_string(string: &str) -> i64 {
    STRING_SHALLOW_SIZE + string.encode_utf16().count() as i64 * 2
}

/// `Math.min(arrayLength - 1, index)`: a one-slot parallel array answers for
/// every update.
///
/// Equivalent to the private static `FieldUpdatesBuffer.getArrayIndex(int, int)`
/// (`FieldUpdatesBuffer.java:405-409`).
fn get_array_index(array_length: usize, index: usize) -> usize {
    debug_assert!(
        array_length == 1 || array_length > index,
        "illegal array index length: {array_length} index: {index}"
    );
    std::cmp::min(array_length - 1, index)
}

impl Accountable for FieldUpdatesBuffer {
    /// What the buffer has charged to its counter.
    ///
    /// Java has no `ramBytesUsed()` on the buffer: it charges the
    /// `BufferedUpdates.fieldUpdatesBytesUsed` counter as it goes. The counter
    /// here belongs to the buffer, so this reads the same total back.
    fn ram_bytes_used(&self) -> i64 {
        self.bytes_used.get()
    }
}

/// One update, as [`BufferedUpdateIterator::next`] hands it out.
///
/// Equivalent to `FieldUpdatesBuffer.BufferedUpdate`
/// (`FieldUpdatesBuffer.java:273-303`), whose fields Java overwrites in place on
/// every `next()`; this port returns a fresh value.
#[derive(Debug, Clone)]
pub struct BufferedUpdate {
    /// The max document ID this update should be applied to.
    pub doc_up_to: i32,
    /// A numeric value, or `0` if this buffer holds binary updates.
    pub numeric_value: i64,
    /// A binary value, or `None` if this buffer holds numeric updates.
    pub binary_value: Option<BytesRef>,
    /// `true` if this update has a value.
    pub has_value: bool,
    /// The update term's field.
    pub term_field: String,
    /// The update term's value.
    pub term_value: BytesRef,
}

/// An iterator over all updates of a buffer, in insertion order — or, when the
/// buffer sorted its terms, in term order with repeated terms collapsed.
///
/// Equivalent to `FieldUpdatesBuffer.BufferedUpdateIterator`
/// (`FieldUpdatesBuffer.java:305-403`).
pub struct BufferedUpdateIterator<'a> {
    buffer: &'a FieldUpdatesBuffer,
    term_values_iterator: BytesRefArrayIterator<'a>,
    look_ahead_term_iterator: Option<BytesRefArrayIterator<'a>>,
    byte_values_iterator: Option<BytesRefArrayIterator<'a>>,
    /// Whether `next()` has returned an update yet: Java tests
    /// `bufferedUpdate.termValue == null` for the same thing.
    started: bool,
}

impl<'a> BufferedUpdateIterator<'a> {
    fn new(buffer: &'a FieldUpdatesBuffer) -> Self {
        let term_values_iterator = buffer
            .term_values
            .iterator_with_sort_state(buffer.term_sort_state.clone());
        let look_ahead_term_iterator = buffer.term_sort_state.as_ref().map(|sort_state| {
            buffer
                .term_values
                .iterator_with_sort_state(Some(sort_state.clone()))
        });
        let byte_values_iterator = buffer.byte_values.as_ref().map(BytesRefArray::iterator);
        Self {
            buffer,
            term_values_iterator,
            look_ahead_term_iterator,
            byte_values_iterator,
            started: false,
        }
    }

    /// If all updates update a single field to the same value, then we can
    /// apply these updates in the term order instead of the request order as
    /// both will yield the same result. This optimization allows us to iterate
    /// the term dictionary faster and de-duplicate updates.
    ///
    /// Equivalent to `BufferedUpdateIterator.isSortedTerms()`.
    pub fn is_sorted_terms(&self) -> bool {
        self.buffer.term_sort_state.is_some()
    }

    /// Moves to the next update, or returns `None` once every update has been
    /// consumed.
    ///
    /// Equivalent to `BufferedUpdateIterator.next()` (`FieldUpdatesBuffer.java:337-366`).
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the term or value arrays raises.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<BufferedUpdate>> {
        let Some(next) = self.next_term()? else {
            return Ok(None);
        };
        self.started = true;
        let buffer = self.buffer;
        let idx = self.term_values_iterator.ord();
        let has_value = buffer
            .has_values
            .as_ref()
            .is_none_or(|has_values| has_values.get(idx));
        let term_field = buffer.fields[get_array_index(buffer.fields.len(), idx)]
            .clone()
            .expect("INVARIANT: every slot up to the last update holds a field");
        let doc_up_to = buffer.docs_up_to[get_array_index(buffer.docs_up_to.len(), idx)];
        let (numeric_value, binary_value) = if has_value {
            if buffer.is_numeric {
                let values = buffer
                    .numeric_values
                    .as_ref()
                    .expect("INVARIANT: a numeric buffer has numeric values");
                (values[get_array_index(values.len(), idx)], None)
            } else {
                let value = self
                    .byte_values_iterator
                    .as_mut()
                    .expect("INVARIANT: a binary buffer has byte values")
                    .next()?;
                (0, value)
            }
        } else {
            (0, None)
        };
        Ok(Some(BufferedUpdate {
            doc_up_to,
            numeric_value,
            binary_value,
            has_value,
            term_field,
            term_value: next,
        }))
    }

    /// Equivalent to the private `BufferedUpdateIterator.nextTerm()`
    /// (`FieldUpdatesBuffer.java:368-389`).
    fn next_term(&mut self) -> Result<Option<BytesRef>> {
        let Some(look_ahead) = self.look_ahead_term_iterator.as_mut() else {
            return self.term_values_iterator.next();
        };
        if !self.started {
            look_ahead.next()?;
        }
        loop {
            let ahead_term = look_ahead.next()?;
            let last_term = self.term_values_iterator.next()?;
            // Shortcut to avoid equals, we did a stable sort before, so
            // aheadTerm can only equal lastTerm when aheadTerm has a larger ord.
            let repeat = match (&ahead_term, &last_term) {
                (Some(ahead), Some(last)) => {
                    look_ahead.ord() > self.term_values_iterator.ord()
                        && ahead.slice() == last.slice()
                }
                _ => false,
            };
            if !repeat {
                return Ok(last_term);
            }
        }
    }
}

impl std::fmt::Debug for BufferedUpdateIterator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferedUpdateIterator")
            .field("is_sorted_terms", &self.is_sorted_terms())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::DocValuesType;

    fn term(field: &str, value: &str) -> Term {
        Term::new(field, BytesRef::new(value.as_bytes().to_vec()))
    }

    fn numeric(term: Term, value: Option<i64>, doc_up_to: i32) -> DocValuesUpdate {
        DocValuesUpdate {
            doc_values_type: DocValuesType::NUMERIC,
            term,
            field: "price".to_string(),
            doc_id_up_to: doc_up_to,
            value: value.map_or(DocValuesUpdateValue::None, DocValuesUpdateValue::Numeric),
        }
    }

    fn drain(buffer: &FieldUpdatesBuffer) -> Vec<BufferedUpdate> {
        let mut iterator = buffer.iterator().unwrap();
        let mut out = Vec::new();
        while let Some(update) = iterator.next().unwrap() {
            out.push(update);
        }
        out
    }

    #[test]
    fn a_single_valued_single_field_buffer_iterates_its_terms_sorted_and_collapsed() {
        let mut buffer =
            FieldUpdatesBuffer::new(numeric(term("id", "c"), Some(7), 10), 10).unwrap();
        buffer.add_numeric_update(&term("id", "a"), 7, 10).unwrap();
        buffer.add_numeric_update(&term("id", "c"), 7, 10).unwrap();
        buffer.add_numeric_update(&term("id", "b"), 7, 10).unwrap();
        buffer.finish().unwrap();
        let iterator = buffer.iterator().unwrap();
        assert!(iterator.is_sorted_terms());
        let terms: Vec<Vec<u8>> = drain(&buffer)
            .into_iter()
            .map(|u| u.term_value.bytes)
            .collect();
        assert_eq!(terms, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn a_mixed_buffer_iterates_in_insertion_order_with_its_parallel_arrays() {
        let mut buffer = FieldUpdatesBuffer::new(numeric(term("id", "c"), Some(1), 3), 3).unwrap();
        buffer
            .add_numeric_update(&term("group", "a"), 2, 5)
            .unwrap();
        buffer.add_no_value(&term("id", "b"), 6).unwrap();
        buffer.add_numeric_update(&term("id", "d"), 1, 3).unwrap();
        buffer.finish().unwrap();
        assert!(!buffer.iterator().unwrap().is_sorted_terms());
        assert!(!buffer.has_single_value());
        let updates = drain(&buffer);
        let seen: Vec<(String, Vec<u8>, i32, bool, i64)> = updates
            .into_iter()
            .map(|u| {
                (
                    u.term_field,
                    u.term_value.bytes,
                    u.doc_up_to,
                    u.has_value,
                    u.numeric_value,
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                ("id".to_string(), b"c".to_vec(), 3, true, 1),
                ("group".to_string(), b"a".to_vec(), 5, true, 2),
                ("id".to_string(), b"b".to_vec(), 6, false, 0),
                ("id".to_string(), b"d".to_vec(), 3, true, 1),
            ]
        );
        assert_eq!(buffer.min_numeric_value(), Some(1));
        assert_eq!(buffer.max_numeric_value(), Some(2));
        assert_eq!(buffer.get_numeric_value(2), 0);
        assert_eq!(buffer.get_numeric_value(3), 1);
    }

    #[test]
    fn the_iterator_and_finish_enforce_the_finished_state() {
        let mut buffer = FieldUpdatesBuffer::new(numeric(term("id", "a"), None, 1), 1).unwrap();
        assert!(buffer.iterator().is_err(), "buffer is not finished yet");
        buffer.finish().unwrap();
        assert!(buffer.finish().is_err(), "buffer was finished already");
        assert!(buffer.add_numeric_update(&term("id", "b"), 1, 1).is_err());
    }

    #[test]
    fn the_charge_follows_javas_arithmetic_for_a_constant_run() {
        // SELF_SHALLOW_SIZE + BytesRefArray(header 16*4 + one 32 KiB block) +
        // sizeOfString("id") + Long.BYTES, then nothing per repeated update but
        // the term bytes, which live in the already-charged block.
        let mut buffer = FieldUpdatesBuffer::new(numeric(term("id", "a"), Some(5), 9), 9).unwrap();
        let initial = buffer.ram_bytes_used();
        assert_eq!(
            initial,
            SELF_SHALLOW_SIZE
                + RamUsageEstimator::NUM_BYTES_ARRAY_HEADER * 4
                + crate::util::byte_block_pool::BYTE_BLOCK_SIZE as i64
                + size_of_string("id")
                + 8
        );
        for i in 0..100 {
            buffer
                .add_numeric_update(&term("id", &format!("t{i}")), 5, 9)
                .unwrap();
        }
        // The term array grows its offsets; the parallel arrays stay one slot.
        assert_eq!(buffer.docs_up_to.len(), 1);
        assert_eq!(buffer.fields.len(), 1);
        assert_eq!(buffer.numeric_values.as_ref().unwrap().len(), 1);
        assert!(buffer.has_values.is_none());
        assert!(buffer.ram_bytes_used() > initial);
    }
}
