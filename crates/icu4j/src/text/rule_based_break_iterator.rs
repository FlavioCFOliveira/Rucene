// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RuleBasedBreakIterator`, with its inner classes
//! `BreakCache` and `DictionaryCache`.

use std::any::Any;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::util::print_stream::{PrintStream, WriterPrintStream};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, StringCharacterIterator};

use crate::jdk::{is_high_surrogate, is_low_surrogate, ByteBuffer, InputStream, OutputStream};
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::breakiter::burmese_break_engine::BurmeseBreakEngine;
use crate::r#impl::breakiter::cjk_break_engine::CjkBreakEngine;
use crate::r#impl::breakiter::dictionary_break_engine::DequeI;
use crate::r#impl::breakiter::khmer_break_engine::KhmerBreakEngine;
use crate::r#impl::breakiter::language_break_engine::LanguageBreakEngine;
use crate::r#impl::breakiter::lao_break_engine::LaoBreakEngine;
use crate::r#impl::breakiter::lstm_break_engine::LSTMBreakEngine;
use crate::r#impl::breakiter::thai_break_engine::ThaiBreakEngine;
use crate::r#impl::breakiter::unhandled_break_engine::UnhandledBreakEngine;
use crate::r#impl::character_iteration::{self, next_trail32, set_index, DONE32};
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::icu_debug::ICUDebug;
use crate::r#impl::rbbi_data_wrapper::RBBIDataWrapper;
use crate::text::break_iterator::{BreakIterator, BreakIteratorLocales, DONE};
use crate::text::rbbi_rule_builder::RBBIRuleBuilder;
use crate::text::utf16::UTF16;
use crate::util::code_point_map::CodePointMap;
use crate::util::u_resource_bundle::is_missing_resource;

/// `START_STATE`: the state number of the starting state.
const START_STATE: i32 = 1;
/// `STOP_STATE`: the state-transition value indicating "stop".
const STOP_STATE: i32 = 0;

// Constants for handleNext() state machine.
/// `RBBI_START`.
const RBBI_START: i32 = 0;
/// `RBBI_RUN`.
const RBBI_RUN: i32 = 1;
/// `RBBI_END`.
const RBBI_END: i32 = 2;

/// `RBBI_DEBUG_ARG`: debugging flag.
const RBBI_DEBUG_ARG: &str = "rbbi";

/// `TRACE`: `ICUDebug.enabled("rbbi") && ICUDebug.value("rbbi").indexOf("trace") >= 0`.
static TRACE: LazyLock<bool> = LazyLock::new(|| {
    ICUDebug::enabled_arg(RBBI_DEBUG_ARG) && ICUDebug::value(RBBI_DEBUG_ARG).contains("trace")
});

/// `fDebugEnv`: the value of the `rbbi` debug argument, `null` when
/// debugging is off.
static F_DEBUG_ENV: LazyLock<Option<String>> = LazyLock::new(|| {
    if ICUDebug::enabled_arg(RBBI_DEBUG_ARG) {
        Some(ICUDebug::value(RBBI_DEBUG_ARG))
    } else {
        None
    }
});

/// `fDebugEnv` (public static, internal use): debugging flags, `None` when
/// off.
pub fn f_debug_env() -> Option<&'static str> {
    F_DEBUG_ENV.as_deref()
}

/// `gUnhandledBreakEngine`: the engine for characters no other engine
/// handles. It is shared among all break iterators.
static G_UNHANDLED_BREAK_ENGINE: LazyLock<Arc<UnhandledBreakEngine>> =
    LazyLock::new(|| Arc::new(UnhandledBreakEngine::new()));

/// `gAllBreakEngines`: the language break engines created by any break
/// iterator; the `ConcurrentLinkedQueue` and its `synchronized` creation
/// section are one lock.
static G_ALL_BREAK_ENGINES: LazyLock<Mutex<Vec<Arc<dyn LanguageBreakEngine>>>> =
    LazyLock::new(|| {
        let unhandled: Arc<dyn LanguageBreakEngine> = G_UNHANDLED_BREAK_ENGINE.clone();
        Mutex::new(vec![unhandled])
    });

/// `java.io.ByteArrayOutputStream`: an output stream into a growing byte
/// array.
#[derive(Debug, Default)]
pub(crate) struct ByteArrayOutputStream {
    buf: Vec<u8>,
}

impl ByteArrayOutputStream {
    /// `new ByteArrayOutputStream()`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `toByteArray()`.
    pub(crate) fn to_byte_array(&self) -> Vec<u8> {
        self.buf.clone()
    }
}

impl OutputStream for ByteArrayOutputStream {
    fn write_byte(&mut self, b: i32) -> std::io::Result<()> {
        self.buf.push(b as u8);
        Ok(())
    }

