// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CompoundTransliterator`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::{runtime_exception, to_rust_string};
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    transliterator_boilerplate, Position, Transliterator, TransliteratorBase, FORWARD, ID_DELIM,
    REVERSE,
};
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_set::UnicodeSet;

/// A transliterator that is composed of two or more other transliterator
/// objects linked together. For example, if one transliterator transliterates
/// from script A to script B, and another transliterates from script B to
/// script C, the two may be combined to form a new transliterator from A to
/// C.
///
/// Composed transliterators may not behave as expected. For example,
/// inverses may not combine to form the identity transliterator.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.CompoundTransliterator`.
#[derive(Debug)]
pub struct CompoundTransliterator {
    base: TransliteratorBase,
    /// The child transliterators (`Transliterator[] trans`), shared by
    /// `safeClone`.
    trans: Arc<Vec<Arc<dyn Transliterator>>>,
    num_anonymous_rbts: i32,
}

impl CompoundTransliterator {
    /// `CompoundTransliterator(List<Transliterator> list)`: package private
    /// constructor for Transliterator from a vector of transliterators. The
    /// caller is responsible for fixing up the ID.
    ///
    /// # Errors
    ///
    /// None in practice (`setMaximumContextLength` of a non-negative
    /// length).
    pub(crate) fn new(list: Vec<Arc<dyn Transliterator>>) -> Result<Self> {
        Self::with_anonymous_rbts(list, 0)
    }

    /// `CompoundTransliterator(List<Transliterator> list, int
    /// numAnonymousRBTs)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub(crate) fn with_anonymous_rbts(
        list: Vec<Arc<dyn Transliterator>>,
        num_anonymous_rbts: i32,
    ) -> Result<Self> {
        let mut this = Self {
            base: TransliteratorBase::with_id(Vec::new()),
            trans: Arc::new(Vec::new()),
            num_anonymous_rbts: 0,
        };
        this.init(list, FORWARD, false)?;
        this.num_anonymous_rbts = num_anonymous_rbts;
        Ok(this)
    }

    /// `CompoundTransliterator(String id, UnicodeFilter filter2,
    /// Transliterator[] trans2, int numAnonymousRBTs2)`: internal
    /// constructor for `safeClone`, sharing the children.
    fn with_children(
        id: Vec<u16>,
        filter2: Option<&dyn UnicodeFilter>,
        trans2: Arc<Vec<Arc<dyn Transliterator>>>,
        num_anonymous_rbts2: i32,
    ) -> Result<Self> {
        Ok(Self {
            base: TransliteratorBase::new(id, filter2)?,
            trans: trans2,
            num_anonymous_rbts: num_anonymous_rbts2,
        })
    }

    /// `init(List<Transliterator> list, int direction, boolean
    /// fixReverseID)`: finish constructing a transliterator: only to be
    /// called by constructors. Before calling init(), set trans and filter
    /// to null.
    ///
    /// `direction` is either `FORWARD` or `REVERSE`; `fix_reverse_id`, if
    /// true, then reconstitute the ID of a reverse compound transliterator
    /// from the IDs of its elements, in reverse order.
    fn init(
        &mut self,
        list: Vec<Arc<dyn Transliterator>>,
        direction: i32,
        fix_reverse_id: bool,
    ) -> Result<()> {
        // assert(trans == 0);

        // Allocate array
        let count = list.len();
        let mut trans: Vec<Arc<dyn Transliterator>> = Vec::with_capacity(count);

        // Move the transliterators from the vector into an array.
        // Reverse the order if necessary.
        for i in 0..count {
            let j = if direction == FORWARD {
                i
            } else {
                count - 1 - i
            };
            trans.push(list[j].clone());
        }
        self.trans = Arc::new(trans);

        // If the direction is UTRANS_REVERSE then we may need to fix the
        // ID.
        if direction == REVERSE && fix_reverse_id {
            let mut new_id = Vec::new();
            for (i, t) in self.trans.iter().enumerate() {
                if i > 0 {
                    new_id.push(ID_DELIM);
                }
                new_id.extend_from_slice(&t.get_id());
            }
            self.base.set_id(new_id);
        }

        self.compute_maximum_context_length()
    }

    /// `getCount()`: return the number of transliterators in this chain.
    pub fn get_count(&self) -> i32 {
        self.trans.len() as i32
    }

    /// `getTransliterator(int index)`: returns the transliterator at the
    /// given index in this chain.
    ///
    /// # Panics
    ///
    /// Java's `ArrayIndexOutOfBoundsException` for an index outside the
    /// chain.
    pub fn get_transliterator(&self, index: i32) -> Arc<dyn Transliterator> {
        self.trans[index as usize].clone()
    }

    /// `_smartAppend(StringBuilder buf, char c)`: append `c` to `buf`,
    /// unless buf is empty or buf already ends in `c`.
    fn smart_append(buf: &mut Vec<u16>, c: u16) {
        if !buf.is_empty() && buf[buf.len() - 1] != c {
            buf.push(c);
        }
    }

    /// `computeMaximumContextLength()`: compute and set the length of the
    /// longest context required by this transliterator. This is preceding
    /// context.
    fn compute_maximum_context_length(&self) -> Result<()> {
        let mut max = 0;
        for t in self.trans.iter() {
            let len = t.get_maximum_context_length();
            if len > max {
                max = len;
            }
        }
        self.base.set_maximum_context_length(max)
    }

    /// `safeClone()`: temporary hack for registry problem. Needs to be
    /// replaced by better architecture.
    ///
    /// # Errors
    ///
    /// None in practice (the filter is a `UnicodeSet`).
    pub fn safe_clone(&self) -> Result<Arc<dyn Transliterator>> {
        let filter = self.base.get_filter().map(|f| UnicodeSet::from_set(&f));
        Ok(Arc::new(CompoundTransliterator::with_children(
            self.base.get_id(),
            filter.as_ref().map(|f| f as &dyn UnicodeFilter),
            self.trans.clone(),
            self.num_anonymous_rbts,
        )?))
    }
}

