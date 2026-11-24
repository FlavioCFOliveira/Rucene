// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2012-2016, Google, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.text.ListFormatter`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::java_regex::{JavaPattern, CASE_INSENSITIVE};
use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::text::{AttributeValue, AttributedCharacterIterator, FieldRef};
use crate::jdk::units;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::formatted_value_string_builder_impl::{
    FormattedValueStringBuilderImpl, SpanFieldPlaceholder,
};
use crate::r#impl::icu_cache::ICUCache;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::simple_cache::SimpleCache;
use crate::r#impl::simple_formatter_impl::{IterInternal, SimpleFormatterImpl};
use crate::r#impl::utility::Utility;
use crate::text::constrained_field_position::ConstrainedFieldPosition;
use crate::text::formatted_value::FormattedValue;
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::UResourceBundle;

/// The Java class name of `ListFormatter.Field`.
pub const FIELD_CLASS: &str = "com.ibm.icu.text.ListFormatter$Field";

/// `ListFormatter.Field` and its constants.
pub mod field {
    use rucene::error::{LuceneError, Result};

    use crate::jdk::text::{Attribute, FieldRef};

    crate::java_format_field_class!(
        /// Field selectors for format fields defined by ListFormatter.
        /// Equivalent to `ListFormatter.Field`.
        pub Field, "com.ibm.icu.text.ListFormatter$Field", []
    );

    /// The literal text in the result which came from the resources.
    /// Equivalent to `LITERAL`.
    pub static LITERAL: Field = Field::new("literal");

    /// The element text in the result which came from the input strings.
    /// Equivalent to `ELEMENT`.
    pub static ELEMENT: Field = Field::new("element");

    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = -8071145668708265437;

    impl Field {
        /// Serizalization method resolve instances to the constant Field
        /// values. Equivalent to `readResolve()`.
        ///
        /// # Errors
        ///
        /// `InvalidObjectException("An invalid object.")`.
        pub fn read_resolve(&self) -> Result<FieldRef> {
            if self.get_name() == LITERAL.get_name() {
                return Ok(&LITERAL);
            }
            if self.get_name() == ELEMENT.get_name() {
                return Ok(&ELEMENT);
            }
            Err(LuceneError::unchecked(
                "java.io.InvalidObjectException",
                "An invalid object.",
            ))
        }
    }
}

/// `ListFormatter.SpanField` and its constant.
pub mod span_field {
    use rucene::error::{LuceneError, Result};

    use crate::jdk::text::{Attribute, FieldRef};

    crate::java_format_field_class!(
        /// Field selectors for format fields defined by ListFormatter.
        /// Equivalent to `ListFormatter.SpanField` (a `UFormat.SpanField`).
        pub SpanField, "com.ibm.icu.text.ListFormatter$SpanField", ["com.ibm.icu.text.UFormat$SpanField"]
    );

    /// The span field for the list: the field value is the index of the
    /// element. Equivalent to `LIST_SPAN`.
    pub static LIST_SPAN: SpanField = SpanField::new("list-span");

    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = 3563544214705634403;

    impl SpanField {
        /// Serialization method resolve instances to the constant
        /// ListFormatter.SpanField values. Equivalent to `readResolve()`.
        ///
        /// # Errors
        ///
        /// `InvalidObjectException("An invalid object.")`.
        pub fn read_resolve(&self) -> Result<FieldRef> {
            if self.get_name() == LIST_SPAN.get_name() {
                return Ok(&LIST_SPAN);
            }
            Err(LuceneError::unchecked(
                "java.io.InvalidObjectException",
                "An invalid object.",
            ))
        }
    }
}

/// Indicates the type of list. Equivalent to `ListFormatter.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    /// Conjunction formatting, e.g. "Alice, Bob, Charlie, and Delta".
    And,
    /// Disjunction (or alternative, or simply one of) formatting, e.g.
    /// "Alice, Bob, Charlie, or Delta".
    Or,
    /// Formatting of a list of values with units, e.g. "5 pounds, 12 ounces".
    Units,
}