    fn write(&mut self, b: &[u8], off: usize, len: usize) -> std::io::Result<()> {
        self.buf.extend_from_slice(&b[off..off + len]);
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn close(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn stream_to_string(&self) -> String {
        "java.io.ByteArrayOutputStream".to_string()
    }
}

/// `String.hashCode()` over UTF-16 units.
fn java_string_hash(s: &[u16]) -> i32 {
    s.iter()
        .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// `CharacterIterator.equals` for the iterators the port can compare: same
/// bounds, index and text.
///
/// **Divergence.** Java calls the iterator's own `equals`
/// (`RuleBasedBreakIterator.java`, `equals`): `StringCharacterIterator`
/// compares text, bounds and index and other classes compare identity; the
/// crate's `CharacterIterator` trait has no `equals`, so the port compares
/// what `StringCharacterIterator.equals` compares for every iterator.
fn character_iterator_equals(a: &dyn CharacterIterator, b: &dyn CharacterIterator) -> bool {
    if a.get_begin_index() != b.get_begin_index()
        || a.get_end_index() != b.get_end_index()
        || a.get_index() != b.get_index()
    {
        return false;
    }
    let mut ca = a.clone_box();
    let mut cb = b.clone_box();
    let mut x = ca.first();
    let mut y = cb.first();
    for _ in a.get_begin_index()..a.get_end_index() {
        if x != y {
            return false;
        }
        x = ca.next();
        y = cb.next();
    }
    true
}

/// Rule Based Break Iterator: the break iterator over compiled rules, with
/// dictionary engines for scripts written without spaces.
///
/// Equivalent to the class `com.ibm.icu.text.RuleBasedBreakIterator`.
///
/// **Divergence.** The inner classes `BreakCache` and `DictionaryCache`
/// read and write the outer iterator (`fText`, `fPosition`, `handleNext`);
/// their state is held in [`BreakCache`] and [`DictionaryCache`], and their
/// methods that reach the outer instance are methods of this iterator named
/// `bc_*`/`dc_*`, with the same bodies.
#[derive(Debug)]
pub struct RuleBasedBreakIterator {
    /// `fText`: the character iterator through which this BreakIterator
    /// accesses the text.
    f_text: Option<Box<dyn CharacterIterator>>,
    /// `fRData`: the rule data for this BreakIterator instance. Not
    /// intended for public use. Declared public for testing purposes only.
    pub f_r_data: Arc<RBBIDataWrapper>,
    /// `fPosition`: the iteration state - current position, rule status for
    /// the current position, and whether the iterator ran off the end,
    /// yielding UBRK_DONE.
    f_position: i32,
    /// `fRuleStatusIndex`: index of the rule status value in the rule
    /// status table.
    f_rule_status_index: i32,
    /// `fDone`: true when iteration has run off the end, and iterator
    /// functions should return UBRK_DONE.
    f_done: bool,
    /// `fLookAheadMatches`: array of look-ahead tentative results.
    f_look_ahead_matches: Vec<i32>,
    /// `fBreakCache`: cache of previously determined boundary positions.
    f_break_cache: BreakCache,
    /// `fPhraseBreaking`: flag used to indicate that phrase breaking is
    /// required.
    f_phrase_breaking: bool,
    /// `fDictionaryCharCount`: counter for the number of characters
    /// encountered with the "dictionary" flag set.
    f_dictionary_char_count: i32,
    /// `fDictionaryCache`.
    f_dictionary_cache: DictionaryCache,
    /// `validLocale`/`actualLocale` of `BreakIterator`.
    locales: BreakIteratorLocales,
}

impl RuleBasedBreakIterator {
    /// `RuleBasedBreakIterator()` (private) over `data`:
    /// `fDictionaryCharCount = 0`, an empty `StringCharacterIterator`.
    fn with_data(data: RBBIDataWrapper) -> Self {
        let la = data.f_table().f_look_ahead_results_size;
        Self {
            f_text: Some(Box::new(StringCharacterIterator::new(Vec::new()))),
            f_r_data: Arc::new(data),
            f_position: 0,
            f_rule_status_index: 0,
            f_done: false,
            f_look_ahead_matches: vec![0; la.max(0) as usize],
            f_break_cache: BreakCache::new(),
            f_phrase_breaking: false,
            f_dictionary_char_count: 0,
            f_dictionary_cache: DictionaryCache::new(),
            locales: BreakIteratorLocales::default(),
        }
    }

    /// `getInstanceFromCompiledRules(InputStream is)`: an iterator over
    /// precompiled binary rule data read from `is`, which is closed.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream or the data.
    pub fn get_instance_from_compiled_rules_stream(
        is: &mut dyn InputStream,
    ) -> Result<RuleBasedBreakIterator> {
        let data = RBBIDataWrapper::get(
            ICUBinary::get_byte_buffer_from_input_stream_and_close_stream(is)?,
        )?;
        Ok(Self::with_data(data))
    }

    /// `getInstanceFromCompiledRules(ByteBuffer bytes, boolean
    /// phraseBreaking)` (package private).
    ///
    /// # Errors
    ///
    /// The `IOException` of the data.
    pub(crate) fn get_instance_from_compiled_rules_phrase(
        bytes: ByteBuffer,
        phrase_breaking: bool,
    ) -> Result<RuleBasedBreakIterator> {
        let mut instance = Self::get_instance_from_compiled_rules(bytes)?;
        instance.f_phrase_breaking = phrase_breaking;
        Ok(instance)
    }

    /// `getInstanceFromCompiledRules(ByteBuffer bytes)` (internal API): an
    /// iterator over precompiled binary rule data.
    ///
    /// # Errors
    ///
    /// The `IOException` of the data.
    pub fn get_instance_from_compiled_rules(bytes: ByteBuffer) -> Result<RuleBasedBreakIterator> {
        Ok(Self::with_data(RBBIDataWrapper::get(bytes)?))
    }

    /// `RuleBasedBreakIterator(String rules)`: compiles `rules`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a rule syntax error; an internal
    /// `IOException` becomes `RuntimeException("RuleBasedBreakIterator rule
    /// compilation internal error: " + message)`.
    pub fn new(rules: &[u16]) -> Result<RuleBasedBreakIterator> {
        let compiled = (|| -> Result<RBBIDataWrapper> {
            let mut rule_os = ByteArrayOutputStream::new();
            Self::compile_rules(rules, &mut rule_os)?;
            RBBIDataWrapper::get(ByteBuffer::wrap(rule_os.to_byte_array()))
        })();
        match compiled {
            Ok(data) => Ok(Self::with_data(data)),
            Err(e) if e.is_io_exception() => {
                //CLOVER:OFF
                // An IO exception can only arrive here if there is a bug in the RBBI Rule compiler,
                //  with the compiler (incorrectly) producing truncated rule data.
                let message = e
                    .original_message()
                    .map(str::to_string)
                    .unwrap_or_else(|| e.to_string());
                Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    format!("RuleBasedBreakIterator rule compilation internal error: {message}"),
                ))
                //CLOVER:ON
            }
            Err(e) => Err(e),
        }
    }

    /// `equals(Object that)`: same rule source, equal texts and position.
    pub fn java_equals(&self, that: &dyn Any) -> bool {
        let Some(other) = that.downcast_ref::<RuleBasedBreakIterator>() else {
            return false;
        };
        if std::ptr::eq(self, other) {
            return true;
        }
        if !Arc::ptr_eq(&self.f_r_data, &other.f_r_data)
            && self.f_r_data.f_rule_source != other.f_r_data.f_rule_source
        {
            return false;
        }
        match (&self.f_text, &other.f_text) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                if !character_iterator_equals(a.as_ref(), b.as_ref()) {
                    return false;
                }
                self.f_position == other.f_position
            }
            _ => false,
        }
    }

    /// `toString()`: the description (rules) used to create this iterator.
    pub fn to_java_string(&self) -> Vec<u16> {
        self.f_r_data.f_rule_source.clone()
    }

    /// `hashCode()`: the hash of the rule source.
    pub fn hash_code(&self) -> i32 {
        java_string_hash(&self.f_r_data.f_rule_source)
    }

    /// `dump(java.io.PrintStream out)` (internal API): dumps the rule data;
    /// `System.out` for `None`.
    ///
    /// # Errors
    ///
    /// As `RBBIDataWrapper.dump`.
    pub fn dump(&self, out: Option<&dyn PrintStream>) -> Result<()> {
        match out {
            Some(out) => self.f_r_data.dump(out),
            None => self.f_r_data.dump(&WriterPrintStream::stdout()),
        }
    }

    /// `compileRules(String rules, OutputStream ruleBinary)` (internal API):
    /// compiles `rules` to binary rule data.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a syntax error; the `IOException` of
    /// the stream.
    pub fn compile_rules(rules: &[u16], rule_binary: &mut dyn OutputStream) -> Result<()> {
        RBBIRuleBuilder::compile_rules(rules, rule_binary)
    }

    /// `fText`, dereferenced as Java does.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the text was set to `null`.
    fn text(&mut self) -> &mut dyn CharacterIterator {
        self.f_text
            .as_deref_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fText"))
    }

    /// `checkOffset(int offset, CharacterIterator text)` (protected static).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("offset out of bounds")`.
    pub fn check_offset(offset: i32, text: &dyn CharacterIterator) -> Result<()> {
        if offset < text.get_begin_index() || offset > text.get_end_index() {
            return Err(LuceneError::illegal_argument("offset out of bounds"));
        }
        Ok(())
    }

    /// `getLanguageBreakEngine(int c)`: the engine for dictionary character
    /// `c`, creating it the first time a character of its script is met.
    fn get_language_break_engine(&self, c: i32) -> Option<Arc<dyn LanguageBreakEngine>> {
        // We have a dictionary character.
        // Does an already instantiated break engine handle it?
        // First read without synchronization, which could lead to a new language
        // break engine being added and we didn't go over it.
        // In the port, both passes run under the one lock.
        let mut engines = G_ALL_BREAK_ENGINES
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for candidate in engines.iter() {
            if candidate.handles(c) {
                return Some(candidate.clone());
            }
        }

        // Another break iterator may have instantiated the desired engine.
        for candidate in engines.iter() {
            if candidate.handles(c) {
                return Some(candidate.clone());
            }
        }

        // The global list doesn't have an existing engine, build one.
        let mut script = UCharacter::get_int_property_value(c, UProperty::SCRIPT);
        if script == UScript::KATAKANA || script == UScript::HIRAGANA {
            // Katakana, Hiragana and Han are handled by the same dictionary engine.
            // Fold them together for mapping from script -> engine.
            script = UScript::HAN;
        }

        let unhandled: Arc<dyn LanguageBreakEngine> = G_UNHANDLED_BREAK_ENGINE.clone();
        let wrap = |r: Result<Arc<dyn LanguageBreakEngine>>| r.map(Some);
        let created: Result<Option<Arc<dyn LanguageBreakEngine>>> = match script {
            s if s == UScript::THAI => {
                match LSTMBreakEngine::create_data_for_script(script)
                    .and_then(|data| LSTMBreakEngine::create(script, data))
                {
                    Ok(e) => Ok(Some(Arc::new(e) as Arc<dyn LanguageBreakEngine>)),
                    Err(e) if is_missing_resource(&e) => wrap(
                        ThaiBreakEngine::new().map(|e| Arc::new(e) as Arc<dyn LanguageBreakEngine>),
                    ),
                    Err(e) => Err(e),
                }
            }
            s if s == UScript::LAO => {
                wrap(LaoBreakEngine::new().map(|e| Arc::new(e) as Arc<dyn LanguageBreakEngine>))
            }
            s if s == UScript::MYANMAR => {
                match LSTMBreakEngine::create_data_for_script(script)
                    .and_then(|data| LSTMBreakEngine::create(script, data))
                {
                    Ok(e) => Ok(Some(Arc::new(e) as Arc<dyn LanguageBreakEngine>)),
                    Err(e) if is_missing_resource(&e) => wrap(
                        BurmeseBreakEngine::new()
                            .map(|e| Arc::new(e) as Arc<dyn LanguageBreakEngine>),
                    ),
                    Err(e) => Err(e),
                }
            }
            s if s == UScript::KHMER => {
                wrap(KhmerBreakEngine::new().map(|e| Arc::new(e) as Arc<dyn LanguageBreakEngine>))
            }
            s if s == UScript::HAN => wrap(
                CjkBreakEngine::new(false).map(|e| Arc::new(e) as Arc<dyn LanguageBreakEngine>),
            ),
            s if s == UScript::HANGUL => {
                wrap(CjkBreakEngine::new(true).map(|e| Arc::new(e) as Arc<dyn LanguageBreakEngine>))
            }
            _ => {
                G_UNHANDLED_BREAK_ENGINE.handle_char(c);
                Ok(Some(unhandled.clone()))
            }
        };
        // `catch (IOException e) { eng = null; }`; any other exception
        // propagates out of the (infallible) iteration methods.
        let eng = match created {
            Ok(eng) => eng,
            Err(e) if e.is_io_exception() => None,
            Err(e) => panic!("{e}"),
        };

        if let Some(eng) = &eng {
            if !std::ptr::addr_eq(Arc::as_ptr(eng), Arc::as_ptr(&unhandled)) {
                engines.push(eng.clone());
            }
        }
        eng
    }

    /// `handleNext()`: the forward-direction implementation; runs the state
    /// machine from `fPosition` and returns the next boundary, or [`DONE`]
    /// at the end of the text. Sets `fRuleStatusIndex` and
    /// `fDictionaryCharCount`.
    fn handle_next(&mut self) -> i32 {
        if *TRACE {
            println!("Handle Next   pos      char  state category");
        }

        // handleNext always sets the break tag value.
        // Set the default for it.
        self.f_rule_status_index = 0;
        self.f_dictionary_char_count = 0;

        // caches for quicker access
        let rdata = self.f_r_data.clone();
        let trie = &rdata.f_trie;

        let state_table = &rdata.f_table().f_table;
        let initial_position = self.f_position;
        let text = self
            .f_text
            .as_deref_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fText"));
        set_index(text, initial_position);
        let mut result = initial_position;

        // Set up the starting char
        let mut c = text.current() as i32;
        if c >= UTF16::LEAD_SURROGATE_MIN_VALUE {
            c = next_trail32(text, c);
            if c == DONE32 {
                self.f_done = true;
                return DONE;
            }
        }

        // Set the initial state for the state machine
        let mut state = START_STATE;
        let mut row = rdata.get_row_index(state);
        let mut category: i16 = 3;
        let flags_state = rdata.f_table().f_flags;
        let dict_start = rdata.f_table().f_dict_categories_start;
        let mut mode = RBBI_RUN;
        if (flags_state & RBBIDataWrapper::RBBI_BOF_REQUIRED) != 0 {
            category = 2;
            mode = RBBI_START;
            if *TRACE {
                print!(
                    "            {}",
                    RBBIDataWrapper::int_to_string(text.get_index(), 5)
                );
                print!("{}", RBBIDataWrapper::int_to_hex_string(c, 10));
                println!(
                    "{}{}",
                    RBBIDataWrapper::int_to_string(state, 7),
                    RBBIDataWrapper::int_to_string(category as i32, 6)
                );
            }
        }

        // loop until we reach the end of the text or transition to state 0
        while state != STOP_STATE {
            if c == DONE32 {
                // Reached end of input string.
                if mode == RBBI_END {
                    // We have already run the loop one last time with the
                    // character set to the pseudo {eof} value. Now it is time
                    // to unconditionally bail out.
                    break;
                }
                // Run the loop one last time with the fake end-of-input character category
                mode = RBBI_END;
                category = 1;
            } else if mode == RBBI_RUN {
                // Get the char category.  An incoming category of 1 or 2 mens that
                //      we are preset for doing the beginning or end of input, and
                //      that we shouldn't get a category from an actual text input character.
                //

                // look up the current character's character category, which tells us
                // which column in the state table to look at.
                //
                category = trie.get(c) as i16;

                // Check for categories that require word dictionary handling.
                if category as i32 >= dict_start {
                    self.f_dictionary_char_count += 1;
                }

                if *TRACE {
                    print!(
                        "            {}",
                        RBBIDataWrapper::int_to_string(text.get_index(), 5)
                    );
                    print!("{}", RBBIDataWrapper::int_to_hex_string(c, 10));
                    println!(
                        "{}{}",
                        RBBIDataWrapper::int_to_string(state, 7),
                        RBBIDataWrapper::int_to_string(category as i32, 6)
                    );
                }

                // Advance to the next character.
                // If this is a beginning-of-input loop iteration, don't advance.
                //    The next iteration will be processing the first real input character.
                c = text.next() as i32;
                if c >= UTF16::LEAD_SURROGATE_MIN_VALUE {
                    c = next_trail32(text, c);
                }
            } else {
                mode = RBBI_RUN;
            }

            // look up a state transition in the state table
            state =
                state_table[(row + RBBIDataWrapper::NEXTSTATES + category as i32) as usize] as i32;
            row = rdata.get_row_index(state);
            let accepting = state_table[(row + RBBIDataWrapper::ACCEPTING) as usize] as i32;
            if accepting == RBBIDataWrapper::ACCEPTING_UNCONDITIONAL {
                // Match found, common case
                result = text.get_index();
                if (UTF16::SUPPLEMENTARY_MIN_VALUE..=UTF16::CODEPOINT_MAX_VALUE).contains(&c) {
                    // The iterator has been left in the middle of a surrogate pair.
                    // We want the start of it.
                    result -= 1;
                }

                //  Remember the break status (tag) values.
                self.f_rule_status_index =
                    state_table[(row + RBBIDataWrapper::TAGSIDX) as usize] as i32;
            } else if accepting > RBBIDataWrapper::ACCEPTING_UNCONDITIONAL {
                // Lookahead match is completed
                let lookahead_result = self.f_look_ahead_matches[accepting as usize];
                if lookahead_result >= 0 {
                    self.f_rule_status_index =
                        state_table[(row + RBBIDataWrapper::TAGSIDX) as usize] as i32;
                    self.f_position = lookahead_result;
                    return lookahead_result;
                }
            }

            // If we are at the position of the '/' in a look-ahead (hard break) rule;
            // record the current position, to be returned later, if the full rule matches.
            // TODO: Move this check before the previous check of fAccepting.
            //       This would enable hard-break rules with no following context.
            //       But there are line break test failures when trying this. Investigate.
            //       Issue ICU-20837
            let rule = state_table[(row + RBBIDataWrapper::LOOKAHEAD) as usize] as i32;
            if rule != 0 {
                let mut pos = text.get_index();
                if (UTF16::SUPPLEMENTARY_MIN_VALUE..=UTF16::CODEPOINT_MAX_VALUE).contains(&c) {
                    // The iterator has been left in the middle of a surrogate pair.
                    // We want the beginning  of it.
                    pos -= 1;
                }
                self.f_look_ahead_matches[rule as usize] = pos;
            }
        } // End of state machine main loop

        // The state machine is done.  Check whether it found a match...

        // If the iterator failed to advance in the match engine force it ahead by one.
        // This indicates a defect in the break rules, which should always match
        // at least one character.

        if result == initial_position {
            if *TRACE {
                println!("Iterator did not move. Advancing by 1.");
            }
            set_index(text, initial_position);
            character_iteration::next32(text);
            result = text.get_index();
            self.f_rule_status_index = 0;
        }

        // Leave the iterator at our result position.
        //   (we may have advanced beyond the last accepting position chasing after
        //    longer matches that never completed.)
        self.f_position = result;

        if *TRACE {
            println!("result = {result}");
        }
        result
    }

    /// `handleSafePrevious(int fromPosition)`: runs the safe reverse table
    /// from `from_position` and returns a safe position to start forward
    /// iteration from, or [`DONE`] at the start of the text.
    fn handle_safe_previous(&mut self, from_position: i32) -> i32 {
        let mut state: i32;
        let mut category: i16;

        // caches for quicker access
        let rdata = self.f_r_data.clone();
        let trie = &rdata.f_trie;
        let state_table = &rdata.r_table().f_table;
        let text = self.text();
        Self::ci_set_index32(text, from_position);
        if *TRACE {
            print!("Handle Previous   pos   char  state category");
        }

        // if we're already at the start of the text, return DONE.
        if text.get_index() == text.get_begin_index() {
            return DONE;
        }

        //  Set the initial state for the state machine
        let mut c = character_iteration::previous32(text);
        state = START_STATE;
        let mut row = rdata.get_row_index(state);

        // loop until we reach the start of the text or transition to state 0
        //
        while c != DONE32 {
            // look up the current character's character category, which tells us
            // which column in the state table to look at.
            //
            //  Off the dictionary flag bit. For reverse iteration it is not used.
            category = trie.get(c) as i16;
            if *TRACE {
                print!(
                    "            {}",
                    RBBIDataWrapper::int_to_string(text.get_index(), 5)
                );
                print!("{}", RBBIDataWrapper::int_to_hex_string(c, 10));
                println!(
                    "{}{}",
                    RBBIDataWrapper::int_to_string(state, 7),
                    RBBIDataWrapper::int_to_string(category as i32, 6)
                );
            }

            // State Transition - move machine to its next state
            //
            debug_assert!((category as i32) < rdata.f_header.f_cat_count);
            state =
                state_table[(row + RBBIDataWrapper::NEXTSTATES + category as i32) as usize] as i32;
            row = rdata.get_row_index(state);

            if state == STOP_STATE {
                // This is the normal exit from the lookup state machine.
                // Transition to state zero means we have found a safe point.
                break;
            }
            c = character_iteration::previous32(text);
        }

        // The state machine is done.
        let result = text.get_index();
        if *TRACE {
            println!("result = {result}");
        }
        result
    }

    /// `CISetIndex32(CharacterIterator ci, int index)`: sets the index,
    /// clamped to the text and moved back off a trail surrogate preceded by
    /// a lead; returns the index set.
    fn ci_set_index32(ci: &mut dyn CharacterIterator, index: i32) -> i32 {
        if index <= ci.get_begin_index() {
            ci.first();
        } else if index >= ci.get_end_index() {
            let end = ci.get_end_index();
            set_index(ci, end);
        } else if is_low_surrogate(set_index(ci, index)) && !is_high_surrogate(ci.previous()) {
            ci.next();
        }
        ci.get_index()
    }

    // -----------------------------------------------------------------
    // DictionaryCache methods that reach the outer instance
    // -----------------------------------------------------------------

    /// `DictionaryCache.populateDictionary(int startPos, int endPos, int
    /// firstRuleStatus, int otherRuleStatus)`: runs the dictionary engines
    /// over a segment and fills the dictionary cache with the breaks.
    fn dc_populate_dictionary(
        &mut self,
        start_pos: i32,
        end_pos: i32,
        first_rule_status: i32,
        other_rule_status: i32,
    ) {
        if (end_pos - start_pos) <= 1 {
            return;
        }

        self.f_dictionary_cache.reset();
        self.f_dictionary_cache.f_first_rule_status_index = first_rule_status;
        self.f_dictionary_cache.f_other_rule_status_index = other_rule_status;

        let range_start = start_pos;
        let range_end = end_pos;

        let mut category: i32;
        let mut current: i32;
        let mut found_break_count = 0;

        // Loop through the text, looking for ranges of dictionary characters.
        // For each span, find the appropriate break engine, and ask it to find
        // any breaks within the span.

        let rdata = self.f_r_data.clone();
        set_index(self.text(), range_start);
        let mut c = character_iteration::current32(self.text());
        category = rdata.f_trie.get(c) as i16 as i32;
        let dict_start = rdata.f_table().f_dict_categories_start;

        loop {
            loop {
                current = self.text().get_index();
                if !(current < range_end && category < dict_start) {
                    break;
                }
                c = character_iteration::next32(self.text()); // pre-increment
                category = rdata.f_trie.get(c) as i16 as i32;
            }
            if current >= range_end {
                break;
            }

            // We now have a dictionary character. Get the appropriate language object
            // to deal with it.
            let lbe = self.get_language_break_engine(c);

            // Ask the language object if there are any breaks. It will add them to the cache and
            // leave the text pointer on the other side of its range, ready to search for the next one.
            if let Some(lbe) = lbe {
                let phrase = self.f_phrase_breaking;
                let text = self
                    .f_text
                    .as_deref_mut()
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException: fText"));
                found_break_count += lbe.find_breaks(
                    text,
                    range_start,
                    range_end,
                    &mut self.f_dictionary_cache.f_breaks,
                    phrase,
                );
            }

            // Reload the loop variables for the next go-round
            c = character_iteration::current32(self.text());
            category = rdata.f_trie.get(c) as i16 as i32;
        }

        // If we found breaks, ensure that the first and last entries are
        // the original starting and ending position. And initialize the
        // cache iteration position to the first entry.

        // System.out.printf("foundBreakCount = %d%n", foundBreakCount);
        let dc = &mut self.f_dictionary_cache;
        if found_break_count > 0 {
            debug_assert!(found_break_count == dc.f_breaks.size());
            if start_pos < dc.f_breaks.element_at(0) {
                // The dictionary did not place a boundary at the start of the segment of text.
                // Add one now. This should not commonly happen, but it would be easy for interactions
                // of the rules for dictionary segments and the break engine implementations to
                // inadvertently cause it. Cover it here, just in case.
                dc.f_breaks.offer(start_pos);
            }
            if end_pos > dc.f_breaks.peek() {
                dc.f_breaks.push(end_pos);
            }
            dc.f_position_in_cache = 0;
            dc.f_start = dc.f_breaks.element_at(0);
            dc.f_limit = dc.f_breaks.peek();
        } else {
            // there were no language-based breaks, even though the segment contained
            // dictionary characters. Subsequent attempts to fetch boundaries from the dictionary cache
            // for this range will fail, and the calling code will fall back to the rule based boundaries.
        }
    }

    // -----------------------------------------------------------------
    // BreakCache methods that reach the outer instance
    // -----------------------------------------------------------------

    /// `BreakCache.next()`: makes the following boundary the current one.
    fn bc_next(&mut self) {
        let bc = &mut self.f_break_cache;
        if bc.f_buf_idx == bc.f_end_buf_idx {
            self.f_done = !self.bc_populate_following();
            self.f_position = self.f_break_cache.f_text_idx;
            self.f_rule_status_index =
                self.f_break_cache.f_statuses[self.f_break_cache.f_buf_idx as usize] as i32;
        } else {
            bc.f_buf_idx = BreakCache::mod_chunk_size(bc.f_buf_idx + 1);
            bc.f_text_idx = bc.f_boundaries[bc.f_buf_idx as usize];
            self.f_position = bc.f_text_idx;
            self.f_rule_status_index = bc.f_statuses[bc.f_buf_idx as usize] as i32;
        }
    }

    /// `BreakCache.previous()`: makes the preceding boundary the current
    /// one.
    fn bc_previous(&mut self) {
        let initial_buf_idx = self.f_break_cache.f_buf_idx;
        if self.f_break_cache.f_buf_idx == self.f_break_cache.f_start_buf_idx {
            // At start of cache. Prepend to it.
            self.bc_populate_preceding();
        } else {
            // Cache already holds the next boundary
            let bc = &mut self.f_break_cache;
            bc.f_buf_idx = BreakCache::mod_chunk_size(bc.f_buf_idx - 1);
            bc.f_text_idx = bc.f_boundaries[bc.f_buf_idx as usize];
        }
        let bc = &self.f_break_cache;
        self.f_done = bc.f_buf_idx == initial_buf_idx;
        self.f_position = bc.f_text_idx;
        self.f_rule_status_index = bc.f_statuses[bc.f_buf_idx as usize] as i32;
    }

    /// `BreakCache.following(int startPos)`: moves to the first boundary
    /// after `start_pos`.
    fn bc_following(&mut self, start_pos: i32) {
        if start_pos == self.f_break_cache.f_text_idx
            || self.f_break_cache.seek(start_pos)
            || self.bc_populate_near(start_pos)
        {
            // startPos is in the cache. Do a next() from that position.
            // TODO: an awkward set of interactions with bi->fDone
            //       seek() does not clear it; it can't because of interactions with populateNear().
            //       next() does not clear it in the fast-path case, where everything matters. Maybe it should.
            //       So clear it here, for the case where seek() succeeded on an iterator that had previously run off the end.
            self.f_done = false;
            self.bc_next();
        }
    }

    /// `BreakCache.preceding(int startPos)`: moves to the last boundary
    /// before `start_pos`.
    fn bc_preceding(&mut self, start_pos: i32) {
        if start_pos == self.f_break_cache.f_text_idx
            || self.f_break_cache.seek(start_pos)
            || self.bc_populate_near(start_pos)
        {
            if start_pos == self.f_break_cache.f_text_idx {
                self.bc_previous();
            } else {
                // seek() leaves the BreakCache positioned at the preceding boundary
                //        if the requested position is between two boundaries.
                // current() pushes the BreakCache position out to the BreakIterator itself.
                debug_assert!(start_pos > self.f_break_cache.f_text_idx);
                self.bc_current();
            }
        }
    }

    /// `BreakCache.current()`: updates the iterator state from the cache's
    /// current position and returns it.
    fn bc_current(&mut self) -> i32 {
        let bc = &self.f_break_cache;
        self.f_position = bc.f_text_idx;
        self.f_rule_status_index = bc.f_statuses[bc.f_buf_idx as usize] as i32;
        self.f_done = false;
        bc.f_text_idx
    }

    /// `BreakCache.populateNear(int position)`: fills the cache with
    /// boundaries around `position`, making the boundary at or before it
    /// current; `false` on failure (should not happen).
    fn bc_populate_near(&mut self, position: i32) -> bool {
        debug_assert!(
            position < self.f_break_cache.f_boundaries[self.f_break_cache.f_start_buf_idx as usize]
                || position
                    > self.f_break_cache.f_boundaries[self.f_break_cache.f_end_buf_idx as usize]
        );

        // Add boundaries to the cache near the specified position.
        // The given position need not be a boundary itself.
        // The input position must be within the range of the text, and
        // on a code point boundary.
        // If the requested position is a break boundary, leave the iteration
        // position on it.
        // If the requested position is not a boundary, leave the iteration
        // position on the preceding boundary and include both the
        // preceding and following boundaries in the cache.
        // Additional boundaries, either preceding or following, may be added
        // to the cache as a side effect.

        // If the requested position is not near already cached positions, clear the existing cache,
        // find a near-by boundary and begin new cache contents there.

        // Threshold for a text position to be considered near to existing cache contents.
        // TODO: See issue ICU-22024 "perf tuning of Cache needed."
        //       This value is subject to change. See the ticket for more details.
        const CACHE_NEAR: i32 = 15;

        let start_of_text = self.text().get_begin_index();
        let mut a_boundary = -1;
        let mut rule_status_index = 0;
        let retain_cache;
        let start_b = self.f_break_cache.f_boundaries[self.f_break_cache.f_start_buf_idx as usize];
        let end_b = self.f_break_cache.f_boundaries[self.f_break_cache.f_end_buf_idx as usize];
        if (position > start_b - CACHE_NEAR) && position < (end_b + CACHE_NEAR) {
            // Requested position is near the existing cache. Retain it.
            retain_cache = true;
        } else if position <= start_of_text + CACHE_NEAR {
            // Requested position is near the start of the text. Fill cache from start, skipping
            // the need to find a safe point.
            retain_cache = false;
            a_boundary = start_of_text;
        } else {
            // Requested position is not near the existing cache.
            // Find a safe point to refill the cache from.
            let backup_pos = self.handle_safe_previous(position);

            let end_b = self.f_break_cache.f_boundaries[self.f_break_cache.f_end_buf_idx as usize];
            let start_b =
                self.f_break_cache.f_boundaries[self.f_break_cache.f_start_buf_idx as usize];
            if end_b < position && end_b >= (backup_pos - CACHE_NEAR) {
                // The requested position is beyond the end of the existing cache, but the
                // reverse rules produced a position near or before the cached region.
                // Retain the existing cache, and fill from the end of it.
                retain_cache = true;
            } else if backup_pos < start_of_text + CACHE_NEAR {
                // The safe reverse rules moved us to near the start of text.
                // Take that (index 0) as the backup boundary, avoiding the complication
                // (in the following block) of moving forward from the safe point to a known boundary.
                //
                // Retain the cache if it begins not too far from the requested position.
                a_boundary = start_of_text;
                retain_cache = start_b <= (position + CACHE_NEAR);
            } else {
                // The safe reverse rules produced a position that is neither near the existing
                // cache, nor near the start of text.
                // Advance to the boundary following.
                // There is a complication: the safe reverse rules identify pairs of code points
                // that are safe. If advancing from the safe point moves forwards by less than
                // two code points, we need to advance one more time to ensure that the boundary
                // is good, including a correct rules status value.
                //
                retain_cache = false;
                self.f_position = backup_pos;
                a_boundary = self.handle_next();
                if a_boundary == backup_pos + 1
                    || (a_boundary == backup_pos + 2
                        && is_high_surrogate(set_index(self.text(), backup_pos))
                        && is_low_surrogate(self.text().next()))
                {
                    // The initial handleNext() only advanced by a single code point. Go again.
                    // Safe rules identify safe pairs.
                    a_boundary = self.handle_next();
                }
                if a_boundary == DONE {
                    a_boundary = self.text().get_end_index();
                }
                rule_status_index = self.f_rule_status_index;
            }
        }

        if !retain_cache {
            debug_assert!(a_boundary != -1);
            self.f_break_cache.reset(a_boundary, rule_status_index); // Reset cache to hold aBoundary as a single starting point.
        }

        // Fill in boundaries between existing cache content and the new requested position.

        if self.f_break_cache.f_boundaries[self.f_break_cache.f_end_buf_idx as usize] < position {
            // The last position in the cache precedes the requested position.
            // Add following position(s) to the cache.
            while self.f_break_cache.f_boundaries[self.f_break_cache.f_end_buf_idx as usize]
                < position
            {
                if !self.bc_populate_following() {
                    debug_assert!(false);
                    return false;
                }
            }
            let bc = &mut self.f_break_cache;
            bc.f_buf_idx = bc.f_end_buf_idx; // Set iterator position to the end of the buffer.
            bc.f_text_idx = bc.f_boundaries[bc.f_buf_idx as usize]; // Required because populateFollowing may add extra boundaries.
            while self.f_break_cache.f_text_idx > position {
                // Move backwards to a position at or preceding the requested pos.
                self.bc_previous();
            }
            return true;
        }

        if self.f_break_cache.f_boundaries[self.f_break_cache.f_start_buf_idx as usize] > position {
            // The first position in the cache is beyond the requested position.
            // back up more until we get a boundary <= the requested position.
            while self.f_break_cache.f_boundaries[self.f_break_cache.f_start_buf_idx as usize]
                > position
            {
                self.bc_populate_preceding();
            }
            let bc = &mut self.f_break_cache;
            bc.f_buf_idx = bc.f_start_buf_idx; // Set iterator position to the start of the buffer.
            bc.f_text_idx = bc.f_boundaries[bc.f_buf_idx as usize]; // Required because populatePreceding may add extra boundaries.
            while self.f_break_cache.f_text_idx < position {
                // Move forwards to a position at or following the requested pos.
                self.bc_next();
            }
            if self.f_break_cache.f_text_idx > position {
                // If position is not itself a boundary, the next() loop above will overshoot.
                // Back up one, leaving cache position at the boundary preceding the requested position.
                self.bc_previous();
            }
            return true;
        }

        debug_assert!(self.f_break_cache.f_text_idx == position);
        true
    }

    /// `BreakCache.populateFollowing()`: adds boundaries following the
    /// cache's end, from the dictionary cache or the rules; `false` at the
    /// end of the text.
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn bc_populate_following(&mut self) -> bool {
        let from_position =
            self.f_break_cache.f_boundaries[self.f_break_cache.f_end_buf_idx as usize];
        let from_rule_status_idx =
            self.f_break_cache.f_statuses[self.f_break_cache.f_end_buf_idx as usize] as i32;
        let mut pos;
        let rule_status_idx;

        if self.f_dictionary_cache.following(from_position) {
            let (b, s) = (
                self.f_dictionary_cache.f_boundary,
                self.f_dictionary_cache.f_status_index,
            );
            self.f_break_cache
                .add_following(b, s, UPDATE_CACHE_POSITION);
            return true;
        }

        self.f_position = from_position;
        pos = self.handle_next();
        if pos == DONE {
            return false;
        }

        rule_status_idx = self.f_rule_status_index;
        if self.f_dictionary_char_count > 0 {
            // The text segment obtained from the rules includes dictionary characters.
            // Subdivide it, with subdivided results going into the dictionary cache.
            self.dc_populate_dictionary(from_position, pos, from_rule_status_idx, rule_status_idx);
            if self.f_dictionary_cache.following(from_position) {
                let (b, s) = (
                    self.f_dictionary_cache.f_boundary,
                    self.f_dictionary_cache.f_status_index,
                );
                self.f_break_cache
                    .add_following(b, s, UPDATE_CACHE_POSITION);
                return true;
                // TODO: may want to move a sizable chunk of the dictionary cache to the break cache at this point.
                //       But be careful with interactions with populateNear().
            }
        }

        // Rule based segment did not include dictionary characters.
        // Or, it did contain dictionary chars, but the dictionary segmenter didn't handle them,
        //    meaning that we didn't take the return, above.
        // Add its end point to the cache.
        self.f_break_cache
            .add_following(pos, rule_status_idx, UPDATE_CACHE_POSITION);

        // Add several non-dictionary boundaries at this point, to optimize straight forward iteration.
        //    (subsequent calls to BreakIterator::next() will take the fast path, getting cached results.
        //
        for _count in 0..6 {
            pos = self.handle_next();
            if pos == DONE || self.f_dictionary_char_count > 0 {
                break;
            }
            let s = self.f_rule_status_index;
            self.f_break_cache
                .add_following(pos, s, RETAIN_CACHE_POSITION);
        }
        true
    }

    /// `BreakCache.populatePreceding()`: adds boundaries preceding the
    /// cache's start; `false` at the start of the text.
    fn bc_populate_preceding(&mut self) -> bool {
        let text_begin = self.text().get_begin_index();
        let from_position =
            self.f_break_cache.f_boundaries[self.f_break_cache.f_start_buf_idx as usize];
        if from_position == text_begin {
            return false;
        }

        let mut position;
        let mut position_status_idx;

        if self.f_dictionary_cache.preceding(from_position) {
            let (b, s) = (
                self.f_dictionary_cache.f_boundary,
                self.f_dictionary_cache.f_status_index,
            );
            self.f_break_cache
                .add_preceding(b, s, UPDATE_CACHE_POSITION);
            return true;
        }

        let mut backup_position = from_position;

        // Find a boundary somewhere preceding the first already-cached boundary
        loop {
            backup_position -= 30;
            if backup_position <= text_begin {
                backup_position = text_begin;
            } else {
                backup_position = self.handle_safe_previous(backup_position);
            }
            if backup_position == DONE || backup_position == text_begin {
                position = text_begin;
                position_status_idx = 0;
            } else {
                // Advance to the boundary following the backup position.
                // There is a complication: the safe reverse rules identify pairs of code points
                // that are safe. If advancing from the safe point moves forwards by less than
                // two code points, we need to advance one more time to ensure that the boundary
                // is good, including a correct rules status value.
                //
                self.f_position = backup_position;
                position = self.handle_next();
                if position == backup_position + 1
                    || (position == backup_position + 2
                        && is_high_surrogate(set_index(self.text(), backup_position))
                        && is_low_surrogate(self.text().next()))
                {
                    // The initial handleNext() only advanced by a single code point. Go again.
                    // Safe rules identify safe pairs.
                    position = self.handle_next();
                }
                position_status_idx = self.f_rule_status_index;
            }
            if position < from_position {
                break;
            }
        }

        // Find boundaries between the one we just located and the first already-cached boundary
        // Put them in a side buffer, because we don't yet know where they will fall in the circular cache buffer.

        self.f_break_cache.f_side_buffer.remove_all_elements();
        self.f_break_cache.f_side_buffer.push(position);
        self.f_break_cache.f_side_buffer.push(position_status_idx);

        loop {
            let mut prev_position = position;
            self.f_position = position;
            let prev_status_idx = position_status_idx;
            position = self.handle_next();
            position_status_idx = self.f_rule_status_index;
            if position == DONE {
                break;
            }

            let mut segment_handled_by_dictionary = false;
            if self.f_dictionary_char_count != 0 {
                // Segment from the rules includes dictionary characters.
                // Subdivide it, with subdivided results going into the dictionary cache.
                let dict_seg_end_position = position;
                self.dc_populate_dictionary(
                    prev_position,
                    dict_seg_end_position,
                    prev_status_idx,
                    position_status_idx,
                );
                while self.f_dictionary_cache.following(prev_position) {
                    position = self.f_dictionary_cache.f_boundary;
                    position_status_idx = self.f_dictionary_cache.f_status_index;
                    segment_handled_by_dictionary = true;
                    debug_assert!(position > prev_position);
                    if position >= from_position {
                        break;
                    }
                    debug_assert!(position <= dict_seg_end_position);
                    self.f_break_cache.f_side_buffer.push(position);
                    self.f_break_cache.f_side_buffer.push(position_status_idx);
                    prev_position = position;
                }
                debug_assert!(position == dict_seg_end_position || position >= from_position);
            }

            if !segment_handled_by_dictionary && position < from_position {
                self.f_break_cache.f_side_buffer.push(position);
                self.f_break_cache.f_side_buffer.push(position_status_idx);
            }
            if position >= from_position {
                break;
            }
        }

        // Move boundaries from the side buffer to the main circular buffer.
        let mut success = false;
        let bc = &mut self.f_break_cache;
        if !bc.f_side_buffer.is_empty() {
            position_status_idx = bc.f_side_buffer.pop();
            position = bc.f_side_buffer.pop();
            bc.add_preceding(position, position_status_idx, UPDATE_CACHE_POSITION);
            success = true;
        }

        while !bc.f_side_buffer.is_empty() {
            position_status_idx = bc.f_side_buffer.pop();
            position = bc.f_side_buffer.pop();
            if !bc.add_preceding(position, position_status_idx, RETAIN_CACHE_POSITION) {
                // No space in circular buffer to hold a new preceding result while
                // also retaining the current cache (iteration) position.
                // Bailing out is safe; the cache will refill again if needed.
                break;
            }
        }
        success
    }
}

