// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.TransliteratorRegistry`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::error::Result;
use rucene::util::java_collections::{JavaHashCode, JavaObjectHashMap};

use crate::jdk::{to_rust_string, units};
use crate::lang::u_script::UScript;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::locale_utility::LocaleUtility;
use crate::text::any_transliterator::AnyTransliterator;
use crate::text::compound_transliterator::CompoundTransliterator;
use crate::text::null_transliterator::NullTransliterator;
use crate::text::rule_based_transliterator::{Data, RuleBasedTransliterator};
use crate::text::transliterator::{
    self, Factory, Transliterator, TransliteratorClass, FORWARD, REVERSE,
};
use crate::text::transliterator_id_parser::{equals_ignore_case, TransliteratorIDParser};
use crate::text::transliterator_parser::TransliteratorParser;
use crate::text::unicode_set::UnicodeSet;
use crate::util::case_insensitive_string::CaseInsensitiveString;
use crate::util::u_locale::ENGLISH;
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

const LOCALE_SEP: u16 = b'_' as u16;

// The registry needs to know how to handle IDs of the form
// "source-target/variant" where source is "Any", and "source-target"
// where source is "Any".

/// `NO_VARIANT`: empty string.
const NO_VARIANT: &[u16] = &[];

/// `ANY`.
const ANY: &str = "Any";

impl JavaHashCode for CaseInsensitiveString {
    /// `CaseInsensitiveString.hashCode()`.
    fn java_hash_code(&self) -> i32 {
        self.hash_code()
    }
}

/// An entry of the registry: Java's `Object` in the `Object[]` wrapper the
/// registry maps an ID to.
#[derive(Clone)]
enum Entry {
    /// A `RuleBasedTransliterator.Data`.
    Data(Arc<Data>),
    /// A `Class<? extends Transliterator>`.
    Class(TransliteratorClass),
    /// An `AliasEntry`: the ID (or compound ID) it aliases.
    Alias(Vec<u16>),
    /// A `Transliterator.Factory`.
    Factory(Arc<dyn Factory>),
    /// A `CompoundRBTEntry`.
    CompoundRbt(Arc<CompoundRBTEntry>),
    /// A `Transliterator` instance.
    Transliterator(Arc<dyn Transliterator>),
    /// A `ResourceEntry`.
    Resource(ResourceEntry),
    /// A `LocaleEntry`.
    Locale(LocaleEntry),
}

/// Java's `Object[]` of one element wrapping an entry: shared between the
/// registry and the finders, and replaced in place once a resource entry
/// is parsed.
type EntryWrapper = Arc<Mutex<Entry>>;

fn lock_entry(wrapper: &EntryWrapper) -> MutexGuard<'_, Entry> {
    wrapper.lock().unwrap_or_else(|e| e.into_inner())
}

/// A specification of a source, target or variant that can fall back: a
/// locale ID falls back to its parents, then to its script; a script name
/// falls back to nothing.
///
/// Equivalent to the nested class `TransliteratorRegistry.Spec`.
///
/// **Divergence.** Java tests `Utility.sameObjects(spec, top)` and
/// `Utility.sameObjects(nextSpec, scriptName)` (string identity,
/// `TransliteratorRegistry.java:151`, `:173`); the
/// flags `spec_is_top` and `next_is_script_name` record which object each
/// field holds.
struct Spec {
    top: Vec<u16>,                 // top spec
    spec: Option<Vec<u16>>,        // current spec
    spec_is_top: bool,             // spec is the object top
    next_spec: Option<Vec<u16>>,   // next spec
    next_is_script_name: bool,     // nextSpec is the object scriptName
    script_name: Option<Vec<u16>>, // script name equivalent of top, if != top
    is_spec_locale: bool,          // true if spec is a locale
    is_next_locale: bool,          // true if nextSpec is a locale
    res: Option<UResourceBundle>,
}

