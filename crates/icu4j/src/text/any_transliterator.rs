// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.AnyTransliterator`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;

use crate::jdk::{to_rust_string, units};
use crate::lang::u_script::UScript;
use crate::text::compound_transliterator::CompoundTransliterator;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    self, register_instance_in, register_special_inverse, transliterator_boilerplate, Position,
    Transliterator, TransliteratorBase, FORWARD,
};
use crate::text::transliterator_id_parser::{equals_ignore_case, TransliteratorIDParser};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_set::UnicodeSet;
use crate::util::u_resource_bundle::is_missing_resource;

//------------------------------------------------------------
// Constants

const TARGET_SEP: u16 = b'-' as u16;
const VARIANT_SEP: u16 = b'/' as u16;
const ANY: &str = "Any";
const NULL_ID: &str = "Null";
const LATIN_PIVOT: &str = "-Latin;Latin-";

/// `WidthFix.INSTANCE`: the transliterator that maps narrow and wide forms
/// to their compatibility decompositions (the nested class `WidthFix`
/// holds it for lazy initialization).
static WIDTH_FIX: LazyLock<Arc<dyn Transliterator>> =
    LazyLock::new(
        || match transliterator::get_instance(&units("[[:dt=Nar:][:dt=Wide:]] nfkd")) {
            Ok(t) => t,
            Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
        },
    );

/// A transliterator named Any-T or Any-T/V, where T is the target and V is
/// the optional variant. The target T is a script. This transliterator
/// transliterates each run of characters of a script S (other than Common
/// and Inherited) to the target script with the transliterator S-T/V,
/// pivoting through Latin when S-T/V does not exist.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.AnyTransliterator`.
#[derive(Debug)]
pub struct AnyTransliterator {
    base: TransliteratorBase,
    /// Cache mapping UScriptCode values to Transliterator*.
    /// (`ConcurrentHashMap<Integer, Transliterator>`, shared by
    /// `safeClone`.)
    cache: Arc<Mutex<HashMap<i32, Arc<dyn Transliterator>>>>,
    /// The target or target/variant string.
    target: Vec<u16>,
    /// The target script code. Never USCRIPT_INVALID_CODE.
    target_script: i32,
}

impl AnyTransliterator {
    /// The private `AnyTransliterator(String id, String theTarget, String
    /// theVariant, int theTargetScript)`.
    fn new_private(
        id: Vec<u16>,
        the_target: &[u16],
        the_variant: &[u16],
        the_target_script: i32,
    ) -> Self {
        let mut target = the_target.to_vec();
        if !the_variant.is_empty() {
            target.push(VARIANT_SEP);
            target.extend_from_slice(the_variant);
        }
        Self {
            base: TransliteratorBase::with_id(id),
            target_script: the_target_script,
            cache: Arc::new(Mutex::new(HashMap::new())),
            target,
        }
    }

    /// `AnyTransliterator(String id, UnicodeFilter filter, String target2,
    /// int targetScript2, Transliterator widthFix2, ConcurrentHashMap
    /// cache2)`: the clone constructor (`widthFix2` is unused in Java).
    ///
    /// # Errors
    ///
    /// The errors of `setFilter`.
    pub(crate) fn with_cache(
        id: Vec<u16>,
        filter: Option<&dyn UnicodeFilter>,
        target2: Vec<u16>,
        target_script2: i32,
        _width_fix2: Option<Arc<dyn Transliterator>>,
        cache2: Arc<Mutex<HashMap<i32, Arc<dyn Transliterator>>>>,
    ) -> Result<Self> {
        Ok(Self {
            base: TransliteratorBase::new(id, filter)?,
            target_script: target_script2,
            cache: cache2,
            target: target2,
        })
    }