impl Clone for RuleBasedBreakIterator {
    /// `clone()`: a copy with its own text, look-ahead array and caches;
    /// the rule data is shared.
    fn clone(&self) -> Self {
        Self {
            f_text: self.f_text.as_ref().map(|t| t.clone_box()),
            f_r_data: self.f_r_data.clone(),
            f_position: self.f_position,
            f_rule_status_index: self.f_rule_status_index,
            f_done: self.f_done,
            f_look_ahead_matches: vec![
                0;
                self.f_r_data.f_table().f_look_ahead_results_size.max(0)
                    as usize
            ],
            f_break_cache: BreakCache::copy_of(&self.f_break_cache),
            f_phrase_breaking: self.f_phrase_breaking,
            f_dictionary_char_count: self.f_dictionary_char_count,
            f_dictionary_cache: self.f_dictionary_cache.clone(),
            locales: self.locales.clone(),
        }
    }
}

impl BreakIterator for RuleBasedBreakIterator {
    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(self.clone())
    }

    /// `first()`: sets the current iteration position to the beginning of
    /// the text.
    fn first(&mut self) -> i32 {
        let Some(text) = self.f_text.as_deref_mut() else {
            return DONE;
        };
        text.first();
        let start = text.get_index();
        if !self.f_break_cache.seek(start) {
            self.bc_populate_near(start);
        }
        self.bc_current();
        debug_assert!(self.f_position == start);
        self.f_position
    }

    /// `last()`: sets the current iteration position to the end of the
    /// text.
    fn last(&mut self) -> i32 {
        let Some(text) = self.f_text.as_deref() else {
            return DONE;
        };
        let end_pos = text.get_end_index();
        let end_should_be_boundary = self.is_boundary(end_pos).unwrap_or_else(|e| panic!("{e}")); // Has side effect of setting iterator position.
        debug_assert!(end_should_be_boundary);
        if self.f_position != end_pos {
            debug_assert!(self.f_position == end_pos);
        }
        end_pos
    }

    /// `next(int n)`: advances `n` boundaries (backwards when negative).
    fn next_n(&mut self, mut n: i32) -> i32 {
        let mut result = 0;
        if n > 0 {
            while n > 0 && result != DONE {
                result = self.next();
                n -= 1;
            }
        } else if n < 0 {
            while n < 0 && result != DONE {
                result = self.previous();
                n += 1;
            }
        } else {
            result = self.current();
        }
        result
    }

    /// `next()`: advances to the next boundary.
    fn next(&mut self) -> i32 {
        self.bc_next();
        if self.f_done {
            DONE
        } else {
            self.f_position
        }
    }

    /// `previous()`: moves to the previous boundary.
    fn previous(&mut self) -> i32 {
        self.bc_previous();
        if self.f_done {
            DONE
        } else {
            self.f_position
        }
    }

    /// `following(int startPos)`: the first boundary following
    /// `start_pos`.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the text is `null`.
    fn following(&mut self, mut start_pos: i32) -> i32 {
        // if the supplied position is before the beginning, return the
        // text's starting offset
        if start_pos < self.text().get_begin_index() {
            return self.first();
        }

        // Move requested offset to a code point start. It might be between a lead and trail surrogate.
        // Or it may be beyond the end of the text.
        start_pos = Self::ci_set_index32(self.text(), start_pos);
        self.bc_following(start_pos);
        if self.f_done {
            DONE
        } else {
            self.f_position
        }
    }

    /// `preceding(int offset)`: the last boundary preceding `offset`.
    fn preceding(&mut self, offset: i32) -> i32 {
        match self.f_text.as_deref() {
            None => return self.last(),
            Some(text) if offset > text.get_end_index() => return self.last(),
            Some(text) if offset < text.get_begin_index() => return self.first(),
            _ => {}
        }

        // Move requested offset to a code point start. It might be between a lead and trail surrogate.
        // int adjustedOffset = CISetIndex32(fText, offset);    // TODO: restore to match ICU4C behavior.
        let adjusted_offset = offset;
        self.bc_preceding(adjusted_offset);
        if self.f_done {
            DONE
        } else {
            self.f_position
        }
    }

    /// `isBoundary(int offset)`: whether `offset` is a boundary.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("offset out of bounds")` outside the text.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the text is `null`.
    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        // TODO: behavior difference with ICU4C, which considers out-of-range offsets
        //       to not be boundaries, and to not be errors.
        Self::check_offset(offset, self.text())?;

        // Adjust offset to be on a code point boundary and not beyond the end of the text.
        // Note that isBoundary() is always false for offsets that are not on code point boundaries.
        // But we still need the side effect of leaving iteration at the following boundary.
        let adjusted_offset = Self::ci_set_index32(self.text(), offset);

        let mut result = false;
        if self.f_break_cache.seek(adjusted_offset) || self.bc_populate_near(adjusted_offset) {
            result = self.bc_current() == offset;
        }

        if !result {
            // Not on a boundary. isBoundary() must leave iterator on the following boundary.
            // fBreakCache.seek(), above, left us on the preceding boundary, so advance one.
            self.next();
        }
        Ok(result)
    }

    /// `current()`: the current iteration position.
    fn current(&self) -> i32 {
        if self.f_text.is_some() {
            self.f_position
        } else {
            DONE
        }
    }

    /// `getRuleStatus()`: the status tag from the break rule that
    /// determined the boundary at the current iteration position; the
    /// numerically largest when several rules apply.
    fn get_rule_status(&self) -> i32 {
        //   Status records have this form:
        //           Count N         <--  fLastRuleStatusIndex points here.
        //           Status val 0
        //           Status val 1
        //              ...
        //           Status val N-1  <--  the value we need to return
        //   The status values are sorted in ascending order.
        //   This function returns the last (largest) of the array of status values.
        let idx = self.f_rule_status_index
            + self.f_r_data.f_status_table[self.f_rule_status_index as usize];
        self.f_r_data.f_status_table[idx as usize]
    }

    /// `getRuleStatusVec(int[] fillInArray)`: the status tags from the
    /// break rules that determined the current boundary; returns their
    /// number.
    fn get_rule_status_vec(&self, fill_in_array: Option<&mut [i32]>) -> i32 {
        let num_status_vals = self.f_r_data.f_status_table[self.f_rule_status_index as usize];
        if let Some(fill_in_array) = fill_in_array {
            let num_to_copy = num_status_vals.min(fill_in_array.len() as i32);
            for i in 0..num_to_copy {
                fill_in_array[i as usize] =
                    self.f_r_data.f_status_table[(self.f_rule_status_index + i + 1) as usize];
            }
        }
        num_status_vals
    }

    /// `getText()`: the `CharacterIterator` over the text being analyzed.
    fn get_text(&self) -> Option<&dyn CharacterIterator> {
        self.f_text.as_deref()
    }

    /// `setText(CharacterIterator newText)`: sets the iterator to analyze a
    /// new piece of text and moves to its first boundary.
    fn set_text(&mut self, new_text: Option<Box<dyn CharacterIterator>>) {
        match &new_text {
            Some(t) => self.f_break_cache.reset(t.get_begin_index(), 0),
            None => self.f_break_cache.reset_default(),
        }
        self.f_dictionary_cache.reset();
        self.f_text = new_text;
        self.first();
    }

    fn locales(&self) -> &BreakIteratorLocales {
        &self.locales
    }

    fn locales_mut(&mut self) -> &mut BreakIteratorLocales {
        &mut self.locales
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The cache of breaks found by the dictionary engines for the
/// dictionary segment being iterated.
///
/// Equivalent to the inner class `RuleBasedBreakIterator.DictionaryCache`
/// (its state and the methods that touch only it; `populateDictionary` is
/// an iterator method).
#[derive(Debug, Clone)]
pub struct DictionaryCache {
    /// `fBreaks`: a vector containing the boundaries.
    f_breaks: DequeI,
    /// `fPositionInCache`: index in fBreaks of the current position, or -1
    /// if the cache is not valid.
    f_position_in_cache: i32,
    /// `fStart`: text position of the first boundary in the cache.
    f_start: i32,
    /// `fLimit`: last boundary in the cache.
    f_limit: i32,
    /// `fFirstRuleStatusIndex`: rule status index for the first boundary.
    f_first_rule_status_index: i32,
    /// `fOtherRuleStatusIndex`: rule status index for the other boundaries.
    f_other_rule_status_index: i32,
    /// `fBoundary`: return value from following() and preceding().
    f_boundary: i32,
    /// `fStatusIndex`: return value from following() and preceding().
    f_status_index: i32,
}

impl DictionaryCache {
    /// `DictionaryCache()`.
    fn new() -> Self {
        Self {
            f_breaks: DequeI::new(),
            f_position_in_cache: -1,
            f_start: 0,
            f_limit: 0,
            f_first_rule_status_index: 0,
            f_other_rule_status_index: 0,
            f_boundary: 0,
            f_status_index: 0,
        }
    }

    /// `reset()`.
    fn reset(&mut self) {
        self.f_position_in_cache = -1;
        self.f_start = 0;
        self.f_limit = 0;
        self.f_first_rule_status_index = 0;
        self.f_other_rule_status_index = 0;
        self.f_breaks.remove_all_elements();
    }

    /// `following(int fromPos)`: finds the cached boundary following
    /// `from_pos` into `fBoundary`/`fStatusIndex`; `false` when it is not in
    /// the cache.
    fn following(&mut self, from_pos: i32) -> bool {
        if from_pos >= self.f_limit || from_pos < self.f_start {
            self.f_position_in_cache = -1;
            return false;
        }

        // Sequential iteration, move from previous boundary to the following

        let mut r;
        if self.f_position_in_cache >= 0
            && self.f_position_in_cache < self.f_breaks.size()
            && self.f_breaks.element_at(self.f_position_in_cache) == from_pos
        {
            self.f_position_in_cache += 1;
            if self.f_position_in_cache >= self.f_breaks.size() {
                self.f_position_in_cache = -1;
                return false;
            }
            r = self.f_breaks.element_at(self.f_position_in_cache);
            debug_assert!(r > from_pos);
            self.f_boundary = r;
            self.f_status_index = self.f_other_rule_status_index;
            return true;
        }

        // Random indexing. Linear search for the boundary following the given position.

        self.f_position_in_cache = 0;
        while self.f_position_in_cache < self.f_breaks.size() {
            r = self.f_breaks.element_at(self.f_position_in_cache);
            if r > from_pos {
                self.f_boundary = r;
                self.f_status_index = self.f_other_rule_status_index;
                return true;
            }
            self.f_position_in_cache += 1;
        }

        // Internal error. fStart <= fromPos < fLimit, but no cache entry contains fromPos.
        debug_assert!(false);
        self.f_position_in_cache = -1;
        false
    }

    /// `preceding(int fromPos)`: finds the cached boundary preceding
    /// `from_pos` into `fBoundary`/`fStatusIndex`; `false` when it is not in
    /// the cache.
    fn preceding(&mut self, from_pos: i32) -> bool {
        if from_pos <= self.f_start || from_pos > self.f_limit {
            self.f_position_in_cache = -1;
            return false;
        }

        if from_pos == self.f_limit {
            self.f_position_in_cache = self.f_breaks.size() - 1;
            if self.f_position_in_cache >= 0 {
                debug_assert!(self.f_breaks.element_at(self.f_position_in_cache) == from_pos);
            }
        }

        let mut r;
        if self.f_position_in_cache > 0
            && self.f_position_in_cache < self.f_breaks.size()
            && self.f_breaks.element_at(self.f_position_in_cache) == from_pos
        {
            self.f_position_in_cache -= 1;
            r = self.f_breaks.element_at(self.f_position_in_cache);
            debug_assert!(r < from_pos);
            self.f_boundary = r;
            self.f_status_index = if r == self.f_start {
                self.f_first_rule_status_index
            } else {
                self.f_other_rule_status_index
            };
            return true;
        }

        if self.f_position_in_cache == 0 {
            self.f_position_in_cache = -1;
            return false;
        }

        self.f_position_in_cache = self.f_breaks.size() - 1;
        while self.f_position_in_cache >= 0 {
            r = self.f_breaks.element_at(self.f_position_in_cache);
            if r < from_pos {
                self.f_boundary = r;
                self.f_status_index = if r == self.f_start {
                    self.f_first_rule_status_index
                } else {
                    self.f_other_rule_status_index
                };
                return true;
            }
            self.f_position_in_cache -= 1;
        }
        debug_assert!(false);
        self.f_position_in_cache = -1;
        false
    }
}

/// `RetainCachePosition`.
const RETAIN_CACHE_POSITION: bool = false;
/// `UpdateCachePosition`.
const UPDATE_CACHE_POSITION: bool = true;

/// The circular cache of boundaries around the current position.
///
/// Equivalent to the inner class `RuleBasedBreakIterator.BreakCache` (its
/// state and the methods that touch only it; the others are iterator
/// methods `bc_*`).
#[derive(Debug, Clone)]
pub struct BreakCache {
    /// `fStartBufIdx`: index of the first valid item in the circular buffer.
    f_start_buf_idx: i32,
    /// `fEndBufIdx`: index of the last valid item in the circular buffer.
    f_end_buf_idx: i32,
    /// `fTextIdx`: text position of the current iteration position.
    f_text_idx: i32,
    /// `fBufIdx`: index of the current iteration position in the buffer.
    f_buf_idx: i32,
    /// `fBoundaries`.
    f_boundaries: [i32; Self::CACHE_SIZE as usize],
    /// `fStatuses`.
    f_statuses: [i16; Self::CACHE_SIZE as usize],
    /// `fSideBuffer`.
    f_side_buffer: DequeI,
}

impl BreakCache {
    /// `CACHE_SIZE`: size of the circular buffer; a power of 2.
    const CACHE_SIZE: i32 = 128;

    /// `BreakCache()`.
    fn new() -> Self {
        let mut this = Self {
            f_start_buf_idx: 0,
            f_end_buf_idx: 0,
            f_text_idx: 0,
            f_buf_idx: 0,
            f_boundaries: [0; Self::CACHE_SIZE as usize],
            f_statuses: [0; Self::CACHE_SIZE as usize],
            f_side_buffer: DequeI::new(),
        };
        this.reset_default();
        this
    }

    /// `BreakCache(BreakCache src)`: a copy with a new, empty side buffer.
    fn copy_of(src: &BreakCache) -> Self {
        Self {
            f_start_buf_idx: src.f_start_buf_idx,
            f_end_buf_idx: src.f_end_buf_idx,
            f_text_idx: src.f_text_idx,
            f_buf_idx: src.f_buf_idx,
            f_boundaries: src.f_boundaries,
            f_statuses: src.f_statuses,
            f_side_buffer: DequeI::new(),
        }
    }

    /// `reset(int pos, int ruleStatus)`.
    fn reset(&mut self, pos: i32, rule_status: i32) {
        self.f_start_buf_idx = 0;
        self.f_end_buf_idx = 0;
        self.f_text_idx = pos;
        self.f_buf_idx = 0;
        self.f_boundaries[0] = pos;
        self.f_statuses[0] = rule_status as i16;
    }

    /// `reset()`: `reset(0, 0)`.
    fn reset_default(&mut self) {
        self.reset(0, 0);
    }

    /// `addFollowing(int position, int ruleStatusIdx, boolean update)`:
    /// adds a boundary at the end of the cache, dropping a chunk at the
    /// start when full.
    fn add_following(&mut self, position: i32, rule_status_idx: i32, update: bool) {
        debug_assert!(position > self.f_boundaries[self.f_end_buf_idx as usize]);
        debug_assert!(rule_status_idx <= i16::MAX as i32);
        let next_idx = Self::mod_chunk_size(self.f_end_buf_idx + 1);
        if next_idx == self.f_start_buf_idx {
            self.f_start_buf_idx = Self::mod_chunk_size(self.f_start_buf_idx + 6);
            // TODO: experiment. Probably revert to 1.
        }
        self.f_boundaries[next_idx as usize] = position;
        self.f_statuses[next_idx as usize] = rule_status_idx as i16;
        self.f_end_buf_idx = next_idx;
        if update == UPDATE_CACHE_POSITION {
            // Set current position to the newly added boundary.
            self.f_buf_idx = next_idx;
            self.f_text_idx = position;
        } else {
            // Retaining the original cache position.
            // Check if the added boundary wraps around the buffer, and would over-write the original position.
            // It's the responsibility of callers of this function to not add too many.
            debug_assert!(next_idx != self.f_buf_idx);
        }
    }

    /// `addPreceding(int position, int ruleStatusIdx, boolean update)`:
    /// adds a boundary at the start of the cache; `false` when that would
    /// overwrite the retained position.
    fn add_preceding(&mut self, position: i32, rule_status_idx: i32, update: bool) -> bool {
        debug_assert!(position < self.f_boundaries[self.f_start_buf_idx as usize]);
        debug_assert!(rule_status_idx <= i16::MAX as i32);
        let next_idx = Self::mod_chunk_size(self.f_start_buf_idx - 1);
        if next_idx == self.f_end_buf_idx {
            if self.f_buf_idx == self.f_end_buf_idx && update == RETAIN_CACHE_POSITION {
                // Failure. The insertion of the new boundary would claim the buffer position that is the
                // current iteration position. And we also want to retain the current iteration position.
                // (The buffer is already completely full of entries that precede the iteration position.)
                return false;
            }
            self.f_end_buf_idx = Self::mod_chunk_size(self.f_end_buf_idx - 1);
        }
        self.f_boundaries[next_idx as usize] = position;
        self.f_statuses[next_idx as usize] = rule_status_idx as i16;
        self.f_start_buf_idx = next_idx;
        if update == UPDATE_CACHE_POSITION {
            self.f_buf_idx = next_idx;
            self.f_text_idx = position;
        }
        true
    }

    /// `seek(int pos)`: positions the cache at the boundary at or
    /// preceding `pos` when `pos` is within the cached range.
    fn seek(&mut self, pos: i32) -> bool {
        if pos < self.f_boundaries[self.f_start_buf_idx as usize]
            || pos > self.f_boundaries[self.f_end_buf_idx as usize]
        {
            return false;
        }
        if pos == self.f_boundaries[self.f_start_buf_idx as usize] {
            // Common case: seek(0), from BreakIterator::first()
            self.f_buf_idx = self.f_start_buf_idx;
            self.f_text_idx = self.f_boundaries[self.f_buf_idx as usize];
            return true;
        }
        if pos == self.f_boundaries[self.f_end_buf_idx as usize] {
            self.f_buf_idx = self.f_end_buf_idx;
            self.f_text_idx = self.f_boundaries[self.f_buf_idx as usize];
            return true;
        }

        let mut min = self.f_start_buf_idx;
        let mut max = self.f_end_buf_idx;
        while min != max {
            let mut probe = (min + max + if min > max { Self::CACHE_SIZE } else { 0 }) / 2;
            probe = Self::mod_chunk_size(probe);
            if self.f_boundaries[probe as usize] > pos {
                max = probe;
            } else {
                min = Self::mod_chunk_size(probe + 1);
            }
        }
        debug_assert!(self.f_boundaries[max as usize] > pos);
        self.f_buf_idx = Self::mod_chunk_size(max - 1);
        self.f_text_idx = self.f_boundaries[self.f_buf_idx as usize];
        debug_assert!(self.f_text_idx <= pos);
        true
    }

    /// `dumpCache()`: prints the cache to standard output.
    #[allow(dead_code)]
    fn dump_cache(&self) {
        println!("fTextIdx:{}   fBufIdx:{}", self.f_text_idx, self.f_buf_idx);
        let mut i = self.f_start_buf_idx;
        loop {
            println!("{}  {}", i, self.f_boundaries[i as usize]);
            if i == self.f_end_buf_idx {
                break;
            }
            i = Self::mod_chunk_size(i + 1);
        }
    }

    /// `modChunkSize(int index)`.
    fn mod_chunk_size(index: i32) -> i32 {
        index & (Self::CACHE_SIZE - 1)
    }
}
