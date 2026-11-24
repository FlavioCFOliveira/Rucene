// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.LocalizedNumberFormatterAsFormat`.

use std::any::Any;

use rucene::error::{LuceneError, Result};

use crate::jdk::text::{AttributedCharacterIterator, FieldPosition, Format, ParsedObject};
use crate::jdk::{DataOutputStream, InputStream, ParsePosition};
use crate::number::localized_number_formatter::LocalizedNumberFormatter;
use crate::number::number_formatter::NumberFormatter;
use crate::number::number_formatter_settings::NumberFormatterSettings;
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::formatted_value_string_builder_impl::FormattedValueStringBuilderImpl;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{DecimalQuantity_DualStorageBCD, JavaNumber};
use crate::text::formatted_value::FormattedValue;
use crate::util::u_locale::ULocale;

/// A wrapper around LocalizedNumberFormatter implementing the Format
/// interface, enabling improved compatibility with other APIs.
///
/// Equivalent to `com.ibm.icu.impl.number.LocalizedNumberFormatterAsFormat`
/// (a `java.text.Format`). Java's `Object` arguments that are numbers are
/// [`JavaNumber`] values. `hashCode()` is [`hash_code`](Self::hash_code),
/// `equals(Object)` [`java_equals`](Self::java_equals); Java serialization
/// replaces the object by a [`Proxy`] ([`write_replace`](Self::write_replace)).
#[derive(Debug, Clone)]
pub struct LocalizedNumberFormatterAsFormat {
    /// `formatter` (transient).
    formatter: LocalizedNumberFormatter,
    /// `locale` (transient).
    locale: Option<ULocale>,
}

impl LocalizedNumberFormatterAsFormat {
    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = 1;

    /// Equivalent to `LocalizedNumberFormatterAsFormat(
    /// LocalizedNumberFormatter, ULocale)`.
    pub fn new(formatter: LocalizedNumberFormatter, locale: Option<ULocale>) -> Self {
        LocalizedNumberFormatterAsFormat { formatter, locale }
    }

    /// Equivalent to `getNumberFormatter()`.
    pub fn get_number_formatter(&self) -> &LocalizedNumberFormatter {
        &self.formatter
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        self.formatter.hash_code()
    }

    /// Equivalent to `equals(Object)` for another object of this class.
    pub fn java_equals(&self, other: &LocalizedNumberFormatterAsFormat) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        self.formatter.java_equals(other.get_number_formatter())
    }

    /// Equivalent to `writeReplace()`.
    ///
    /// # Errors
    ///
    /// The `UnsupportedOperationException` of `toSkeleton`.
    pub fn write_replace(&self) -> Result<Proxy> {
        let locale = self.locale.as_ref().ok_or_else(|| crate::jdk::null_pointer("locale"))?;
        Ok(Proxy {
            language_tag: Some(locale.to_language_tag()),
            skeleton: Some(crate::jdk::to_rust_string(&self.formatter.to_skeleton()?)),
        })
    }
}

impl Format for LocalizedNumberFormatterAsFormat {
    /// Formats a Number using the wrapped LocalizedNumberFormatter. The
    /// provided `to_append_to` is appended to, and the `pos` is updated
    /// with the first field position.
    fn format_object_to(&self, obj: &dyn Any, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()> {
        let number = match obj.downcast_ref::<JavaNumber>() {
            Some(n) => n,
            None => return Err(crate::jdk::illegal_argument_no_message()),
        };
        let mut dq = DecimalQuantity_DualStorageBCD::from_number(number)?;
        let mut string = FormattedStringBuilder::new();
        self.formatter.format_impl(&mut dq, &mut string)?;
        // always return first occurrence:
        pos.set_begin_index(0);
        pos.set_end_index(0);
        let found = FormattedValueStringBuilderImpl::next_field_position(&string, pos)?;
        if found && !to_append_to.is_empty() {
            pos.set_begin_index(pos.get_begin_index() + to_append_to.len() as i32);
            pos.set_end_index(pos.get_end_index() + to_append_to.len() as i32);
        }
        to_append_to.extend_from_slice(&string.to_java_string());
        Ok(())
    }

    /// Not supported: throws `UnsupportedOperationException`.
    fn parse_object_at(&self, _source: &[u16], _pos: &mut ParsePosition) -> Result<Option<ParsedObject>> {
        Err(crate::jdk::unchecked_no_message("java.lang.UnsupportedOperationException"))
    }

    fn clone_format(&self) -> Box<dyn Format> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Formats a Number using the wrapped LocalizedNumberFormatter. The
    /// returned AttributedCharacterIterator contains the full field position
    /// information.
    fn format_to_character_iterator(&self, obj: &dyn Any) -> Result<Box<dyn AttributedCharacterIterator>> {
        let number = match obj.downcast_ref::<JavaNumber>() {
            Some(n) => n,
            None => return Err(crate::jdk::illegal_argument_no_message()),
        };
        self.formatter.format_number(number)?.to_character_iterator()
    }
}

/// The serialized form of a `LocalizedNumberFormatterAsFormat`: its
/// language tag and skeleton.
///
/// Equivalent to `LocalizedNumberFormatterAsFormat.Proxy` (an
/// `Externalizable`).
#[derive(Debug, Clone, Default)]
pub struct Proxy {
    /// `languageTag`.
    language_tag: Option<String>,
    /// `skeleton`.
    skeleton: Option<String>,
}

impl Proxy {
    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = 1;

