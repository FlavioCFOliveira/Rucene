// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnescapeTransliterator`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::transliterator::{factory, register_factory_in, transliterator_boilerplate, Position, Transliterator, TransliteratorBase};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A transliterator that converts Unicode escape forms to the characters
/// they represent. Escape forms have a prefix, a suffix, a radix, and
/// minimum and maximum digit counts.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.UnescapeTransliterator`.
#[derive(Debug)]
pub struct UnescapeTransliterator {
    base: TransliteratorBase,
    /// The encoded pattern specification. The pattern consists of zero or
    /// more forms. Each form consists of a prefix, suffix, radix, minimum
    /// digit count, and maximum digit count. These values are stored as a
    /// five character header. That is, their numeric values are cast to
    /// 16-bit characters and stored in the string. Following these five
    /// characters, the prefix characters, then suffix characters are
    /// stored. Each form thus takes n+5 characters, where n is the total
    /// length of the prefix and suffix. The end is marked by a header of
    /// length one consisting of the character END.
    spec: Vec<u16>,
}

/// `END`: special character marking the end of the spec[] array.
const END: u16 = 0xFFFF;

const BS: u16 = b'\\' as u16;

impl UnescapeTransliterator {
    /// `register()`: registers standard variants with the system. Called by
    /// Transliterator during initialization (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        // Unicode: "U+10FFFF" hex, min=4, max=6
        register_factory_in(
            registry,
            &units("Hex-Any/Unicode"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new(
                    "Hex-Any/Unicode",
                    vec![2, 0, 16, 4, 6, b'U' as u16, b'+' as u16, END],
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // Java: "\\uFFFF" hex, min=4, max=4
        register_factory_in(
            registry,
            &units("Hex-Any/Java"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new("Hex-Any/Java", vec![2, 0, 16, 4, 4, BS, b'u' as u16, END]))
                    as Arc<dyn Transliterator>))
            }),
        )?;

        // C: "\\uFFFF" hex, min=4, max=4; \\U0010FFFF hex, min=8, max=8
        register_factory_in(
            registry,
            &units("Hex-Any/C"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new(
                    "Hex-Any/C",
                    vec![2, 0, 16, 4, 4, BS, b'u' as u16, 2, 0, 16, 8, 8, BS, b'U' as u16, END],
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // XML: "&#x10FFFF;" hex, min=1, max=6
        register_factory_in(
            registry,
            &units("Hex-Any/XML"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new(
                    "Hex-Any/XML",
                    vec![3, 1, 16, 1, 6, b'&' as u16, b'#' as u16, b'x' as u16, b';' as u16, END],
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // XML10: "&1114111;" dec, min=1, max=7 (not really "Hex-Any")
        register_factory_in(
            registry,
            &units("Hex-Any/XML10"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new(
                    "Hex-Any/XML10",
                    vec![2, 1, 10, 1, 7, b'&' as u16, b'#' as u16, b';' as u16, END],
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // Perl: "\\x{263A}" hex, min=1, max=6
        register_factory_in(
            registry,
            &units("Hex-Any/Perl"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new(
                    "Hex-Any/Perl",
                    vec![3, 1, 16, 1, 6, BS, b'x' as u16, b'{' as u16, b'}' as u16, END],
                )) as Arc<dyn Transliterator>))
            }),
        )?;

        // All: Java, C, Perl, XML, XML10, Unicode
        register_factory_in(
            registry,
            &units("Hex-Any"),
            factory(|_id| {
                Ok(Some(Arc::new(UnescapeTransliterator::new(
                    "Hex-Any",
                    vec![
                        2, 0, 16, 4, 6, b'U' as u16, b'+' as u16, // Unicode
                        2, 0, 16, 4, 4, BS, b'u' as u16, // Java
                        2, 0, 16, 8, 8, BS, b'U' as u16, // C (surrogates)
                        3, 1, 16, 1, 6, b'&' as u16, b'#' as u16, b'x' as u16, b';' as u16, // XML
                        2, 1, 10, 1, 7, b'&' as u16, b'#' as u16, b';' as u16, // XML10
                        3, 1, 16, 1, 6, BS, b'x' as u16, b'{' as u16, b'}' as u16, // Perl
                        END,
                    ],
                )) as Arc<dyn Transliterator>))
            }),
        )?;
        Ok(())
    }

    /// `UnescapeTransliterator(String ID, char spec[])`: package private
    /// constructor. Takes the encoded spec array.
    pub(crate) fn new(id: &str, spec: Vec<u16>) -> Self {
        Self {
            base: TransliteratorBase::with_id(units(id)),
            spec,
        }
    }
}

impl Transliterator for UnescapeTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position pos, boolean
    /// isIncremental)`.
    fn handle_transliterate(&self, text: &dyn Replaceable, pos: &mut Position, is_incremental: bool) -> Result<()> {
        let spec = &self.spec;
        let mut start = pos.start;
        let mut limit = pos.limit;

        'loop_: while start < limit {
            // Loop over the forms in spec[].  Exit this loop when we
            // match one of the specs.  Exit the outer loop if a
            // partial match is detected and isIncremental is true.
            let mut ipat = 0usize;
            while spec[ipat] != END {
                // Read the header
                let prefix_len = spec[ipat] as usize;
                ipat += 1;
                let suffix_len = spec[ipat] as usize;
                ipat += 1;
                let radix = spec[ipat] as i32;
                ipat += 1;
                let min_digits = spec[ipat] as i32;
                ipat += 1;
                let max_digits = spec[ipat] as i32;
                ipat += 1;

                // s is a copy of start that is advanced over the
                // characters as we parse them.
                let mut s = start;
                let mut match_ = true;

                for i in 0..prefix_len {
                    if s >= limit && i > 0 {
                        // We've already matched a character.  This is
                        // a partial match, so we return if in
                        // incremental mode.  In non-incremental mode,
                        // go to the next spec.
                        if is_incremental {
                            break 'loop_;
                        }
                        match_ = false;
                        break;
                    }
                    let c = text.char_at(s);
                    s += 1;
                    if c != spec[ipat + i] {
                        match_ = false;
                        break;
                    }
                }

                if match_ {
                    let mut u = 0i32;
                    let mut digit_count = 0;
                    loop {
                        if s >= limit {
                            // Check for partial match in incremental mode.
                            if s > start && is_incremental {
                                break 'loop_;
                            }
                            break;
                        }
                        let ch = text.char32_at(s);
                        let digit = UCharacter::digit_radix(ch, radix);
                        if digit < 0 {
                            break;
                        }
                        s += UTF16::get_char_count(ch);
                        u = u.wrapping_mul(radix).wrapping_add(digit);
                        digit_count += 1;
                        if digit_count == max_digits {
                            break;
                        }
                    }

                    match_ = digit_count >= min_digits;

                    if match_ {
                        for i in 0..suffix_len {
                            if s >= limit {
                                // Check for partial match in incremental mode.
                                if s > start && is_incremental {
                                    break 'loop_;
                                }
                                match_ = false;
                                break;
                            }
                            let c = text.char_at(s);
                            s += 1;
                            if c != spec[ipat + prefix_len + i] {
                                match_ = false;
                                break;
                            }
                        }

                        if match_ {
                            // At this point, we have a match
                            let str = UTF16::value_of(u)?;
                            text.replace(start, s, &str);
                            limit -= s - start - str.len() as i32;
                            // The following break statement leaves the
                            // loop that is traversing the forms in
                            // spec[].  We then parse the next input
                            // character.
                            break;
                        }
                    }
                }

                ipat += prefix_len + suffix_len;
            }

            if start < limit {
                start += UTF16::get_char_count(text.char32_at(start));
            }
        }

        pos.context_limit += limit - pos.limit;
        pos.limit = limit;
        pos.start = start;
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(&self, input_filter: &UnicodeSet, source_set: &mut UnicodeSet, target_set: &mut UnicodeSet) -> Result<()> {
        // Each form consists of a prefix, suffix,
        // * radix, minimum digit count, and maximum digit count.  These
        // * values are stored as a five character header. ...
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        let mut items = UnicodeSet::new();
        let mut buffer = Vec::new();
        let spec = &self.spec;
        let mut i = 0usize;
        while spec[i] != END {
            // first 5 items are header
            let end = i + spec[i] as usize + spec[i + 1] as usize + 5;
            let radix = spec[i + 2] as i32;
            for j in 0..radix {
                Utility::append_number(&mut buffer, j, radix, 0)?;
            }
            // then add the characters
            for &c in &spec[i + 5..end] {
                items.add(c as i32)?;
            }
            // and go to next block
            i = end;
        }
        items.add_all_str(&buffer)?;
        items.retain_all(&my_filter)?;

        if items.size() > 0 {
            source_set.add_all(&items)?;
            target_set.add_all_range(0, 0x10FFFF)?; // assume we can produce any character
        }
        Ok(())
    }
}
