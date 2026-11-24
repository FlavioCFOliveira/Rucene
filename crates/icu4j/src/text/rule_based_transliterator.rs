// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RuleBasedTransliterator`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use rucene::error::Result;

use crate::text::function_replacer::FunctionReplacer;
use crate::text::quantifier::Quantifier;
use crate::text::replaceable::Replaceable;
use crate::text::string_matcher::StringMatcher;
use crate::text::transliteration_rule_set::TransliterationRuleSet;
use crate::text::transliterator::{
    transliterator_boilerplate, Position, Transliterator, TransliteratorBase,
};
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_matcher::UnicodeMatcher;
use crate::text::unicode_replacer::UnicodeReplacer;
use crate::text::unicode_set::UnicodeSet;

/// A transliterator that reads a set of rules in order to determine how to
/// perform translations. Rule sets are stored in resource bundles indexed
/// by name, and are compiled at run time by `TransliteratorParser`.
///
/// Equivalent to the (deprecated) class
/// `com.ibm.icu.text.RuleBasedTransliterator`.
#[derive(Debug)]
pub struct RuleBasedTransliterator {
    base: TransliteratorBase,
    data: Arc<Data>,
}

impl RuleBasedTransliterator {
    /// `RuleBasedTransliterator(String ID, Data data, UnicodeFilter
    /// filter)`.
    ///
    /// # Errors
    ///
    /// The errors of `setFilter` (`addMatchSetTo` of a non-`UnicodeSet`
    /// filter).
    pub(crate) fn new(
        id: Vec<u16>,
        data: Arc<Data>,
        filter: Option<&dyn UnicodeFilter>,
    ) -> Result<Self> {
        let base = TransliteratorBase::new(id, filter)?;
        let max = data.rule_set().get_maximum_context_length();
        base.set_maximum_context_length(max)?;
        Ok(Self { base, data })
    }

    /// `safeClone()`: a new transliterator over the same data, with a copy
    /// of the filter.
    ///
    /// # Errors
    ///
    /// The errors of the constructor.
    pub fn safe_clone(&self) -> Result<Arc<dyn Transliterator>> {
        let filter = self.base.get_filter();
        // filter instanceof UnicodeSet: the stored filter is always one
        let filter = filter.map(|f| UnicodeSet::from_set(&f));
        Ok(Arc::new(RuleBasedTransliterator::new(
            self.base.get_id(),
            self.data.clone(),
            filter.as_ref().map(|f| f as &dyn UnicodeFilter),
        )?))
    }
}

impl Transliterator for RuleBasedTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position index, boolean
    /// incremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        index: &mut Position,
        incremental: bool,
    ) -> Result<()> {
        // We keep contextStart and contextLimit fixed the entire time,
        // relative to the text -- contextLimit may move numerically if
        // text is inserted or removed.  The start offset moves toward
        // limit, with replacements happening under it.
        //
        // Example: rules 1. ab>x|y
        //                2. yc>z
        //
        // |eabcd   start - no match, advance start
        // e|abcd   match rule 1 - change text & adjust start
        // ex|ycd   match rule 2 - change text & adjust start
        // exz|d    no match, advance start
        // exzd|    done

        // A rule like
        //   a>b|a
        // creates an infinite loop. To prevent that, we put an arbitrary
        // limit on the number of iterations that we take, one that is
        // high enough that any reasonable rules are ok, but low enough to
        // prevent a server from hanging.  The limit is 16 times the
        // number of characters n, unless n is so large that 16n exceeds a
        // uint32_t.
        let rule_set = self.data.rule_set(); // synchronized(data)
        let mut loop_count = 0;
        let mut loop_limit = (index.limit - index.start) << 4;
        if loop_limit < 0 {
            loop_limit = 0x7FFFFFFF;
        }

        while index.start < index.limit
            && loop_count <= loop_limit
            && rule_set.transliterate(text, index, incremental)?
        {
            loop_count += 1;
        }
        Ok(())
    }

    /// `toRules(boolean escapeUnprintable)`.
    fn to_rules(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        self.data.rule_set().to_rules(escape_unprintable)
    }

    /// `addSourceTargetSet(UnicodeSet filter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(
        &self,
        filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        self.data
            .rule_set()
            .add_source_target_set(filter, source_set, target_set)
    }
}