    /// Equivalent to `Proxy()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `writeExternal(ObjectOutput)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream; `NullPointerException` for a
    /// missing field (`writeUTF(null)`).
    pub fn write_external(&self, out: &mut DataOutputStream<'_>) -> Result<()> {
        // Older versions of ICU only use 0 in the version tag
        out.write_byte(0)?;
        write_utf(out, self.language_tag.as_deref())?;
        write_utf(out, self.skeleton.as_deref())?;
        Ok(())
    }

    /// Equivalent to `readExternal(ObjectInput)`.
    ///
    /// # Errors
    ///
    /// The `IOException`/`EOFException`/`UTFDataFormatException` of the
    /// stream.
    pub fn read_external(&mut self, input: &mut dyn InputStream) -> Result<()> {
        read_u8(input)?; // version number
        self.language_tag = Some(read_utf(input)?);
        self.skeleton = Some(read_utf(input)?);
        Ok(())
    }

    /// Equivalent to `readResolve()`.
    ///
    /// # Errors
    ///
    /// The errors of the skeleton parsing and of `ULocale.forLanguageTag`.
    pub fn read_resolve(&self) -> Result<LocalizedNumberFormatterAsFormat> {
        let skeleton = self.skeleton.as_deref().ok_or_else(|| crate::jdk::null_pointer("skeleton"))?;
        let language_tag = self.language_tag.as_deref().ok_or_else(|| crate::jdk::null_pointer("languageTag"))?;
        Ok(NumberFormatter::for_skeleton(&crate::jdk::units(skeleton))?
            .locale(&ULocale::for_language_tag(language_tag)?)
            .to_format())
    }
}

/// `DataOutput.writeUTF(String)`.
fn write_utf(out: &mut DataOutputStream<'_>, s: Option<&str>) -> Result<()> {
    let s = s.ok_or_else(LuceneError::null_pointer)?;
    let mut bytes: Vec<u8> = Vec::new();
    for c in s.encode_utf16() {
        let c = c as u32;
        if (0x0001..=0x007f).contains(&c) {
            bytes.push(c as u8);
        } else if c <= 0x07ff {
            bytes.push((0xc0 | ((c >> 6) & 0x1f)) as u8);
            bytes.push((0x80 | (c & 0x3f)) as u8);
        } else {
            bytes.push((0xe0 | ((c >> 12) & 0x0f)) as u8);
            bytes.push((0x80 | ((c >> 6) & 0x3f)) as u8);
            bytes.push((0x80 | (c & 0x3f)) as u8);
        }
    }
    if bytes.len() > 65535 {
        return Err(LuceneError::unchecked(
            "java.io.UTFDataFormatException",
            format!("encoded string ({}...{}) too long: {} bytes", truncate8(s, true), truncate8(s, false), bytes.len()),
        ));
    }
    out.write_short(bytes.len() as i32)?;
    for b in bytes {
        out.write_byte(b as i32)?;
    }
    Ok(())
}

/// The 8-char head or tail `DataOutputStream.writeUTF` quotes in its
/// too-long message.
fn truncate8(s: &str, head: bool) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    let part = if head { &u[..u.len().min(8)] } else { &u[u.len().saturating_sub(8)..] };
    String::from_utf16_lossy(part)
}

fn read_u8(input: &mut dyn InputStream) -> Result<i32> {
    let b = input.read_byte().map_err(LuceneError::from)?;
    if b < 0 {
        return Err(LuceneError::unchecked_with_message("java.io.EOFException", None));
    }
    Ok(b)
}

/// `DataInput.readUTF()`.
fn read_utf(input: &mut dyn InputStream) -> Result<String> {
    let utflen = ((read_u8(input)? << 8) | read_u8(input)?) as usize;
    let mut bytes = vec![0u8; utflen];
    for b in bytes.iter_mut() {
        *b = read_u8(input)? as u8;
    }
    let mut chars: Vec<u16> = Vec::with_capacity(utflen);
    let mut count = 0;
    let malformed = |at: usize| {
        LuceneError::unchecked("java.io.UTFDataFormatException", format!("malformed input around byte {at}"))
    };
    while count < utflen {
        let c = bytes[count] as u32;
        match c >> 4 {
            0..=7 => {
                count += 1;
                chars.push(c as u16);
            }
            12 | 13 => {
                count += 2;
                if count > utflen {
                    return Err(LuceneError::unchecked(
                        "java.io.UTFDataFormatException",
                        "malformed input: partial character at end",
                    ));
                }
                let c2 = bytes[count - 1] as u32;
                if (c2 & 0xc0) != 0x80 {
                    return Err(malformed(count));
                }
                chars.push((((c & 0x1f) << 6) | (c2 & 0x3f)) as u16);
            }
            14 => {
                count += 3;
                if count > utflen {
                    return Err(LuceneError::unchecked(
                        "java.io.UTFDataFormatException",
                        "malformed input: partial character at end",
                    ));
                }
                let c2 = bytes[count - 2] as u32;
                let c3 = bytes[count - 1] as u32;
                if (c2 & 0xc0) != 0x80 || (c3 & 0xc0) != 0x80 {
                    return Err(malformed(count - 1));
                }
                chars.push((((c & 0x0f) << 12) | ((c2 & 0x3f) << 6) | (c3 & 0x3f)) as u16);
            }
            _ => return Err(malformed(count)),
        }
    }
    Ok(String::from_utf16_lossy(&chars))
}