    /// `getTransliterator(int source)`: returns a transliterator from the
    /// given source to our target or target/variant. Returns `None` if the
    /// source is the same as our target script, or if the source is
    /// USCRIPT_INVALID_CODE. Returns `None` if no transliterator can be
    /// instantiated. Caches the result.
    fn get_transliterator(&self, source: i32) -> Result<Option<Arc<dyn Transliterator>>> {
        if source == self.target_script || source == UScript::INVALID_CODE {
            if self.is_wide(self.target_script) {
                return Ok(None);
            } else {
                return Ok(Some(WIDTH_FIX.clone()));
            }
        }

        let key = source;
        let mut t = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned();
        if t.is_none() {
            let source_name =
                units(&UScript::get_name(source)?.unwrap_or_else(|| "null".to_string()));
            let mut id = source_name.clone();
            id.push(TARGET_SEP);
            id.extend_from_slice(&self.target);

            // catch (RuntimeException e)
            t = transliterator::get_instance_dir(&id, FORWARD).ok();
            if t.is_none() {
                let mut id = source_name;
                id.extend_from_slice(&units(LATIN_PIVOT));
                id.extend_from_slice(&self.target);
                // catch (RuntimeException e)
                t = transliterator::get_instance_dir(&id, FORWARD).ok();
            }

            if let Some(found) = t.take() {
                let mut found = found;
                if !self.is_wide(self.target_script) {
                    let v: Vec<Arc<dyn Transliterator>> = vec![WIDTH_FIX.clone(), found];
                    found = Arc::new(CompoundTransliterator::new(v)?);
                }
                let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                // putIfAbsent
                let prev_cached_t = cache.get(&key).cloned();
                match prev_cached_t {
                    Some(prev) => found = prev,
                    None => {
                        cache.insert(key, found.clone());
                    }
                }
                t = Some(found);
            } else if !self.is_wide(self.target_script) {
                return Ok(Some(WIDTH_FIX.clone()));
            }
        }

        Ok(t)
    }

    /// `isWide(int script)`.
    fn is_wide(&self, script: i32) -> bool {
        script == UScript::BOPOMOFO
            || script == UScript::HAN
            || script == UScript::HANGUL
            || script == UScript::HIRAGANA
            || script == UScript::KATAKANA
    }

    /// `register()`: registers standard transliterators with the system.
    /// Called by Transliterator during initialization. Scan all current
    /// targets and register those that are scripts T as Any-T/V. Runs on
    /// the registry under construction (or the system registry for
    /// `registerAny()`).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        let mut seen: HashMap<Vec<u16>, HashSet<Vec<u16>>> = HashMap::new();

        for source in registry.get_available_sources() {
            let source = source?;
            // Ignore the "Any" source
            if equals_ignore_case(&source, &units(ANY)) {
                continue;
            }

            for target in registry.get_available_targets(&source) {
                let target = target?;
                // Get the script code for the target.  If not a script, ignore.
                let target_script = Self::script_name_to_code(&target)?;
                if target_script == UScript::INVALID_CODE {
                    continue;
                }

                let seen_variants = seen.entry(target.clone()).or_default();

                for variant in registry.get_available_variants(&source, &target) {
                    let variant = variant?;
                    // Only process each target/variant pair once
                    if seen_variants.contains(&variant) {
                        continue;
                    }
                    seen_variants.insert(variant.clone());

                    let id =
                        TransliteratorIDParser::stv_to_id(&units(ANY), &target, Some(&variant));
                    let trans =
                        AnyTransliterator::new_private(id, &target, &variant, target_script);
                    register_instance_in(registry, Arc::new(trans), true)?;
                    register_special_inverse(&target, &units(NULL_ID), false);
                }
            }
        }
        Ok(())
    }

    /// `scriptNameToCode(String name)`: return the script code for a given
    /// name, or UScript.INVALID_CODE if not found.
    fn script_name_to_code(name: &[u16]) -> Result<i32> {
        match UScript::get_code(&to_rust_string(name)) {
            Ok(codes) => Ok(codes.map_or(UScript::INVALID_CODE, |c| c[0])),
            Err(e) if is_missing_resource(&e) => Ok(UScript::INVALID_CODE),
            Err(e) => Err(e),
        }
    }

    /// `safeClone()`: temporary hack for registry problem. Needs to be
    /// replaced by better architecture.
    ///
    /// # Errors
    ///
    /// None in practice (the filter is a `UnicodeSet`).
    pub fn safe_clone(&self) -> Result<Arc<dyn Transliterator>> {
        let filter = self.base.get_filter().map(|f| UnicodeSet::from_set(&f));
        Ok(Arc::new(AnyTransliterator::with_cache(
            self.base.get_id(),
            filter.as_ref().map(|f| f as &dyn UnicodeFilter),
            self.target.clone(),
            self.target_script,
            None,
            self.cache.clone(),
        )?))
    }
}