/// `String.startsWith("%Pass")`.
fn starts_with_pass(id: &[u16]) -> bool {
    id.starts_with(&[
        b'%' as u16,
        b'P' as u16,
        b'a' as u16,
        b's' as u16,
        b's' as u16,
    ])
}

impl Transliterator for CompoundTransliterator {
    transliterator_boilerplate!();

    /// `toRules(boolean escapeUnprintable)`: override `Transliterator`:
    /// create rules that create this compound transliterator.
    fn to_rules(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        // We do NOT call toRules() on our component transliterators, in
        // general.  If we have several rule-based transliterators, this
        // yields a concatenation of the rules -- not what we want.  We do
        // handle compound RBT transliterators specially -- those for which
        // compoundRBTIndex >= 0.  For the transliterator at compoundRBTIndex,
        // we do call toRules() recursively.
        let mut rules_source = Vec::new();
        if self.num_anonymous_rbts >= 1 {
            if let Some(filter) = self.base.get_filter() {
                // We might be a compound RBT and if we have a global
                // filter, then emit it at the top.
                rules_source.extend_from_slice(&[b':' as u16, b':' as u16]);
                rules_source.extend_from_slice(&filter.to_pattern(escape_unprintable));
                rules_source.push(ID_DELIM);
            }
        }
        for i in 0..self.trans.len() {
            let mut rule;

            // Anonymous RuleBasedTransliterators (inline rules and
            // ::BEGIN/::END blocks) are given IDs that begin with
            // "%Pass": use toRules() to write all the rules to the output
            // (and insert "::Null;" if we have two in a row)
            let id = self.trans[i].get_id();
            if starts_with_pass(&id) {
                rule = self.trans[i].to_rules(escape_unprintable)?;
                if self.num_anonymous_rbts > 1
                    && i > 0
                    && starts_with_pass(&self.trans[i - 1].get_id())
                {
                    let mut r = crate::jdk::units("::Null;");
                    r.extend_from_slice(&rule);
                    rule = r;
                }

            // we also use toRules() on CompoundTransliterators (which we
            // check for by looking for a semicolon in the ID)-- this gets
            // the list of their child transliterators output in the right
            // format
            } else if id.contains(&(b';' as u16)) {
                rule = self.trans[i].to_rules(escape_unprintable)?;

            // for everything else, use baseToRules()
            } else {
                rule = self.trans[i].base_to_rules(escape_unprintable)?;
            }
            Self::smart_append(&mut rules_source, b'\n' as u16);
            rules_source.extend_from_slice(&rule);
            Self::smart_append(&mut rules_source, ID_DELIM);
        }
        Ok(rules_source)
    }