/// An entry of `Data.variables`: Java's `Object[]` holds `UnicodeSet`,
/// `StringMatcher`, `Quantifier` and `FunctionReplacer` objects, and `null`
/// for a segment stand-in whose matcher is not yet known.
#[derive(Clone)]
pub(crate) enum Variable {
    /// `null`.
    Null,
    /// A `UnicodeSet` (a `UnicodeMatcher`).
    Set(Arc<UnicodeSet>),
    /// A `StringMatcher` (a `UnicodeMatcher` and a `UnicodeReplacer`).
    StringMatcher(Arc<StringMatcher>),
    /// A `Quantifier` (a `UnicodeMatcher`).
    Quantifier(Arc<Quantifier>),
    /// A `FunctionReplacer` (a `UnicodeReplacer`).
    FunctionReplacer(Arc<FunctionReplacer>),
}

impl Variable {
    fn class_name(&self) -> &'static str {
        match self {
            Variable::Null => "null",
            Variable::Set(_) => "com.ibm.icu.text.UnicodeSet",
            Variable::StringMatcher(_) => "com.ibm.icu.text.StringMatcher",
            Variable::Quantifier(_) => "com.ibm.icu.text.Quantifier",
            Variable::FunctionReplacer(_) => "com.ibm.icu.text.FunctionReplacer",
        }
    }

    /// `(UnicodeMatcher) variable`.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` for a `FunctionReplacer` (the parser
    /// rejects such rules, so it does not occur).
    pub(crate) fn as_matcher(&self) -> Option<&dyn UnicodeMatcher> {
        match self {
            Variable::Null => None,
            Variable::Set(s) => Some(&**s),
            Variable::StringMatcher(m) => Some(&**m),
            Variable::Quantifier(q) => Some(&**q),
            Variable::FunctionReplacer(_) => panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.text.UnicodeMatcher",
                self.class_name()
            ),
        }
    }

    /// `(UnicodeMatcher) variable` as a shared reference.
    ///
    /// # Panics
    ///
    /// As [`as_matcher`](Self::as_matcher).
    pub(crate) fn as_matcher_arc(&self) -> Option<Arc<dyn UnicodeMatcher>> {
        match self {
            Variable::Null => None,
            Variable::Set(s) => Some(s.clone()),
            Variable::StringMatcher(m) => Some(m.clone()),
            Variable::Quantifier(q) => Some(q.clone()),
            Variable::FunctionReplacer(_) => panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.text.UnicodeMatcher",
                self.class_name()
            ),
        }
    }

    /// `(UnicodeReplacer) variable`.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` for a `UnicodeSet` or a `Quantifier`
    /// (the parser rejects such rules, so it does not occur).
    pub(crate) fn as_replacer(&self) -> Option<&dyn UnicodeReplacer> {
        match self {
            Variable::Null => None,
            Variable::StringMatcher(m) => Some(&**m),
            Variable::FunctionReplacer(f) => Some(&**f),
            Variable::Set(_) | Variable::Quantifier(_) => panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.text.UnicodeReplacer",
                self.class_name()
            ),
        }
    }

    /// `variable instanceof UnicodeMatcher`.
    pub(crate) fn is_matcher(&self) -> bool {
        matches!(
            self,
            Variable::Set(_) | Variable::StringMatcher(_) | Variable::Quantifier(_)
        )
    }

    /// `variable instanceof UnicodeReplacer`.
    pub(crate) fn is_replacer(&self) -> bool {
        matches!(
            self,
            Variable::StringMatcher(_) | Variable::FunctionReplacer(_)
        )
    }

    /// `variable == other`: the same object (`null == null`).
    pub(crate) fn same_object(&self, other: &Variable) -> bool {
        match (self, other) {
            (Variable::Null, Variable::Null) => true,
            (Variable::Set(a), Variable::Set(b)) => Arc::ptr_eq(a, b),
            (Variable::StringMatcher(a), Variable::StringMatcher(b)) => Arc::ptr_eq(a, b),
            (Variable::Quantifier(a), Variable::Quantifier(b)) => Arc::ptr_eq(a, b),
            (Variable::FunctionReplacer(a), Variable::FunctionReplacer(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// The data of a `RuleBasedTransliterator`: its rule set and the variables
/// its stand-in characters refer to.
///
/// Equivalent to the nested class `RuleBasedTransliterator.Data`.
///
/// **Divergence.** Java's fields (`RuleBasedTransliterator.java:120-158`)
/// are plain fields mutated by the parser and shared by the matchers built
/// over this data; here they sit behind
/// interior mutability: `ruleSet` behind the [`Mutex`] that also stands for
/// `synchronized(data)` in `handleTransliterate`, `variables` in a
/// [`OnceLock`] (Java assigns it once, after the parse), `variablesBase` in
/// an atomic. The matchers hold this data weakly (an `Arc` would close a
/// cycle); since a variable defined in one rule section may be used by a
/// later section of the same rule source, each data holds the data of the
/// sections parsed before it (`earlier`), which Java's garbage collector
/// keeps alive through those matchers.
pub struct Data {
    /// `ruleSet`: the rules.
    rule_set: Mutex<TransliterationRuleSet>,
    /// `variableNames`: map variable name (String) to variable (char[]). A
    /// variable name corresponds to zero or more characters, stored in a
    /// char[] array in this hash. One or more of these chars may also
    /// correspond to a UnicodeMatcher, in which case the character in the
    /// char[] in this hash is a stand-in: it is an index for a secondary
    /// lookup in `variables`.
    variable_names: Mutex<HashMap<Vec<u16>, Vec<u16>>>,
    /// `variables`: map category variable (Character) to UnicodeMatcher or
    /// UnicodeReplacer. Variables that correspond to a set of characters
    /// are mapped from variable name to a stand-in character in
    /// `variableNames`. The stand-in then serves as a key in this hash to
    /// lookup the actual UnicodeSet object. In addition, the stand-in is
    /// stored in the rule text to represent the set of characters.
    /// `variables[i]` represents character `(variablesBase + i)`.
    variables: OnceLock<Vec<Variable>>,
    /// `variablesBase`: the character that represents `variables[0]`.
    /// Characters `variablesBase..variablesBase+variables.length-1`
    /// represent `UnicodeSet` and `UnicodeMatcher` objects.
    variables_base: AtomicU16,
    /// The data of the sections parsed before this one (see the type's
    /// divergence note); held only to keep them alive.
    #[allow(dead_code)]
    earlier: Vec<Arc<Data>>,
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data")
            .field("variables_base", &self.variables_base())
            .finish()
    }
}

impl Default for Data {
    fn default() -> Self {
        Self::new()
    }
}

impl Data {
    /// `Data()`: an empty rule set and no variables.
    pub fn new() -> Self {
        Self::new_after(Vec::new())
    }

    /// `Data()` for a section that follows the sections `earlier` in the
    /// same rule source.
    pub(crate) fn new_after(earlier: Vec<Arc<Data>>) -> Self {
        Self {
            variable_names: Mutex::new(HashMap::new()),
            rule_set: Mutex::new(TransliterationRuleSet::new()),
            variables: OnceLock::new(),
            variables_base: AtomicU16::new(0),
            earlier,
        }
    }

    /// `ruleSet`, locked (`synchronized(data)`).
    pub(crate) fn rule_set(&self) -> MutexGuard<'_, TransliterationRuleSet> {
        self.rule_set.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `variableNames`, locked.
    pub(crate) fn variable_names(&self) -> MutexGuard<'_, HashMap<Vec<u16>, Vec<u16>>> {
        self.variable_names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// `variables = ...`: assigns the variables (once, after the parse).
    pub(crate) fn set_variables(&self, variables: Vec<Variable>) {
        // Java assigns the field once per parse; a second assignment does
        // not occur.
        let _ = self.variables.set(variables);
    }

    /// `variablesBase`.
    pub(crate) fn variables_base(&self) -> u16 {
        self.variables_base.load(Ordering::Relaxed)
    }

    /// `variablesBase = (char) start`.
    pub(crate) fn set_variables_base(&self, base: u16) {
        self.variables_base.store(base, Ordering::Relaxed);
    }

    fn variables(&self) -> &[Variable] {
        self.variables
            .get()
            .map(Vec::as_slice)
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: Cannot read the array length because \"this.variables\" is null"))
    }

    /// `lookupMatcher(int standIn)`: the matcher object that a stand-in
    /// represents, or `None`.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` before the variables are assigned, and
    /// its `ClassCastException` for a stand-in of a replacer-only object.
    pub fn lookup_matcher(&self, stand_in: i32) -> Option<&dyn UnicodeMatcher> {
        let i = stand_in - self.variables_base() as i32;
        let variables = self.variables();
        if i >= 0 && i < variables.len() as i32 {
            variables[i as usize].as_matcher()
        } else {
            None
        }
    }

    /// `lookupReplacer(int standIn)`: the replacer object that a stand-in
    /// represents, or `None`.
    ///
    /// # Panics
    ///
    /// As [`lookup_matcher`](Self::lookup_matcher), for a matcher-only
    /// object.
    pub fn lookup_replacer(&self, stand_in: i32) -> Option<&dyn UnicodeReplacer> {
        let i = stand_in - self.variables_base() as i32;
        let variables = self.variables();
        if i >= 0 && i < variables.len() as i32 {
            variables[i as usize].as_replacer()
        } else {
            None
        }
    }
}