impl Transliterator for AnyTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position pos, boolean
    /// isIncremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        pos: &mut Position,
        is_incremental: bool,
    ) -> Result<()> {
        let all_start = pos.start;
        let mut all_limit = pos.limit;

        let mut it = ScriptRunIterator::new(text, pos.context_start, pos.context_limit);

        while it.next()? {
            // Ignore runs in the ante context
            if it.limit <= all_start {
                continue;
            }

            // Try to instantiate transliterator from it.scriptCode to
            // our target or target/variant
            let Some(t) = self.get_transliterator(it.script_code)? else {
                // We have no transliterator.  Do nothing, but keep
                // pos.start up to date.
                pos.start = it.limit;
                continue;
            };

            // If the run end is before the transliteration limit, do
            // a non-incremental transliteration.  Otherwise do an
            // incremental one.
            let incremental = is_incremental && (it.limit >= all_limit);

            pos.start = all_start.max(it.start);
            pos.limit = all_limit.min(it.limit);
            let limit = pos.limit;
            t.filtered_transliterate(text, pos, incremental)?;
            let delta = pos.limit - limit;
            all_limit += delta;
            it.adjust_limit(delta);

            // We're done if we enter the post context
            if it.limit >= all_limit {
                break;
            }
        }

        // Restore limit.  pos.start is fine where the last transliterator
        // left it, or at the end of the last run.
        pos.limit = all_limit;
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(
        &self,
        input_filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        // Assume that it can modify any character to any character
        source_set.add_all(&my_filter)?;
        if my_filter.size() != 0 {
            target_set.add_all_range(0, 0x10FFFF)?;
        }
        Ok(())
    }
}

/// Returns a series of ranges corresponding to scripts. They will be of
/// the form:
///
/// ccccSScSSccccTTcTcccc - c = common, S = first script, T = second
/// |            |          - first run (start, limit)
///          |           |  - second run (start, limit)
///
/// That is, the runs will overlap. The reason for this is so that a
/// transliterator can consider common characters both before and after the
/// scripts.
///
/// Equivalent to the nested class `AnyTransliterator.ScriptRunIterator`.
struct ScriptRunIterator<'a> {
    text: &'a dyn Replaceable,
    text_start: i32,
    text_limit: i32,

    /// The code of the current run, valid after `next()` returns. May be
    /// UScript.INVALID_CODE if and only if the entire text is
    /// COMMON/INHERITED.
    script_code: i32,

    /// The start of the run, inclusive, valid after `next()` returns.
    start: i32,

    /// The end of the run, exclusive, valid after `next()` returns.
    limit: i32,
}

impl<'a> ScriptRunIterator<'a> {
    /// `ScriptRunIterator(Replaceable text, int start, int limit)`:
    /// constructs a run iterator over the given text from start (inclusive)
    /// to limit (exclusive).
    fn new(text: &'a dyn Replaceable, start: i32, limit: i32) -> Self {
        Self {
            text,
            text_start: start,
            text_limit: limit,
            script_code: UScript::INVALID_CODE,
            start: 0,
            limit: start,
        }
    }

    /// `next()`: returns true if there are any more runs. True is always
    /// returned at least once. Upon return, the caller should examine
    /// scriptCode, start, and limit.
    fn next(&mut self) -> Result<bool> {
        let mut ch;
        let mut s;

        self.script_code = UScript::INVALID_CODE; // don't know script yet
        self.start = self.limit;

        // Are we done?
        if self.start == self.text_limit {
            return Ok(false);
        }

        // Move start back to include adjacent COMMON or INHERITED
        // characters
        while self.start > self.text_start {
            ch = self.text.char32_at(self.start - 1); // look back
            s = UScript::get_script(ch)?;
            if s == UScript::COMMON || s == UScript::INHERITED {
                self.start -= 1;
            } else {
                break;
            }
        }

        // Move limit ahead to include COMMON, INHERITED, and characters
        // of the current script.
        while self.limit < self.text_limit {
            ch = self.text.char32_at(self.limit); // look ahead
            s = UScript::get_script(ch)?;
            if s != UScript::COMMON && s != UScript::INHERITED {
                if self.script_code == UScript::INVALID_CODE {
                    self.script_code = s;
                } else if s != self.script_code {
                    break;
                }
            }
            self.limit += 1;
        }

        // Return true even if the entire text is COMMON / INHERITED, in
        // which case scriptCode will be UScript.INVALID_CODE.
        Ok(true)
    }

    /// `adjustLimit(int delta)`: adjusts internal indices for a change in
    /// the limit index of the given delta. A positive delta means the
    /// limit has increased.
    fn adjust_limit(&mut self, delta: i32) {
        self.limit += delta;
        self.text_limit += delta;
    }
}