/// Indicates the width of the list. Equivalent to `ListFormatter.Width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Width {
    /// Use list formatting with full words (no abbreviations) when possible.
    Wide,
    /// Use list formatting of typical length.
    Short,
    /// Use list formatting of the shortest possible length.
    Narrow,
}

/// An immutable class containing the result of a list formatting
/// operation. Equivalent to `ListFormatter.FormattedList`.
#[derive(Debug, Clone)]
pub struct FormattedList {
    string: FormattedStringBuilder,
}

impl FormattedList {
    /// Equivalent to the package-private `FormattedList(FormattedStringBuilder)`.
    pub(crate) fn new(string: FormattedStringBuilder) -> Self {
        Self { string }
    }
}

impl FormattedValue for FormattedList {
    fn length(&self) -> i32 {
        self.string.length()
    }

    fn char_at(&self, index: i32) -> u16 {
        self.string.char_at(index)
    }

    fn sub_sequence(&self, start: i32, end: i32) -> Vec<u16> {
        self.string.sub_string(start, end)
    }

    fn to_java_string(&self) -> Vec<u16> {
        self.string.to_java_string()
    }

    fn append_to<'a>(&self, appendable: &'a mut Vec<u16>) -> &'a mut Vec<u16> {
        Utility::append_to(&self.string.to_java_string(), appendable)
    }

    fn next_position(&self, cfpos: &mut ConstrainedFieldPosition) -> bool {
        FormattedValueStringBuilderImpl::next_position(&self.string, cfpos, None)
    }

    fn to_character_iterator(&self) -> Result<Box<dyn AttributedCharacterIterator>> {
        FormattedValueStringBuilderImpl::to_character_iterator(&self.string, None)
    }
}

/// `PatternHandler`: the two and end patterns, possibly depending on the
/// text of the element that follows them.
#[derive(Debug, Clone)]
enum PatternHandler {
    /// `StaticHandler`.
    Static {
        two_pattern: Vec<u16>,
        end_pattern: Vec<u16>,
    },
    /// `ContextualHandler`.
    Contextual {
        regexp: &'static LazyLock<JavaPattern>,
        then_two_pattern: Vec<u16>,
        else_two_pattern: Vec<u16>,
        then_end_pattern: Vec<u16>,
        else_end_pattern: Vec<u16>,
    },
}

impl PatternHandler {
    /// `getTwoPattern(String)`.
    fn get_two_pattern(&self, text: &[u16]) -> &[u16] {
        match self {
            PatternHandler::Static { two_pattern, .. } => two_pattern,
            PatternHandler::Contextual {
                regexp,
                then_two_pattern,
                else_two_pattern,
                ..
            } => {
                if regexp.matches(&String::from_utf16_lossy(text)) {
                    then_two_pattern
                } else {
                    else_two_pattern
                }
            }
        }
    }

    /// `getEndPattern(String)`.
    fn get_end_pattern(&self, text: &[u16]) -> &[u16] {
        match self {
            PatternHandler::Static { end_pattern, .. } => end_pattern,
            PatternHandler::Contextual {
                regexp,
                then_end_pattern,
                else_end_pattern,
                ..
            } => {
                if regexp.matches(&String::from_utf16_lossy(text)) {
                    then_end_pattern
                } else {
                    else_end_pattern
                }
            }
        }
    }
}

fn compile_static(pattern: &str) -> Vec<u16> {
    ListFormatter::compile_pattern(&units(pattern), &mut Vec::new())
        .expect("the built-in list patterns are valid")
}

fn regex(pattern: &str, flags: u32) -> JavaPattern {
    JavaPattern::compile(pattern, flags).expect("the built-in list regular expressions are valid")
}

