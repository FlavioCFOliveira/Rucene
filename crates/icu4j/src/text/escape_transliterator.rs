// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.EscapeTransliterator`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::units;
use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{
    factory, register_factory_in, transliterator_boilerplate, Position, Transliterator,
    TransliteratorBase,
};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A transliterator that converts Unicode characters to an escape form.
/// Examples of escape forms are "U+4E01" and "&#x10FFFF;". Escape forms
/// have a prefix and suffix, either of which may be empty, a radix,
/// typically 16 or 10, a minimum digit count, typically 1, 4, or 8, and a
/// boolean that specifies whether supplemental characters are handled as
/// 32-bit code points or as two 16-bit code units. Most escape forms
/// handle 32-bit code points, but some, such as the Java form, intentionally
/// break them into two surrogate pairs, for backward compatibility.
///
/// Some escape forms actually have two different patterns, one for BMP
/// characters (0..FFFF) and one for supplements (10000..10FFFF). The
/// Unicode escape form is an example: "丁" vs. "\U00010000". To
/// handle this, a second `EscapeTransliterator` may be defined that
/// specifies the pattern to be produced for supplementals. An example of
/// a form that requires this is the C form, which uses "￿" for BMP
/// characters and "\U0010FFFF" for supplementals.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.EscapeTransliterator`.
#[derive(Debug)]
pub struct EscapeTransliterator {
    base: TransliteratorBase,
    /// The prefix of the escape form; may be empty, but usually isn't.
    prefix: Vec<u16>,
    /// The prefix of the escape form; often empty.
    suffix: Vec<u16>,
    /// The radix to display the number in. Typically 16 or 10. Must be in
    /// the range 2 to 36.
    radix: i32,
    /// The minimum number of digits. Typically 1, 4, or 8. Values less
    /// than 1 are equivalent to 1.
    min_digits: i32,
    /// If true, supplementals are handled as 32-bit code points. If false,
    /// they are handled as two 16-bit code units.
    grok_supplementals: bool,
    /// The form to be used for supplementals. If this is null then the
    /// same form is used for BMP characters and supplementals. If this is
    /// not null and if `grokSupplementals` is true then the prefix,
    /// suffix, radix, and minDigits of this object are used for
    /// supplementals.
    supplemental_handler: Option<Box<EscapeTransliterator>>,
}

impl EscapeTransliterator {
    /// `register()`: registers standard variants with the system. Called by
    /// Transliterator during initialization (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        // Unicode: "U+10FFFF" hex, min=4, max=6
        register_factory_in(
            registry,
            &units("Any-Hex/Unicode"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/Unicode",
                    "U+",
                    "",
                    16,
                    4,
                    true,
                    None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // Java: "\\uFFFF" hex, min=4, max=4
        register_factory_in(
            registry,
            &units("Any-Hex/Java"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/Java",
                    "\\u",
                    "",
                    16,
                    4,
                    false,
                    None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // C: "\\uFFFF" hex, min=4, max=4; \\U0010FFFF hex, min=8, max=8
        register_factory_in(
            registry,
            &units("Any-Hex/C"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/C",
                    "\\u",
                    "",
                    16,
                    4,
                    true,
                    Some(EscapeTransliterator::new("", "\\U", "", 16, 8, true, None)),
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // XML: "&#x10FFFF;" hex, min=1, max=6
        register_factory_in(
            registry,
            &units("Any-Hex/XML"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/XML",
                    "&#x",
                    ";",
                    16,
                    1,
                    true,
                    None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // XML10: "&1114111;" dec, min=1, max=7 (not really "Hex-Any")
        register_factory_in(
            registry,
            &units("Any-Hex/XML10"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/XML10",
                    "&#",
                    ";",
                    10,
                    1,
                    true,
                    None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // Perl: "\\x{263A}" hex, min=1, max=6
        register_factory_in(
            registry,
            &units("Any-Hex/Perl"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/Perl",
                    "\\x{",
                    "}",
                    16,
                    1,
                    true,
                    None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // Plain: "FFFF" hex, min=4, max=6
        register_factory_in(
            registry,
            &units("Any-Hex/Plain"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex/Plain",
                    "",
                    "",
                    16,
                    4,
                    true,
                    None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // Generic
        register_factory_in(
            registry,
            &units("Any-Hex"),
            factory(|_id| {
                Ok(Some(Arc::new(EscapeTransliterator::new(
                    "Any-Hex", "\\u", "", 16, 4, false, None,
                )) as Arc<dyn Transliterator>))
            }),
        )?;
        Ok(())
    }

    /// `EscapeTransliterator(String ID, String prefix, String suffix, int
    /// radix, int minDigits, boolean grokSupplementals,
    /// EscapeTransliterator supplementalHandler)`: constructs an escape
    /// transliterator with the given ID and parameters. See the class
    /// member documentation for details.
    pub(crate) fn new(
        id: &str,
        prefix: &str,
        suffix: &str,
        radix: i32,
        min_digits: i32,
        grok_supplementals: bool,
        supplemental_handler: Option<EscapeTransliterator>,
    ) -> Self {
        Self {
            base: TransliteratorBase::with_id(units(id)),
            prefix: units(prefix),
            suffix: units(suffix),
            radix,
            min_digits,
            grok_supplementals,
            supplemental_handler: supplemental_handler.map(Box::new),
        }
    }
}

impl Transliterator for EscapeTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position pos, boolean
    /// incremental)`.
    fn handle_transliterate(
        &self,
        text: &dyn Replaceable,
        pos: &mut Position,
        _incremental: bool,
    ) -> Result<()> {
        let mut start = pos.start;
        let mut limit = pos.limit;

        let mut buf = self.prefix.clone();
        let prefix_len = self.prefix.len();
        let mut redo_prefix = false;

        while start < limit {
            let c = if self.grok_supplementals {
                text.char32_at(start)
            } else {
                text.char_at(start) as i32
            };
            let char_len = if self.grok_supplementals {
                UTF16::get_char_count(c)
            } else {
                1
            };

            if let Some(handler) = self
                .supplemental_handler
                .as_ref()
                .filter(|_| (c as u32 & 0xFFFF0000) != 0)
            {
                buf.clear();
                buf.extend_from_slice(&handler.prefix);
                Utility::append_number(&mut buf, c, handler.radix, handler.min_digits)?;
                buf.extend_from_slice(&handler.suffix);
                redo_prefix = true;
            } else {
                if redo_prefix {
                    buf.clear();
                    buf.extend_from_slice(&self.prefix);
                    redo_prefix = false;
                } else {
                    buf.truncate(prefix_len);
                }
                Utility::append_number(&mut buf, c, self.radix, self.min_digits)?;
                buf.extend_from_slice(&self.suffix);
            }

            text.replace(start, start + char_len, &buf);
            start += buf.len() as i32;
            limit += buf.len() as i32 - char_len;
        }

        pos.context_limit += limit - pos.limit;
        pos.limit = limit;
        pos.start = start;
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
        source_set.add_all(&self.get_filter_as_unicode_set(input_filter)?)?;
        let mut it = Some(self);
        while let Some(t) = it {
            if input_filter.size() != 0 {
                target_set.add_all_str(&t.prefix)?;
                target_set.add_all_str(&t.suffix)?;
                let mut buffer = Vec::new();
                for i in 0..t.radix {
                    Utility::append_number(&mut buffer, i, t.radix, t.min_digits)?;
                }
                target_set.add_all_str(&buffer)?; // TODO drop once we've changed to char sets
            }
            it = t.supplemental_handler.as_deref();
        }
        Ok(())
    }
}