impl Spec {
    /// `Spec(String theSpec)`.
    fn new(the_spec: &[u16]) -> Result<Self> {
        let mut this = Self {
            top: the_spec.to_vec(),
            spec: None,
            spec_is_top: false,
            next_spec: None,
            next_is_script_name: false,
            script_name: None,
            is_spec_locale: false,
            is_next_locale: false,
            res: None,
        };
        let top = to_rust_string(&this.top);
        let attempt = (|| -> Result<()> {
            // Canonicalize script name.  If top is a script name then
            // script != UScript.INVALID_CODE.
            let script = UScript::get_code_from_name(&top)?;

            // Canonicalize script name -or- do locale->script mapping
            let s = UScript::get_code(&top)?;
            if let Some(s) = s {
                let name = UScript::get_name(s[0])?.unwrap_or_else(|| "null".to_string());
                let name = units(&name);
                // If the script name is the same as top then it's
                // redundant
                this.script_name = if equals_ignore_case(&name, &this.top) {
                    None
                } else {
                    Some(name)
                };
            }

            // Canonicalize top
            this.is_spec_locale = false;
            this.res = None;
            // If 'top' is not a script name, try a locale lookup
            if script == UScript::INVALID_CODE {
                let toploc = LocaleUtility::get_locale_from_name(&top);
                let res = UResourceBundle::get_bundle_instance_locale(
                    Some(ICUData::ICU_TRANSLIT_BASE_NAME),
                    Some(&toploc),
                )?;
                // Make sure we got the bundle we wanted; otherwise, don't use it
                if LocaleUtility::is_fallback_of(&res.get_u_locale()?.to_string(), &top) {
                    this.is_spec_locale = true;
                }
                this.res = Some(res);
            }
            Ok(())
        })();
        match attempt {
            Ok(()) => {}
            Err(e) if is_missing_resource(&e) => {
                //CLOVER:OFF
                this.script_name = None;
                //CLOVER:ON
            }
            Err(e) => return Err(e),
        }
        // assert(spec != top);
        this.reset()?;
        Ok(this)
    }

    /// `hasFallback()`.
    fn has_fallback(&self) -> bool {
        self.next_spec.is_some()
    }

    /// `reset()`.
    fn reset(&mut self) -> Result<()> {
        if !self.spec_is_top {
            self.spec = Some(self.top.clone());
            self.spec_is_top = true;
            self.is_spec_locale = self.res.is_some();
            self.setup_next();
        }
        Ok(())
    }

    /// `setupNext()`.
    fn setup_next(&mut self) {
        self.is_next_locale = false;
        if self.is_spec_locale {
            let spec = self.spec.clone().expect("a locale spec is not null");
            let i = crate::jdk::last_index_of_unit(&spec, LOCALE_SEP, spec.len() as i32 - 1);
            // If i == 0 then we have _FOO, so we fall through
            // to the scriptName.
            if i > 0 {
                self.next_spec = Some(spec[..i as usize].to_vec());
                self.next_is_script_name = false;
                self.is_next_locale = true;
            } else {
                self.next_spec = self.script_name.clone(); // scriptName may be null
                self.next_is_script_name = true;
            }
        } else {
            // Fallback to the script, which may be null
            let same = self.next_is_script_name
                || (self.next_spec.is_none() && self.script_name.is_none());
            if !same {
                self.next_spec = self.script_name.clone();
                self.next_is_script_name = true;
            } else {
                self.next_spec = None;
                self.next_is_script_name = false;
            }
        }
    }

    // Protocol:
    // for(String& s(spec.get());
    //     spec.hasFallback(); s(spec.next())) { ...

    /// `next()`.
    fn next(&mut self) -> Option<Vec<u16>> {
        self.spec = self.next_spec.clone();
        self.spec_is_top = false;
        self.is_spec_locale = self.is_next_locale;
        self.setup_next();
        self.spec.clone()
    }

    /// `get()`.
    fn get(&self) -> Option<&[u16]> {
        self.spec.as_deref()
    }

    /// `isLocale()`.
    fn is_locale(&self) -> bool {
        self.is_spec_locale
    }

    /// `getBundle()`: return the `ResourceBundle` for this spec, at the
    /// current level of iteration. The level of iteration goes from
    /// aa_BB_CCC to aa_BB to aa. If the bundle does not correspond to the
    /// current level of iteration, return `None`. If `is_locale()` is
    /// false, always return `None`.
    fn get_bundle(&self) -> Result<Option<&UResourceBundle>> {
        if let Some(res) = &self.res {
            if let Some(spec) = &self.spec {
                if units(&res.get_u_locale()?.to_string()) == *spec {
                    return Ok(Some(res));
                }
            }
        }
        Ok(None)
    }

    /// `getTop()`.
    fn get_top(&self) -> &[u16] {
        &self.top
    }
}

/// `ResourceEntry`: rules held in a resource, with their direction.
#[derive(Debug, Clone)]
struct ResourceEntry {
    resource: Vec<u16>,
    direction: i32,
}

/// `LocaleEntry`: rules found in a locale bundle, with their direction.
#[derive(Debug, Clone)]
struct LocaleEntry {
    rule: Vec<u16>,
    direction: i32,
}