// Compiled SimpleFormatter patterns.
/// `compiledY`.
static COMPILED_Y: LazyLock<Vec<u16>> = LazyLock::new(|| compile_static("{0} y {1}"));
/// `compiledE`.
static COMPILED_E: LazyLock<Vec<u16>> = LazyLock::new(|| compile_static("{0} e {1}"));
/// `compiledO`.
static COMPILED_O: LazyLock<Vec<u16>> = LazyLock::new(|| compile_static("{0} o {1}"));
/// `compiledU`.
static COMPILED_U: LazyLock<Vec<u16>> = LazyLock::new(|| compile_static("{0} u {1}"));

// Regex to match Spanish heuristic.
/// `changeToE`.
static CHANGE_TO_E: LazyLock<JavaPattern> =
    LazyLock::new(|| regex("(i.*|hi|hi[^ae].*)", CASE_INSENSITIVE));
/// `changeToU`.
static CHANGE_TO_U: LazyLock<JavaPattern> =
    LazyLock::new(|| regex("((o|ho|8).*|11)", CASE_INSENSITIVE));

// Compiled SimpleFormatter patterns.
/// `compiledVav`.
static COMPILED_VAV: LazyLock<Vec<u16>> = LazyLock::new(|| compile_static("{0} \u{05D5}{1}"));
/// `compiledVavDash`.
static COMPILED_VAV_DASH: LazyLock<Vec<u16>> = LazyLock::new(|| compile_static("{0} \u{05D5}-{1}"));

// Regex to match Hebrew heuristic.
/// `changeToVavDash`.
static CHANGE_TO_VAV_DASH: LazyLock<JavaPattern> =
    LazyLock::new(|| regex("^[\\P{InHebrew}].*$", 0));

/// `cache`.
static CACHE: LazyLock<Cache> = LazyLock::new(|| Cache {
    cache: SimpleCache::new(),
});

/// Immutable class for formatting a list, using data from CLDR (or
/// supplied separately). The class is not subclassable.
///
/// Equivalent to `com.ibm.icu.text.ListFormatter`. Patterns are UTF-16;
/// the `Object` items of `format` are given by their `toString()` texts
/// (the only thing Java reads from them, through `String.valueOf` and
/// `toString`). Instances are shared through the cache (`Arc`).
#[derive(Debug, Clone)]
pub struct ListFormatter {
    // Compiled SimpleFormatter patterns.
    start: Vec<u16>,
    middle: Vec<u16>,
    locale: Option<ULocale>,
    pattern_handler: PatternHandler,
}

impl ListFormatter {
    /// Internal: Create a ListFormatter from component strings, with
    /// definitions as in LDML. Equivalent to `ListFormatter(String, String,
    /// String, String)` (internal, deprecated).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a pattern does not have exactly two
    /// arguments.
    pub fn new(two: &[u16], start: &[u16], middle: &[u16], end: &[u16]) -> Result<Self> {
        Ok(Self::with_locale(
            Self::compile_pattern(two, &mut Vec::new())?,
            Self::compile_pattern(start, &mut Vec::new())?,
            Self::compile_pattern(middle, &mut Vec::new())?,
            Self::compile_pattern(end, &mut Vec::new())?,
            None,
        ))
    }

    /// `ListFormatter(String, String, String, String, ULocale)`.
    fn with_locale(
        two: Vec<u16>,
        start: Vec<u16>,
        middle: Vec<u16>,
        end: Vec<u16>,
        locale: Option<ULocale>,
    ) -> Self {
        let pattern_handler = Self::create_pattern_handler(locale.as_ref(), two, end);
        Self {
            start,
            middle,
            locale,
            pattern_handler,
        }
    }

    /// `compilePattern(String, StringBuilder)`.
    fn compile_pattern(pattern: &[u16], sb: &mut Vec<u16>) -> Result<Vec<u16>> {
        SimpleFormatterImpl::compile_to_string_min_max_arguments(pattern, sb, 2, 2)
    }

