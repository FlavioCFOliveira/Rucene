// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.SortedSetRelation`.

use std::collections::BTreeSet;

use rucene::error::{LuceneError, Result};

/// Computes the relations between sorted sets, as in a Venn diagram.
///
/// Equivalent to `com.ibm.icu.impl.SortedSetRelation` (Java's
/// `SortedSet<T>` with a natural order is a `BTreeSet<T>`).
pub struct SortedSetRelation;

impl SortedSetRelation {
    /// `A_NOT_B`.
    pub const A_NOT_B: i32 = 4;
    /// `A_AND_B`.
    pub const A_AND_B: i32 = 2;
    /// `B_NOT_A`.
    pub const B_NOT_A: i32 = 1;
    /// `ANY`.
    pub const ANY: i32 = Self::A_NOT_B | Self::A_AND_B | Self::B_NOT_A;
    /// `CONTAINS`.
    pub const CONTAINS: i32 = Self::A_NOT_B | Self::A_AND_B;
    /// `DISJOINT`.
    pub const DISJOINT: i32 = Self::A_NOT_B | Self::B_NOT_A;
    /// `ISCONTAINED`.
    pub const ISCONTAINED: i32 = Self::A_AND_B | Self::B_NOT_A;
    /// `NO_B`.
    pub const NO_B: i32 = Self::A_NOT_B;
    /// `EQUALS`.
    pub const EQUALS: i32 = Self::A_AND_B;
    /// `NO_A`.
    pub const NO_A: i32 = Self::B_NOT_A;
    /// `NONE`.
    pub const NONE: i32 = 0;
    /// `ADDALL`.
    pub const ADDALL: i32 = Self::ANY;
    /// `A`.
    pub const A: i32 = Self::CONTAINS;
    /// `COMPLEMENTALL`.
    pub const COMPLEMENTALL: i32 = Self::DISJOINT;
    /// `B`.
    pub const B: i32 = Self::ISCONTAINED;
    /// `REMOVEALL`.
    pub const REMOVEALL: i32 = Self::NO_B;
    /// `RETAINALL`.
    pub const RETAINALL: i32 = Self::EQUALS;
    /// `B_REMOVEALL`.
    pub const B_REMOVEALL: i32 = Self::NO_A;

    /// `hasRelation(SortedSet a, int allow, SortedSet b)`: whether the
    /// relation of `a` and `b` is allowed by the `allow` bits.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for `allow` outside `NONE..=ANY`.
    pub fn has_relation<T: Ord>(a: &BTreeSet<T>, allow: i32, b: &BTreeSet<T>) -> Result<bool> {
        if !(Self::NONE..=Self::ANY).contains(&allow) {
            return Err(LuceneError::illegal_argument(format!("Relation {allow} out of range")));
        }
        // extract filter conditions
        // these are the ALLOWED conditions Set
        let anb = (allow & Self::A_NOT_B) != 0;
        let ab = (allow & Self::A_AND_B) != 0;
        let bna = (allow & Self::B_NOT_A) != 0;

        // quick check on sizes
        match allow {
            Self::CONTAINS => {
                if a.len() < b.len() {
                    return Ok(false);
                }
            }
            Self::ISCONTAINED => {
                if a.len() > b.len() {
                    return Ok(false);
                }
            }
            Self::EQUALS
                if a.len() != b.len() => {
                    return Ok(false);
                }
            _ => {}
        }

        // check for null sets
        if a.is_empty() {
            if b.is_empty() {
                return Ok(true);
            }
            return Ok(bna);
        } else if b.is_empty() {
            return Ok(anb);
        }

        // pick up first strings, and start comparing
        let mut ait = a.iter().peekable();
        let mut bit = b.iter().peekable();
        let mut aa = ait.next().expect("a is not empty");
        let mut bb = bit.next().expect("b is not empty");

        loop {
            let comp = aa.cmp(bb);
            if comp.is_eq() {
                if !ab {
                    return Ok(false);
                }
                if ait.peek().is_none() {
                    if bit.peek().is_none() {
                        return Ok(true);
                    }
                    return Ok(bna);
                } else if bit.peek().is_none() {
                    return Ok(anb);
                }
                aa = ait.next().expect("checked by peek");
                bb = bit.next().expect("checked by peek");
            } else if comp.is_lt() {
                if !anb {
                    return Ok(false);
                }
                match ait.next() {
                    None => return Ok(bna),
                    Some(x) => aa = x,
                }
            } else {
                if !bna {
                    return Ok(false);
                }
                match bit.next() {
                    None => return Ok(anb),
                    Some(x) => bb = x,
                }
            }
        }
    }

    /// `doOperation(SortedSet a, int relation, SortedSet b)`: replaces
    /// `a` by the elements the relation selects, and returns it.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an unknown relation.
    pub fn do_operation<'a, T: Ord + Clone>(a: &'a mut BTreeSet<T>, relation: i32, b: &BTreeSet<T>) -> Result<&'a mut BTreeSet<T>> {
        // TODO: optimize this as above
        match relation {
            Self::ADDALL => {
                a.extend(b.iter().cloned());
            }
            Self::A => {
                // no action
            }
            Self::B => {
                a.clear();
                a.extend(b.iter().cloned());
            }
            Self::REMOVEALL => {
                a.retain(|x| !b.contains(x));
            }
            Self::RETAINALL => {
                a.retain(|x| b.contains(x));
            }
            // the following is the only case not really supported by Java
            // although all could be optimized
            Self::COMPLEMENTALL => {
                let temp: BTreeSet<T> = b.iter().filter(|x| !a.contains(*x)).cloned().collect();
                a.retain(|x| !b.contains(x));
                a.extend(temp);
            }
            Self::B_REMOVEALL => {
                let temp: BTreeSet<T> = b.iter().filter(|x| !a.contains(*x)).cloned().collect();
                a.clear();
                a.extend(temp);
            }
            Self::NONE => {
                a.clear();
            }
            _ => return Err(LuceneError::illegal_argument(format!("Relation {relation} out of range"))),
        }
        Ok(a)
    }
}