    /// `addSourceTargetSet(UnicodeSet filter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(
        &self,
        filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        let mut my_filter = UnicodeSet::from_set(&self.get_filter_as_unicode_set(filter)?);
        let mut temp_target_set = UnicodeSet::new();
        for t in self.trans.iter() {
            // each time we produce targets, those can be used by subsequent items, despite the filter.
            // so we get just those items, and add them to the filter each time.
            temp_target_set.clear()?;
            t.add_source_target_set(&my_filter, source_set, &mut temp_target_set)?;
            target_set.add_all(&temp_target_set)?;
            my_filter.add_all(&temp_target_set)?;
        }
        Ok(())
    }

    /// `handleTransliterate(Replaceable text, Position index, boolean
    /// incremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        index: &mut Position,
        incremental: bool,
    ) -> Result<()> {
        /* Call each transliterator with the same start value and
         * initial cursor index, but with the limit index as modified
         * by preceding transliterators.  The cursor index must be
         * reset for each transliterator to give each a chance to
         * transliterate the text.  The initial cursor index is known
         * to still point to the same place after each transliterator
         * is called because each transliterator will not change the
         * text between start and the initial value of cursor.
         *
         * IMPORTANT: After the first transliterator, each subsequent
         * transliterator only gets to transliterate text committed by
         * preceding transliterators; that is, the cursor (output
         * value) of transliterator i becomes the limit (input value)
         * of transliterator i+1.  Finally, the overall limit is fixed
         * up before we return.
         *
         * Assumptions we make here:
         * (1) contextStart <= start <= limit <= contextLimit <= text.length()
         * (2) start <= start' <= limit'  ;cursor doesn't move back
         * (3) start <= limit'            ;text before cursor unchanged
         * - start' is the value of start after calling handleKT
         * - limit' is the value of limit after calling handleKT
         */

        /*
         * Example: 3 transliterators.  This example illustrates the
         * mechanics we need to implement.  C, S, and L are the contextStart,
         * start, and limit.  gl is the globalLimit.  contextLimit is
         * equal to limit throughout.
         *
         * 1. h-u, changes hex to Unicode
         *
         *    4  7  a  d  0      4  7  a
         *    abc/u0061/u    =>  abca/u
         *    C  S       L       C   S L   gl=f->a
         *
         * 2. upup, changes "x" to "XX"
         *
         *    4  7  a       4  7  a
         *    abca/u    =>  abcAA/u
         *    C  SL         C    S
         *                       L    gl=a->b
         * 3. u-h, changes Unicode to hex
         *
         *    4  7  a        4  7  a  d  0  3
         *    abcAA/u    =>  abc/u0041/u0041/u
         *    C  S L         C              S
         *                                  L   gl=b->15
         * 4. return
         *
         *    4  7  a  d  0  3
         *    abc/u0041/u0041/u
         *    C S L
         */

        if self.trans.is_empty() {
            index.start = index.limit;
            return Ok(()); // Short circuit for empty compound translit
        }

        // compoundLimit is the limit value for the entire compound
        // operation.  We overwrite index.limit with the previous
        // index.start.  After each transliteration, we update
        // compoundLimit for insertions or deletions that have happened.
        let mut compound_limit = index.limit;

        // compoundStart is the start for the entire compound
        // operation.
        let compound_start = index.start;

        let mut delta = 0; // delta in length

        // Give each transliterator a crack at the run of characters.
        // See comments at the top of the method for more detail.
        for t in self.trans.iter() {
            index.start = compound_start; // Reset start
            let limit = index.limit;

            if index.start == index.limit {
                // Short circuit for empty range
                break;
            }

            t.filtered_transliterate(text, index, incremental)?;

            // In a properly written transliterator, start == limit after
            // handleTransliterate() returns when incremental is false.
            // Catch cases where the subclass doesn't do this, and throw
            // an exception.  (Just pinning start to limit is a bad idea,
            // because what's probably happening is that the subclass
            // isn't transliterating all the way to the end, and it should
            // in non-incremental mode.)
            if !incremental && index.start != index.limit {
                return Err(runtime_exception(Some(format!(
                    "ERROR: Incomplete non-incremental transliteration by {}",
                    to_rust_string(&t.get_id())
                ))));
            }

            // Cumulative delta for insertions/deletions
            delta += index.limit - limit;

            if incremental {
                // In the incremental case, only allow subsequent
                // transliterators to modify what has already been
                // completely processed by prior transliterators.  In the
                // non-incrmental case, allow each transliterator to
                // process the entire text.
                index.limit = index.start;
            }
        }

        compound_limit += delta;

        // Start is good where it is -- where the last transliterator left
        // it.  Limit needs to be put back where it was, modulo
        // adjustments for deletions/insertions.
        index.limit = compound_limit;
        Ok(())
    }
}