    /// Create a list formatter that is appropriate for a locale. Equivalent
    /// to `getInstance(ULocale, Type, Width)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid list format type/width")`, and the
    /// errors of the list data.
    pub fn get_instance_typed(
        locale: &ULocale,
        type_: Type,
        width: Width,
    ) -> Result<Arc<ListFormatter>> {
        let style_name = match Self::type_width_to_style_string(type_, width) {
            None => {
                return Err(LuceneError::illegal_argument(
                    "Invalid list format type/width",
                ))
            }
            Some(s) => s,
        };
        CACHE.get(locale, style_name)
    }

    /// Equivalent to `getInstance(Locale, Type, Width)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance_typed`](Self::get_instance_typed).
    pub fn get_instance_locale_typed(
        locale: &Locale,
        type_: Type,
        width: Width,
    ) -> Result<Arc<ListFormatter>> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::get_instance_typed(&ulocale, type_, width)
    }

    /// Create a list formatter that is appropriate for a locale (AND, WIDE).
    /// Equivalent to `getInstance(ULocale)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance_typed`](Self::get_instance_typed).
    pub fn get_instance_ulocale(locale: &ULocale) -> Result<Arc<ListFormatter>> {
        Self::get_instance_typed(locale, Type::And, Width::Wide)
    }

    /// Equivalent to `getInstance(Locale)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance_typed`](Self::get_instance_typed).
    pub fn get_instance_locale(locale: &Locale) -> Result<Arc<ListFormatter>> {
        Self::get_instance_locale_typed(locale, Type::And, Width::Wide)
    }

    /// Create a list formatter that is appropriate for the default FORMAT
    /// locale. Equivalent to `getInstance()`.
    ///
    /// # Errors
    ///
    /// As [`get_instance_typed`](Self::get_instance_typed).
    pub fn get_instance() -> Result<Arc<ListFormatter>> {
        Self::get_instance_ulocale(&ULocale::get_default_category(Category::Format))
    }

    /// Format a list of objects. Equivalent to `format(Object...)` and
    /// `format(Collection<?>)`.
    pub fn format(&self, items: &[Vec<u16>]) -> Vec<u16> {
        self.format_impl(items, false).to_java_string()
    }

    /// Format a list of objects to a FormattedList. You can access the
    /// offsets of each element from the FormattedList. Equivalent to
    /// `formatToValue(Object...)` and `formatToValue(Collection<?>)`.
    pub fn format_to_value(&self, items: &[Vec<u16>]) -> FormattedList {
        self.format_impl(items, true).to_value()
    }

    /// Formats a collection of objects. Equivalent to the package-private
    /// `formatImpl(Collection<?>, boolean)`.
    pub(crate) fn format_impl(
        &self,
        items: &[Vec<u16>],
        needs_fields: bool,
    ) -> FormattedListBuilder {
        let mut it = items.iter();
        let count = items.len() as i32;
        match count {
            0 => return FormattedListBuilder::new(&[], needs_fields),
            1 => return FormattedListBuilder::new(it.next().expect("one item"), needs_fields),
            2 => {
                let first = it.next().expect("two items");
                let second = it.next().expect("two items");
                let mut b = FormattedListBuilder::new(first, needs_fields);
                b.append(self.pattern_handler.get_two_pattern(second), second, 1);
                return b;
            }
            _ => {}
        }
        let mut builder = FormattedListBuilder::new(it.next().expect("items"), needs_fields);
        builder.append(&self.start, it.next().expect("items"), 1);
        for idx in 2..count - 1 {
            builder.append(&self.middle, it.next().expect("items"), idx);
        }
        let last = it.next().expect("items");
        builder.append(self.pattern_handler.get_end_pattern(last), last, count - 1);
        builder
    }

    /// `createPatternHandler(String, String)`.
    fn create_pattern_handler(
        locale: Option<&ULocale>,
        two: Vec<u16>,
        end: Vec<u16>,
    ) -> PatternHandler {
        if let Some(locale) = locale {
            let language = locale.get_language();
            if language == "es" {
                let two_is_y = two == *COMPILED_Y;
                let end_is_y = end == *COMPILED_Y;
                if two_is_y || end_is_y {
                    return PatternHandler::Contextual {
                        regexp: &CHANGE_TO_E,
                        then_two_pattern: if two_is_y {
                            COMPILED_E.clone()
                        } else {
                            two.clone()
                        },
                        else_two_pattern: two,
                        then_end_pattern: if end_is_y {
                            COMPILED_E.clone()
                        } else {
                            end.clone()
                        },
                        else_end_pattern: end,
                    };
                }
                let two_is_o = two == *COMPILED_O;
                let end_is_o = end == *COMPILED_O;
                if two_is_o || end_is_o {
                    return PatternHandler::Contextual {
                        regexp: &CHANGE_TO_U,
                        then_two_pattern: if two_is_o {
                            COMPILED_U.clone()
                        } else {
                            two.clone()
                        },
                        else_two_pattern: two,
                        then_end_pattern: if end_is_o {
                            COMPILED_U.clone()
                        } else {
                            end.clone()
                        },
                        else_end_pattern: end,
                    };
                }
            } else if language == "he" || language == "iw" {
                let two_is_vav = two == *COMPILED_VAV;
                let end_is_vav = end == *COMPILED_VAV;
                if two_is_vav || end_is_vav {
                    return PatternHandler::Contextual {
                        regexp: &CHANGE_TO_VAV_DASH,
                        then_two_pattern: if two_is_vav {
                            COMPILED_VAV_DASH.clone()
                        } else {
                            two.clone()
                        },
                        else_two_pattern: two,
                        then_end_pattern: if end_is_vav {
                            COMPILED_VAV_DASH.clone()
                        } else {
                            end.clone()
                        },
                        else_end_pattern: end,
                    };
                }
            }
        }
        PatternHandler::Static {
            two_pattern: two,
            end_pattern: end,
        }
    }

    /// Returns the pattern to use for a particular item count. Equivalent to
    /// `getPatternForNumItems(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("count must be > 0")`.
    pub fn get_pattern_for_num_items(&self, count: i32) -> Result<Vec<u16>> {
        if count <= 0 {
            return Err(LuceneError::illegal_argument("count must be > 0"));
        }
        let list: Vec<Vec<u16>> = (0..count).map(|i| units(&format!("{{{i}}}"))).collect();
        Ok(self.format(&list))
    }

    /// Returns the locale of this object (`None` for a formatter built from
    /// component strings). Equivalent to `getLocale()` (internal).
    pub fn get_locale(&self) -> Option<&ULocale> {
        self.locale.as_ref()
    }

    /// Equivalent to the package-private `typeWidthToStyleString(Type,
    /// Width)`.
    pub(crate) fn type_width_to_style_string(type_: Type, width: Width) -> Option<&'static str> {
        Some(match (type_, width) {
            (Type::And, Width::Wide) => "standard",
            (Type::And, Width::Short) => "standard-short",
            (Type::And, Width::Narrow) => "standard-narrow",
            (Type::Or, Width::Wide) => "or",
            (Type::Or, Width::Short) => "or-short",
            (Type::Or, Width::Narrow) => "or-narrow",
            (Type::Units, Width::Wide) => "unit",
            (Type::Units, Width::Short) => "unit-short",
            (Type::Units, Width::Narrow) => "unit-narrow",
        })
    }
}