/// `CompoundRBTEntry`: the parse result of rules that make up a compound
/// of ID blocks and rule-based passes.
struct CompoundRBTEntry {
    id: Vec<u16>,
    id_block_vector: Vec<Vec<u16>>,
    data_vector: Vec<Arc<Data>>,
    compound_filter: Option<UnicodeSet>,
}

impl CompoundRBTEntry {
    /// `getInstance()`.
    fn get_instance(&self) -> Result<Arc<dyn Transliterator>> {
        let mut transliterators: Vec<Arc<dyn Transliterator>> = Vec::new();
        let mut pass_number = 1;

        let limit = self.id_block_vector.len().max(self.data_vector.len());
        for i in 0..limit {
            if i < self.id_block_vector.len() {
                let id_block = &self.id_block_vector[i];
                if !id_block.is_empty() {
                    transliterators.push(transliterator::get_instance(id_block)?);
                }
            }
            if i < self.data_vector.len() {
                let data = self.data_vector[i].clone();
                transliterators.push(Arc::new(RuleBasedTransliterator::new(
                    units(&format!("%Pass{pass_number}")),
                    data,
                    None,
                )?));
                pass_number += 1;
            }
        }

        let t: Arc<dyn Transliterator> = Arc::new(CompoundTransliterator::with_anonymous_rbts(
            transliterators,
            pass_number - 1,
        )?);
        t.set_id(self.id.clone());
        if let Some(compound_filter) = &self.compound_filter {
            t.set_filter(Some(compound_filter))?;
        }
        Ok(t)
    }
}

/// The registry of transliterator IDs: maps IDs to the entries that create
/// transliterators, finds IDs with fallback through locales and scripts,
/// and keeps the source/target/variant DAG of the visible IDs.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.TransliteratorRegistry`.
pub(crate) struct TransliteratorRegistry {
    /// Map of ID to `Object[]` wrapping an entry. The entry is: a
    /// `Class` (a constructor); a `Factory`; an `AliasEntry`; a
    /// `ResourceEntry` or `LocaleEntry` (rules, parsed on first use and
    /// replaced by their parse result); a `Data`; a `CompoundRBTEntry`; or
    /// a `Transliterator` instance. The `Object[]` wrapper is a
    /// `Collections.synchronizedMap(new HashMap())` value.
    registry: Mutex<HashMap<CaseInsensitiveString, EntryWrapper>>,
    /// DAG of visible IDs by spec. Hashtable: source => (Hashtable: target
    /// => (Vector: variant)). The UnicodeString objects in the Vector are
    /// owned by this object. We use a Vector instead of a Hashtable
    /// because we want the variants to be ordered (in the order they were
    /// registered).
    spec_dag: Mutex<
        JavaObjectHashMap<
            CaseInsensitiveString,
            JavaObjectHashMap<CaseInsensitiveString, Vec<CaseInsensitiveString>>,
        >,
    >,
    /// Vector of public full IDs (a `LinkedHashSet`).
    available_ids: Mutex<(Vec<CaseInsensitiveString>, HashSet<CaseInsensitiveString>)>,
}

impl fmt::Debug for TransliteratorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransliteratorRegistry")
            .finish_non_exhaustive()
    }
}

impl TransliteratorRegistry {
    /// `TransliteratorRegistry()`: an empty registry.
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(HashMap::new()),
            spec_dag: Mutex::new(JavaObjectHashMap::new()),
            available_ids: Mutex::new((Vec::new(), HashSet::new())),
        }
    }

    fn registry_map(&self) -> MutexGuard<'_, HashMap<CaseInsensitiveString, EntryWrapper>> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[allow(clippy::type_complexity)]
    fn spec_dag(
        &self,
    ) -> MutexGuard<
        '_,
        JavaObjectHashMap<
            CaseInsensitiveString,
            JavaObjectHashMap<CaseInsensitiveString, Vec<CaseInsensitiveString>>,
        >,
    > {
        self.spec_dag.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn available_ids(
        &self,
    ) -> MutexGuard<'_, (Vec<CaseInsensitiveString>, HashSet<CaseInsensitiveString>)> {
        self.available_ids.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `get(String ID, StringBuffer aliasReturn)`: given a simple ID
    /// (forward direction, no inline filter, not compound) attempt to
    /// instantiate it from the registry. Return 0 on failure. Return a
    /// non-empty `alias_return` value if the ID is an alias; the caller
    /// then instantiates the alias.
    ///
    /// # Errors
    ///
    /// The errors of the lookup and of the instantiation (rule parsing).
    pub fn get(
        &self,
        id: &[u16],
        alias_return: &mut Vec<u16>,
    ) -> Result<Option<Arc<dyn Transliterator>>> {
        match self.find(id)? {
            None => Ok(None),
            Some(entry) => self.instantiate_entry(id, &entry, alias_return),
        }
    }

    /// `put(String ID, Class transliteratorSubclass, boolean visible)`:
    /// register a class.
    ///
    /// # Errors
    ///
    /// The errors of [`TransliteratorIDParser::id_to_stv`] callers (none).
    pub fn put_class(
        &self,
        id: &[u16],
        transliterator_subclass: TransliteratorClass,
        visible: bool,
    ) -> Result<()> {
        self.register_entry_id(id, Entry::Class(transliterator_subclass), visible)
    }

    /// `put(String ID, Transliterator.Factory factory, boolean visible)`:
    /// register an ID and a factory function pointer for it.
    ///
    /// # Errors
    ///
    /// As [`put_class`](Self::put_class).
    pub fn put_factory(&self, id: &[u16], factory: Arc<dyn Factory>, visible: bool) -> Result<()> {
        self.register_entry_id(id, Entry::Factory(factory), visible)
    }

    /// `put(String ID, String resourceName, int dir, boolean visible)`:
    /// register an ID and a resource name. This adds an entry to the
    /// dynamic store, or replaces an existing entry. Any entry in the
    /// underlying static locale resource store is masked.
    ///
    /// # Errors
    ///
    /// As [`put_class`](Self::put_class).
    pub fn put_resource(
        &self,
        id: &[u16],
        resource_name: Vec<u16>,
        dir: i32,
        visible: bool,
    ) -> Result<()> {
        self.register_entry_id(
            id,
            Entry::Resource(ResourceEntry {
                resource: resource_name,
                direction: dir,
            }),
            visible,
        )
    }

    /// `put(String ID, String alias, boolean visible)`: register an ID and
    /// an alias ID. This adds an entry to the dynamic store, or replaces an
    /// existing entry. Any entry in the underlying static locale resource
    /// store is masked.
    ///
    /// # Errors
    ///
    /// As [`put_class`](Self::put_class).
    pub fn put_alias(&self, id: &[u16], alias: Vec<u16>, visible: bool) -> Result<()> {
        self.register_entry_id(id, Entry::Alias(alias), visible)
    }

    /// `put(String ID, Transliterator trans, boolean visible)`: register an
    /// ID and a `Transliterator` object. This adds an entry to the dynamic
    /// store, or replaces an existing entry. Any entry in the underlying
    /// static locale resource store is masked.
    ///
    /// # Errors
    ///
    /// As [`put_class`](Self::put_class).
    pub fn put_transliterator(
        &self,
        id: &[u16],
        trans: Arc<dyn Transliterator>,
        visible: bool,
    ) -> Result<()> {
        self.register_entry_id(id, Entry::Transliterator(trans), visible)
    }

    /// `remove(String ID)`: unregister an ID. This removes an entry from
    /// the dynamic store, if there is one. The static locale resource store
    /// is unaffected.
    ///
    /// # Errors
    ///
    /// None (kept for the uniform `put`/`remove` signatures).
    pub fn remove(&self, id: &[u16]) -> Result<()> {
        let stv = TransliteratorIDParser::id_to_stv(id);
        // Only need to do this if ID.indexOf('-') < 0
        let id = TransliteratorIDParser::stv_to_id(&stv.0, &stv.1, Some(&stv.2));
        let ci = CaseInsensitiveString::new(id);
        self.registry_map().remove(&ci);
        self.remove_stv(&stv.0, &stv.1, &stv.2);
        let mut ids = self.available_ids();
        if ids.1.remove(&ci) {
            ids.0.retain(|x| *x != ci);
        }
        Ok(())
    }

    /// `getAvailableIDs()`: the visible IDs, in registration order
    /// (Java returns an `Enumeration` over them).
    pub fn get_available_ids(&self) -> Vec<Vec<u16>> {
        self.available_ids()
            .0
            .iter()
            .map(|c| c.get_string().to_vec())
            .collect()
    }

    /// `getAvailableSources()`: the sources of the visible IDs, in Java's
    /// `HashMap` iteration order.
    pub fn get_available_sources(&self) -> Vec<Vec<u16>> {
        self.spec_dag()
            .keys()
            .map(|c| c.get_string().to_vec())
            .collect()
    }

    /// `getAvailableTargets(String source)`: the targets of the given
    /// source, in Java's `HashMap` iteration order.
    pub fn get_available_targets(&self, source: &[u16]) -> Vec<Vec<u16>> {
        let cisrc = CaseInsensitiveString::new(source.to_vec());
        let dag = self.spec_dag();
        match dag.get(&cisrc) {
            None => Vec::new(),
            Some(targets) => targets.keys().map(|c| c.get_string().to_vec()).collect(),
        }
    }

    /// `getAvailableVariants(String source, String target)`: the variants of
    /// the given source and target, in registration order.
    pub fn get_available_variants(&self, source: &[u16], target: &[u16]) -> Vec<Vec<u16>> {
        let cisrc = CaseInsensitiveString::new(source.to_vec());
        let citrg = CaseInsensitiveString::new(target.to_vec());
        let dag = self.spec_dag();
        let Some(targets) = dag.get(&cisrc) else {
            return Vec::new();
        };
        match targets.get(&citrg) {
            None => Vec::new(),
            Some(variants) => variants.iter().map(|c| c.get_string().to_vec()).collect(),
        }
    }

    //----------------------------------------------------------------------
    // class TransliteratorRegistry: internal
    //----------------------------------------------------------------------

    /// `registerEntry(String source, String target, String variant, Object
    /// entry, boolean visible)`: convenience method. Calls 6-arg
    /// `registerEntry()`.
    fn register_entry_stv(
        &self,
        source: &[u16],
        target: &[u16],
        variant: &[u16],
        entry: EntryWrapper,
        visible: bool,
    ) {
        let s = if source.is_empty() {
            units(ANY)
        } else {
            source.to_vec()
        };
        let id = TransliteratorIDParser::stv_to_id(source, target, Some(variant));
        self.register_entry_full(&id, &s, target, variant, entry, visible);
    }

    /// `registerEntry(String ID, Object entry, boolean visible)`:
    /// convenience method. Calls 6-arg `registerEntry()`.
    fn register_entry_id(&self, id: &[u16], entry: Entry, visible: bool) -> Result<()> {
        let stv = TransliteratorIDParser::id_to_stv(id);
        // Only need to do this if ID.indexOf('-') < 0
        let id = TransliteratorIDParser::stv_to_id(&stv.0, &stv.1, Some(&stv.2));
        self.register_entry_full(
            &id,
            &stv.0,
            &stv.1,
            &stv.2,
            Arc::new(Mutex::new(entry)),
            visible,
        );
        Ok(())
    }

    /// `registerEntry(String ID, String source, String target, String
    /// variant, Object entry, boolean visible)`: register an entry object
    /// (adopted) with the given ID, source, target, and variant strings.
    fn register_entry_full(
        &self,
        id: &[u16],
        source: &[u16],
        target: &[u16],
        variant: &[u16],
        entry: EntryWrapper,
        visible: bool,
    ) {
        let ci_id = CaseInsensitiveString::new(id.to_vec());

        // Store the entry within an array so it can be modified later
        self.registry_map().insert(ci_id.clone(), entry);
        if visible {
            self.register_stv(source, target, variant);
            let mut ids = self.available_ids();
            if ids.1.insert(ci_id.clone()) {
                ids.0.push(ci_id);
            }
        } else {
            self.remove_stv(source, target, variant);
            let mut ids = self.available_ids();
            if ids.1.remove(&ci_id) {
                ids.0.retain(|x| *x != ci_id);
            }
        }
    }

    /// `registerSTV(String source, String target, String variant)`:
    /// register a source-target/variant in the specDAG. Variant may be
    /// empty, but source and target must not be. If variant is empty then
    /// the special variant NO_VARIANT is stored in slot zero of the
    /// UVector of variants.
    fn register_stv(&self, source: &[u16], target: &[u16], variant: &[u16]) {
        // assert(source.length() > 0);
        // assert(target.length() > 0);
        let cisrc = CaseInsensitiveString::new(source.to_vec());
        let citrg = CaseInsensitiveString::new(target.to_vec());
        let civar = CaseInsensitiveString::new(variant.to_vec());
        let mut dag = self.spec_dag();
        if dag.get(&cisrc).is_none() {
            dag.insert(cisrc.clone(), JavaObjectHashMap::new());
        }
        let targets = dag.get_mut(&cisrc).expect("inserted above");
        if targets.get(&citrg).is_none() {
            targets.insert(citrg.clone(), Vec::new());
        }
        let variants = targets.get_mut(&citrg).expect("inserted above");
        // assert(NO_VARIANT == "");
        // We add the variant string.  If it is the special "no variant"
        // string, that is, the empty string, we add it at position zero.
        if !variants.contains(&civar) {
            if !variant.is_empty() {
                variants.push(civar);
            } else {
                variants.insert(0, civar);
            }
        }
    }

    /// `removeSTV(String source, String target, String variant)`: remove a
    /// source-target/variant from the specDAG.
    fn remove_stv(&self, source: &[u16], target: &[u16], variant: &[u16]) {
        // assert(source.length() > 0);
        // assert(target.length() > 0);
        let cisrc = CaseInsensitiveString::new(source.to_vec());
        let citrg = CaseInsensitiveString::new(target.to_vec());
        let civar = CaseInsensitiveString::new(variant.to_vec());
        let mut dag = self.spec_dag();
        let Some(targets) = dag.get_mut(&cisrc) else {
            return; // should never happen for valid s-t/v
        };
        let Some(variants) = targets.get_mut(&citrg) else {
            return; // should never happen for valid s-t/v
        };
        if let Some(i) = variants.iter().position(|v| *v == civar) {
            variants.remove(i);
        }
        if variants.is_empty() {
            targets.remove(&citrg); // should delete variants
            if targets.is_empty() {
                dag.remove(&cisrc); // should delete targets
            }
        }
    }

    /// `findInDynamicStore(Spec src, Spec trg, String variant)`: attempt to
    /// find a source-target/variant in the dynamic registry store. Return
    /// `None` on failure.
    fn find_in_dynamic_store(
        &self,
        src: &Spec,
        trg: &Spec,
        variant: &[u16],
    ) -> Option<EntryWrapper> {
        let id = TransliteratorIDParser::stv_to_id(
            src.get().unwrap_or(&units("null")),
            trg.get().unwrap_or(&units("null")),
            Some(variant),
        );
        self.registry_map()
            .get(&CaseInsensitiveString::new(id))
            .cloned()
    }

    /// `findInStaticStore(Spec src, Spec trg, String variant)`: attempt to
    /// find a source-target/variant in the static locale resource store.
    /// Do not perform fallback. Return `None` on failure.
    ///
    /// On success, create a new entry object, register it in the dynamic
    /// store, and return a pointer to it, but do not make it public --
    /// just because someone requested something, we do not expand the
    /// available ID list (or spec DAG).
    fn find_in_static_store(
        &self,
        src: &Spec,
        trg: &Spec,
        variant: &[u16],
    ) -> Result<Option<EntryWrapper>> {
        let mut entry = None;
        if src.is_locale() {
            entry = self.find_in_bundle(src, trg, variant, FORWARD)?;
        } else if trg.is_locale() {
            entry = self.find_in_bundle(trg, src, variant, REVERSE)?;
        }

        // If we found an entry, store it in the Hashtable for next
        // time.
        if let Some(entry) = &entry {
            self.register_entry_stv(src.get_top(), trg.get_top(), variant, entry.clone(), false);
        }

        Ok(entry)
    }

    /// `findInBundle(Spec specToOpen, Spec specToFind, String variant, int
    /// direction)`: attempt to find an entry in a single resource bundle.
    /// This is a one-sided lookup. `find_in_static_store()` performs up to
    /// two such lookups, one for the source, and one for the target.
    ///
    /// Do not perform fallback. Return `None` on failure.
    ///
    /// **Divergence.** The tag's `specToFind.get().toUpperCase(Locale.ENGLISH)`
    /// (`TransliteratorRegistry.java:685`) is computed with ICU's English
    /// upper-case mapping: the JDK's `String` case mapping is not in the
    /// workspace, and both apply the full Unicode mapping without
    /// English-specific rules.
    fn find_in_bundle(
        &self,
        spec_to_open: &Spec,
        spec_to_find: &Spec,
        variant: &[u16],
        direction: i32,
    ) -> Result<Option<EntryWrapper>> {
        // assert(specToOpen.isLocale());
        let Some(res) = spec_to_open.get_bundle()? else {
            // This means that the bundle's locale does not match
            // the current level of iteration for the spec.
            return Ok(None);
        };

        for pass in 0..2 {
            let mut tag = String::new();
            // First try either TransliteratorTo_xxx or
            // TransliterateFrom_xxx, then try the bidirectional
            // Transliterate_xxx.  This precedence order is arbitrary
            // but must be consistent and documented.
            if pass == 0 {
                tag.push_str(if direction == FORWARD {
                    "TransliterateTo"
                } else {
                    "TransliterateFrom"
                });
            } else {
                tag.push_str("Transliterate");
            }
            let find = spec_to_find.get().expect("the spec to find is not null");
            tag.push_str(&to_rust_string(
                &crate::lang::u_character::UCharacter::to_upper_case_ulocale(&ENGLISH, find),
            ));

            // The Transliterate*_xxx resource is an array of
            // strings of the format { <v0>, <r0>, ... }.  Each
            // <vi> is a variant name, and each <ri> is a rule.
            match res.get_string_array_by_key(&tag) {
                Ok(subres) => {
                    // assert(subres != null);
                    // assert(subres.length % 2 == 0);
                    let mut i = 0;
                    if !variant.is_empty() {
                        while i < subres.len() {
                            if equals_ignore_case(&subres[i], variant) {
                                break;
                            }
                            i += 2;
                        }
                    }

                    if i < subres.len() {
                        // We have a match, or there is no variant and i == 0.
                        // We have succeeded in loading a string from the
                        // locale resources.  Return the rule string which
                        // will itself become the registry entry.

                        // The direction is always forward for the
                        // TransliterateTo_xxx and TransliterateFrom_xxx
                        // items; those are unidirectional forward rules.
                        // For the bidirectional Transliterate_xxx items,
                        // the direction is the value passed in to this
                        // function.
                        let dir = if pass == 0 { FORWARD } else { direction };
                        return Ok(Some(Arc::new(Mutex::new(Entry::Locale(LocaleEntry {
                            rule: subres[i + 1].clone(),
                            direction: dir,
                        })))));
                    }
                }
                Err(e) if is_missing_resource(&e) => {}
                Err(e) => return Err(e),
            }
        }

        // If we get here we had a missing resource exception or we
        // failed to find a desired variant.
        Ok(None)
    }

    /// `find(String ID)`: convenience method. Calls 3-arg `find()`.
    fn find(&self, id: &[u16]) -> Result<Option<EntryWrapper>> {
        let stv = TransliteratorIDParser::id_to_stv(id);
        self.find_stv(&stv.0, &stv.1, &stv.2)
    }

    /// `find(String source, String target, String variant)`: top-level find
    /// method. Attempt to find a source-target/variant in either the
    /// dynamic or the static (locale resource) store. Perform fallback.
    ///
    /// Lookup sequence for ss_SS_SSS-tt_TT_TTT/v:
    ///
    ///   ss_SS_SSS-tt_TT_TTT/v -- in hashtable
    ///   ss_SS_SSS-tt_TT_TTT/v -- in ss_SS_SSS (no fallback)
    ///
    ///     repeat with t = tt_TT_TTT, tt_TT, tt, and tscript
    ///
    ///     ss_SS_SSS-t/*
    ///     ss_SS-t/*
    ///     ss-t/*
    ///     sscript-t/*
    ///
    /// Here * matches the first variant listed.
    ///
    /// Caller does NOT own returned object. Return `None` on failure.
    fn find_stv(
        &self,
        source: &[u16],
        target: &[u16],
        variant: &[u16],
    ) -> Result<Option<EntryWrapper>> {
        let mut src = Spec::new(source)?;
        let mut trg = Spec::new(target)?;

        // NOTE: The ICU4C implementation is more complex than this.  It
        // could be done this way, but this is clearer and probably faster.

        // Seek exact match in hashtable.  Temporary fix for ICU 4.6.
        // TODO: Fix for ICU 4.8 and later.
        if !variant.is_empty() {
            // Seek exact match in hashtable
            if let Some(entry) = self.find_in_dynamic_store(&src, &trg, variant) {
                return Ok(Some(entry));
            }

            // Seek exact match in locale resources
            if let Some(entry) = self.find_in_static_store(&src, &trg, variant)? {
                return Ok(Some(entry));
            }
        }

        loop {
            src.reset()?;
            loop {
                // Seek match in hashtable
                if let Some(entry) = self.find_in_dynamic_store(&src, &trg, NO_VARIANT) {
                    return Ok(Some(entry));
                }

                // Seek match in locale resources
                if let Some(entry) = self.find_in_static_store(&src, &trg, NO_VARIANT)? {
                    return Ok(Some(entry));
                }
                if !src.has_fallback() {
                    break;
                }
                src.next();
            }
            if !trg.has_fallback() {
                break;
            }
            trg.next();
        }

        Ok(None)
    }

    /// `instantiateEntry(String ID, Object[] entryWrapper, StringBuffer
    /// aliasReturn)`: given an entry object, instantiate it. Caller owns
    /// result. Return `None` on failure.
    ///
    /// Return a non-empty `alias_return` value if the ID points to an
    /// alias. We cannot instantiate it ourselves because the alias may
    /// contain filters or compounds, which we do not understand. Caller
    /// should make `alias_return` empty before calling.
    ///
    /// The entry object is assumed to reside in the dynamic store. It may
    /// be modified.
    fn instantiate_entry(
        &self,
        id: &[u16],
        entry_wrapper: &EntryWrapper,
        alias_return: &mut Vec<u16>,
    ) -> Result<Option<Arc<dyn Transliterator>>> {
        // We actually modify the entry object in some cases.  If it
        // is a string, we may partially parse it and turn it into a
        // more processed precursor.  This makes the next
        // instantiation faster and allows sharing of immutable
        // components like the RuleBasedTransliterator.Data objects.
        // For this reason, the entry object is an Object[] of length
        // 1.

        loop {
            let entry = lock_entry(entry_wrapper).clone();
            let (resource, direction) = match entry {
                Entry::Data(data) => {
                    return Ok(Some(Arc::new(RuleBasedTransliterator::new(
                        id.to_vec(),
                        data,
                        None,
                    )?)));
                }
                Entry::Class(class) => return Ok(Some(class())),
                Entry::Alias(alias) => {
                    alias_return.extend_from_slice(&alias);
                    return Ok(None);
                }
                Entry::Factory(factory) => return factory.get_instance(id),
                Entry::CompoundRbt(compound) => return Ok(Some(compound.get_instance()?)),
                Entry::Transliterator(t) => {
                    if let Some(temp) = t.as_any().downcast_ref::<AnyTransliterator>() {
                        return Ok(Some(temp.safe_clone()?));
                    } else if let Some(temp) = t.as_any().downcast_ref::<RuleBasedTransliterator>()
                    {
                        return Ok(Some(temp.safe_clone()?));
                    } else if let Some(temp) = t.as_any().downcast_ref::<CompoundTransliterator>() {
                        return Ok(Some(temp.safe_clone()?));
                    }
                    return Ok(Some(t));
                }
                // At this point entry type must be either RULES_FORWARD or
                // RULES_REVERSE.  We process the rule data into a
                // TransliteratorRuleData object, and possibly also into an
                // .id header and/or footer.  Then we modify the registry with
                // the parsed data and retry.
                Entry::Resource(re) => (re.resource, re.direction),
                Entry::Locale(le) => (le.rule, le.direction),
            };

            let mut parser = TransliteratorParser::new();
            parser.parse(&resource, direction)?;

            // Reset entry to something that we process at the
            // top of the loop, then loop back to the top.  As long as we
            // do this, we only loop through twice at most.
            // NOTE: The logic here matches that in
            // Transliterator.createFromRules().
            let new_entry = if parser.id_block_vector.is_empty() && parser.data_vector.is_empty() {
                // No idBlock, no data -- this is just an
                // alias for Null
                Entry::Alias(units(NullTransliterator::ID))
            } else if parser.id_block_vector.is_empty() && parser.data_vector.len() == 1 {
                // No idBlock, data != 0 -- this is an
                // ordinary RBT_DATA
                Entry::Data(parser.data_vector[0].clone())
            } else if parser.id_block_vector.len() == 1 && parser.data_vector.is_empty() {
                // idBlock, no data -- this is an alias.  The ID has
                // been munged from reverse into forward mode, if
                // necessary, so instantiate the ID in the forward
                // direction.
                match &parser.compound_filter {
                    Some(compound_filter) => {
                        let mut s = compound_filter.to_pattern(false);
                        s.push(b';' as u16);
                        s.extend_from_slice(&parser.id_block_vector[0]);
                        Entry::Alias(s)
                    }
                    None => Entry::Alias(parser.id_block_vector[0].clone()),
                }
            } else {
                Entry::CompoundRbt(Arc::new(CompoundRBTEntry {
                    id: id.to_vec(),
                    id_block_vector: parser.id_block_vector.clone(),
                    data_vector: parser.data_vector.clone(),
                    compound_filter: parser.compound_filter.clone(),
                }))
            };
            *lock_entry(entry_wrapper) = new_entry;
        }
    }
}

impl Default for TransliteratorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