/// Builds the formatted list. Equivalent to the package-private
/// `ListFormatter.FormattedListBuilder`.
#[derive(Debug, Clone)]
pub(crate) struct FormattedListBuilder {
    string: FormattedStringBuilder,
    needs_fields: bool,
}

impl FormattedListBuilder {
    // Start is the first object in the list; If needsFields is true, enable
    // the slightly more expensive code path that records offsets.
    fn new(start: &[u16], needs_fields: bool) -> Self {
        let mut string = FormattedStringBuilder::new();
        string.set_appendable_field(Some(FieldObject::Field(&field::LITERAL)));
        let mut b = Self {
            string,
            needs_fields,
        };
        b.append_element(start, 0);
        b
    }

    // Appends additional object. pattern is a template indicating where
    // the new object gets added in relation to the rest of the list. {0}
    // represents the rest of the list; {1} represents the new object in
    // pattern. next is the object to be added. If recordOffset is true,
    // records the offset of next in the formatted string.
    fn append(&mut self, compiled_pattern: &[u16], next: &[u16], position: i32) -> &mut Self {
        debug_assert!(SimpleFormatterImpl::get_argument_limit(compiled_pattern) == 2);
        self.string.set_append_index(0);
        let mut state: i64 = 0;
        loop {
            // IterInternal.step appends the literal segments to the builder
            // (an Appendable, field LITERAL at the append index).
            let mut literal: Vec<u16> = Vec::new();
            state = IterInternal::step(state, compiled_pattern, &mut literal);
            if !literal.is_empty() {
                self.string.append_csq(&literal);
            }
            if state == IterInternal::DONE {
                break;
            }
            let arg_index = IterInternal::get_arg_index(state);
            if arg_index == 0 {
                let length = self.string.length();
                self.string.set_append_index(length);
            } else {
                self.append_element(next, position);
            }
        }
        self
    }

    fn append_element(&mut self, element_string: &[u16], position: i32) {
        if self.needs_fields {
            let field = SpanFieldPlaceholder {
                span_field: Some(&span_field::LIST_SPAN as FieldRef),
                normal_field: Some(&field::ELEMENT as FieldRef),
                value: Some(AttributeValue::Integer(position)),
                start: -1,
                length: element_string.len() as i32,
            };
            self.string
                .append(element_string, Some(FieldObject::Wrapper(Arc::new(field))));
        } else {
            self.string.append(element_string, None);
        }
    }

    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn append_to(&self, appendable: &mut Vec<u16>) {
        Utility::append_to(&self.string.to_java_string(), appendable);
    }

    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn get_offset(&self, field_position_found_index: i32) -> i32 {
        FormattedValueStringBuilderImpl::find_span(
            &self.string,
            &AttributeValue::Integer(field_position_found_index),
        )
    }

    pub(crate) fn to_java_string(&self) -> Vec<u16> {
        self.string.to_java_string()
    }

    #[allow(clippy::wrong_self_convention)] // Java method name and receiver
    pub(crate) fn to_value(self) -> FormattedList {
        FormattedList::new(self.string)
    }
}

/// The formatter cache. Equivalent to `ListFormatter.Cache`.
struct Cache {
    cache: SimpleCache<String, Arc<ListFormatter>>,
}

impl Cache {
    fn get(&self, locale: &ULocale, style: &str) -> Result<Arc<ListFormatter>> {
        let key = format!("{}:{}", locale, style);
        if let Some(result) = self.cache.get(&key) {
            return Ok(result);
        }
        let result = Arc::new(Self::load(locale, style)?);
        self.cache.put(key, result.clone());
        Ok(result)
    }

    fn load(ulocale: &ULocale, style: &str) -> Result<ListFormatter> {
        let r =
            UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(ulocale))?;
        let r = r.as_icu()?;
        let mut sb: Vec<u16> = Vec::new();
        let get = |part: &str| -> Result<Vec<u16>> {
            r.get_with_fallback(&format!("listPattern/{style}/{part}"))?
                .get_string()
        };
        Ok(ListFormatter::with_locale(
            ListFormatter::compile_pattern(&get("2")?, &mut sb)?,
            ListFormatter::compile_pattern(&get("start")?, &mut sb)?,
            ListFormatter::compile_pattern(&get("middle")?, &mut sb)?,
            ListFormatter::compile_pattern(&get("end")?, &mut sb)?,
            Some(ulocale.clone()),
        ))
    }
}
