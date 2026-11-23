// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LanguageTag`.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use crate::r#impl::locale::ascii_util::{
    len16, substring16, substring16_from, to_lower_case_default_locale,
    to_upper_case_default_locale, AsciiUtil, CaseInsensitiveKey,
};
use crate::r#impl::locale::base_locale::BaseLocale;
use crate::r#impl::locale::locale_extensions::LocaleExtensions;
use crate::r#impl::locale::parse_status::ParseStatus;
use crate::r#impl::locale::string_token_iterator::StringTokenIterator;

/// A BCP 47 language tag, parsed into its subtags.
///
/// Equivalent to `com.ibm.icu.impl.locale.LanguageTag` (`JDKIMPL` is
/// `false`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageTag {
    language: String,
    script: String,
    region: String,
    privateuse: String,
    extlangs: Vec<String>,
    variants: Vec<String>,
    extensions: Vec<String>,
}

/// `LEGACY`: the legacy tags ("Type: grandfathered" in BCP 47) and their
/// preferred mappings, by case-insensitive tag.
static LEGACY: LazyLock<HashMap<CaseInsensitiveKey, [&'static str; 2]>> = LazyLock::new(|| {
    // legacy        = irregular           ; non-redundant tags registered
    //               / regular             ; during the RFC 3066 era
    //
    // irregular     = "en-GB-oed"         ; irregular tags do not match
    //               / "i-ami"             ; the 'langtag' production and
    //               / "i-bnn"             ; would not otherwise be
    //               / "i-default"         ; considered 'well-formed'
    //               / "i-enochian"        ; These tags are all valid,
    //               / "i-hak"             ; but most are deprecated
    //               / "i-klingon"         ; in favor of more modern
    //               / "i-lux"             ; subtags or subtag
    //               / "i-mingo"           ; combination
    //               / "i-navajo"
    //               / "i-pwn"
    //               / "i-tao"
    //               / "i-tay"
    //               / "i-tsu"
    //               / "sgn-BE-FR"
    //               / "sgn-BE-NL"
    //               / "sgn-CH-DE"
    //
    // regular       = "art-lojban"        ; these tags match the 'langtag'
    //               / "cel-gaulish"       ; production, but their subtags
    //               / "no-bok"            ; are not extended language
    //               / "no-nyn"            ; or variant subtags: their meaning
    //               / "zh-guoyu"          ; is defined by their registration
    //               / "zh-hakka"          ; and all of these are deprecated
    //               / "zh-min"            ; in favor of a more modern
    //               / "zh-min-nan"        ; subtag or sequence of subtags
    //               / "zh-xiang"
    let entries: [[&str; 2]; 26] = [
        //{"tag",         "preferred"},
        ["art-lojban", "jbo"],
        ["cel-gaulish", "xtg"],  // fallback
        ["en-GB-oed", "en-GB-x-oed"], // fallback
        ["i-ami", "ami"],
        ["i-bnn", "bnn"],
        ["i-default", "en-x-i-default"],   // fallback
        ["i-enochian", "und-x-i-enochian"], // fallback
        ["i-hak", "hak"],
        ["i-klingon", "tlh"],
        ["i-lux", "lb"],
        ["i-mingo", "see-x-i-mingo"], // fallback
        ["i-navajo", "nv"],
        ["i-pwn", "pwn"],
        ["i-tao", "tao"],
        ["i-tay", "tay"],
        ["i-tsu", "tsu"],
        ["no-bok", "nb"],
        ["no-nyn", "nn"],
        ["sgn-BE-FR", "sfb"],
        ["sgn-BE-NL", "vgt"],
        ["sgn-CH-DE", "sgg"],
        ["zh-guoyu", "cmn"],
        ["zh-hakka", "hak"],
        ["zh-min", "nan-x-zh-min"], // fallback
        ["zh-min-nan", "nan"],
        ["zh-xiang", "hsn"],
    ];
    let mut m = HashMap::new();
    for e in entries {
        m.insert(CaseInsensitiveKey::new(e[0]), e);
    }
    m
});

impl LanguageTag {
    /// `SEP`.
    pub const SEP: &'static str = "-";
    /// `PRIVATEUSE`.
    pub const PRIVATEUSE: &'static str = "x";
    /// `UNDETERMINED` (a non-final public static in Java that ICU never
    /// reassigns).
    pub const UNDETERMINED: &'static str = "und";
    /// `PRIVUSE_VARIANT_PREFIX`.
    pub const PRIVUSE_VARIANT_PREFIX: &'static str = "lvariant";

    /// `parse(String languageTag, ParseStatus sts)`: `sts` `None` is a
    /// fresh status, as Java's `null`.
    pub fn parse(language_tag: &str, sts: Option<&mut ParseStatus>) -> LanguageTag {
        let mut local = ParseStatus::new();
        let sts = match sts {
            Some(s) => {
                s.reset();
                s
            }
            None => &mut local,
        };

        let mut is_legacy = false;
        let mut gfmap = LEGACY.get(&CaseInsensitiveKey::new(language_tag));
        // Language tag is at least 2 alpha so we can skip searching the first 2 chars.
        let units: Vec<u16> = language_tag.encode_utf16().collect();
        let mut dash = 2i32;
        while gfmap.is_none() {
            dash = crate::jdk::index_of_unit(&units, b'-' as u16, dash + 1);
            if dash == -1 {
                break;
            }
            gfmap = LEGACY.get(&CaseInsensitiveKey::new(&substring16(language_tag, 0, dash)));
        }

        let mut itr = if let Some(gfmap) = gfmap {
            is_legacy = true;
            if len16(gfmap[0]) == len16(language_tag) {
                // use preferred mapping
                StringTokenIterator::new(gfmap[1], Self::SEP)
            } else {
                // append the rest of the tag.
                StringTokenIterator::new(
                    &format!("{}{}", gfmap[1], substring16_from(language_tag, dash)),
                    Self::SEP,
                )
            }
        } else {
            StringTokenIterator::new(language_tag, Self::SEP)
        };

        let mut tag = LanguageTag::default();

        // langtag must start with either language or privateuse
        if tag.parse_language(&mut itr, sts) {
            // ExtLang can only be preceded by 2-3 letter language subtag.
            if len16(&tag.language) <= 3 {
                tag.parse_extlangs(&mut itr, sts);
            }
            tag.parse_script(&mut itr, sts);
            tag.parse_region(&mut itr, sts);
            tag.parse_variants(&mut itr, sts);
            tag.parse_extensions(&mut itr, sts);
        }
        tag.parse_privateuse(&mut itr, sts);

        if is_legacy {
            // A legacy tag is replaced with a well-formed tag above.
            // However, the parsed length must be the original tag length.
            debug_assert!(itr.is_done());
            debug_assert!(!sts.is_error());
            sts.parse_length = len16(language_tag);
        } else if !itr.is_done() && !sts.is_error() {
            let s = itr.cur().to_string();
            sts.error_index = itr.current_start();
            if s.is_empty() {
                sts.error_msg = Some("Empty subtag".to_string());
            } else {
                sts.error_msg = Some(format!("Invalid subtag: {s}"));
            }
        }

        tag
    }

    //
    // Language subtag parsers
    //

    fn parse_language(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        let s = itr.cur().to_string();
        if Self::is_language(&s) {
            found = true;
            self.language = s;
            sts.parse_length = itr.current_end();
            itr.next();
        }
        found
    }

    fn parse_extlangs(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        while !itr.is_done() {
            let s = itr.cur().to_string();
            if !Self::is_extlang(&s) {
                break;
            }
            found = true;
            self.extlangs.push(s);
            sts.parse_length = itr.current_end();
            itr.next();

            if self.extlangs.len() == 3 {
                // Maximum 3 extlangs
                break;
            }
        }
        found
    }

    fn parse_script(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        let s = itr.cur().to_string();
        if Self::is_script(&s) {
            found = true;
            self.script = s;
            sts.parse_length = itr.current_end();
            itr.next();
        }
        found
    }

    fn parse_region(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        let s = itr.cur().to_string();
        if Self::is_region(&s) {
            found = true;
            self.region = s;
            sts.parse_length = itr.current_end();
            itr.next();
        }
        found
    }

    fn parse_variants(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        while !itr.is_done() {
            let s = itr.cur().to_string();
            if !Self::is_variant(&s) {
                break;
            }
            found = true;
            // Ignore repeated variant
            let s = to_upper_case_default_locale(&s);
            if !self.variants.contains(&s) {
                self.variants.push(s);
            }
            sts.parse_length = itr.current_end();
            itr.next();
        }
        found
    }

    fn parse_extensions(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        while !itr.is_done() {
            let s = itr.cur().to_string();
            if Self::is_extension_singleton(&s) {
                let start = itr.current_start();
                let singleton = to_lower_case_default_locale(&s);
                let mut sb = singleton.clone();

                itr.next();
                while !itr.is_done() {
                    let s = itr.cur().to_string();
                    if Self::is_extension_subtag(&s) {
                        sb.push_str(Self::SEP);
                        sb.push_str(&s);
                        sts.parse_length = itr.current_end();
                    } else {
                        break;
                    }
                    itr.next();
                }

                if sts.parse_length <= start {
                    sts.error_index = start;
                    sts.error_msg = Some(format!("Incomplete extension '{singleton}'"));
                    break;
                }

                // Ignore the extension if it is already in _extensions.
                let first = sb.encode_utf16().next();
                let mut already_has = false;
                for extension in &self.extensions {
                    already_has |= extension.encode_utf16().next() == first;
                }
                if !already_has {
                    self.extensions.push(sb);
                }
                found = true;
            } else {
                break;
            }
        }
        found
    }

    fn parse_privateuse(&mut self, itr: &mut StringTokenIterator, sts: &mut ParseStatus) -> bool {
        if itr.is_done() || sts.is_error() {
            return false;
        }
        let mut found = false;
        let s = itr.cur().to_string();
        if Self::is_privateuse_prefix(&s) {
            let start = itr.current_start();
            let mut sb = s;

            itr.next();
            while !itr.is_done() {
                let s = itr.cur().to_string();
                if !Self::is_privateuse_subtag(&s) {
                    break;
                }
                sb.push_str(Self::SEP);
                sb.push_str(&s);
                sts.parse_length = itr.current_end();

                itr.next();
            }

            if sts.parse_length <= start {
                // need at least 1 private subtag
                sts.error_index = start;
                sts.error_msg = Some("Incomplete privateuse".to_string());
            } else {
                self.privateuse = sb;
                found = true;
            }
        }
        found
    }

    /// `parseLocale(BaseLocale baseLocale, LocaleExtensions
    /// localeExtensions)`.
    pub fn parse_locale(base_locale: &BaseLocale, locale_extensions: &LocaleExtensions) -> Self {
        let mut tag = LanguageTag::default();

        let mut language = base_locale.get_language().to_string();
        let script = base_locale.get_script();
        let region = base_locale.get_region();
        let variant = base_locale.get_variant();

        let mut has_subtag = false;

        let mut privuse_var: Option<String> = None; // store ill-formed variant subtags

        if !language.is_empty() && Self::is_language(&language) {
            // Convert a deprecated language code used by Java to
            // a new code
            if language == "iw" {
                language = "he".to_string();
            } else if language == "ji" {
                language = "yi".to_string();
            } else if language == "in" {
                language = "id".to_string();
            }
            tag.language = language;
        }

        if !script.is_empty() && Self::is_script(script) {
            tag.script = Self::canonicalize_script(script);
            has_subtag = true;
        }

        if !region.is_empty() && Self::is_region(region) {
            tag.region = Self::canonicalize_region(region);
            has_subtag = true;
        }

        if !variant.is_empty() {
            let mut variants: Option<Vec<String>> = None;
            let mut varitr = StringTokenIterator::new(variant, BaseLocale::SEP);
            while !varitr.is_done() {
                let var = varitr.cur().to_string();
                if !Self::is_variant(&var) {
                    break;
                }
                variants
                    .get_or_insert_with(Vec::new)
                    .push(Self::canonicalize_variant(&var));
                varitr.next();
            }
            if let Some(variants) = variants {
                tag.variants = variants;
                has_subtag = true;
            }
            if !varitr.is_done() {
                // ill-formed variant subtags
                let mut buf = String::new();
                while !varitr.is_done() {
                    let prvv = varitr.cur().to_string();
                    if !Self::is_privateuse_subtag(&prvv) {
                        // cannot use private use subtag - truncated
                        break;
                    }
                    if !buf.is_empty() {
                        buf.push_str(Self::SEP);
                    }
                    buf.push_str(&AsciiUtil::to_lower_string(&prvv));
                    varitr.next();
                }
                if !buf.is_empty() {
                    privuse_var = Some(buf);
                }
            }
        }

        let mut extensions: Option<Vec<String>> = None;
        let mut privateuse: Option<String> = None;

        for locext_key in locale_extensions.get_keys() {
            let ext = locale_extensions
                .get_extension(locext_key)
                .expect("a key of the extensions has its extension");
            if Self::is_privateuse_prefix_char(locext_key) {
                privateuse = Some(ext.get_value().to_string());
            } else {
                extensions.get_or_insert_with(Vec::new).push(format!(
                    "{}{}{}",
                    locext_key,
                    Self::SEP,
                    ext.get_value()
                ));
            }
        }

        if let Some(extensions) = extensions {
            tag.extensions = extensions;
            has_subtag = true;
        }

        // append ill-formed variant subtags to private use
        if let Some(privuse_var) = privuse_var {
            privateuse = Some(match privateuse {
                None => format!("{}{}{}", Self::PRIVUSE_VARIANT_PREFIX, Self::SEP, privuse_var),
                Some(p) => format!(
                    "{}{}{}{}{}",
                    p,
                    Self::SEP,
                    Self::PRIVUSE_VARIANT_PREFIX,
                    Self::SEP,
                    privuse_var.replace(BaseLocale::SEP, Self::SEP)
                ),
            });
        }

        if let Some(p) = &privateuse {
            tag.privateuse = p.clone();
        }

        if tag.language.is_empty() && (has_subtag || privateuse.is_none()) {
            // use lang "und" when 1) no language is available AND
            // 2) any of other subtags other than private use are available or
            // no private use tag is available
            tag.language = Self::UNDETERMINED.to_string();
        }

        tag
    }

    //
    // Getter methods for language subtag fields
    //

    /// `getLanguage()`.
    pub fn get_language(&self) -> &str {
        &self.language
    }

    /// `getExtlangs()`.
    pub fn get_extlangs(&self) -> &[String] {
        &self.extlangs
    }

    /// `getScript()`.
    pub fn get_script(&self) -> &str {
        &self.script
    }

    /// `getRegion()`.
    pub fn get_region(&self) -> &str {
        &self.region
    }

    /// `getVariants()`.
    pub fn get_variants(&self) -> &[String] {
        &self.variants
    }

    /// `getExtensions()`.
    pub fn get_extensions(&self) -> &[String] {
        &self.extensions
    }

    /// `getPrivateuse()`.
    pub fn get_privateuse(&self) -> &str {
        &self.privateuse
    }

    //
    // Language subtag syntax checking methods
    //

    /// `isLanguage(String s)`.
    pub fn is_language(s: &str) -> bool {
        // language      = 2*3ALPHA            ; shortest ISO 639 code
        //                 ["-" extlang]       ; sometimes followed by
        //                                     ;   extended language subtags
        //               / 4ALPHA              ; or reserved for future use
        //               / 5*8ALPHA            ; or registered language subtag
        let n = len16(s);
        (2..=8).contains(&n) && AsciiUtil::is_alpha_string(s)
    }

    /// `isExtlang(String s)`.
    pub fn is_extlang(s: &str) -> bool {
        // extlang       = 3ALPHA              ; selected ISO 639 codes
        //                 *2("-" 3ALPHA)      ; permanently reserved
        len16(s) == 3 && AsciiUtil::is_alpha_string(s)
    }

    /// `isScript(String s)`.
    pub fn is_script(s: &str) -> bool {
        // script        = 4ALPHA              ; ISO 15924 code
        len16(s) == 4 && AsciiUtil::is_alpha_string(s)
    }

    /// `isRegion(String s)`.
    pub fn is_region(s: &str) -> bool {
        // region        = 2ALPHA              ; ISO 3166-1 code
        //               / 3DIGIT              ; UN M.49 code
        let n = len16(s);
        (n == 2 && AsciiUtil::is_alpha_string(s)) || (n == 3 && AsciiUtil::is_numeric_string(s))
    }

    /// `isVariant(String s)`.
    pub fn is_variant(s: &str) -> bool {
        // variant       = 5*8alphanum         ; registered variants
        //               / (DIGIT 3alphanum)
        let len = len16(s);
        if (5..=8).contains(&len) {
            return AsciiUtil::is_alpha_numeric_string(s);
        }
        if len == 4 {
            let b = s.as_bytes();
            return b[0].is_ascii_digit()
                && b[1].is_ascii_alphanumeric()
                && b[2].is_ascii_alphanumeric()
                && b[3].is_ascii_alphanumeric();
        }
        false
    }

    /// `isTKey(String s)`.
    pub fn is_t_key(s: &str) -> bool {
        // tkey        =  = alpha digit ;
        let b = s.as_bytes();
        len16(s) == 2 && b[0].is_ascii_alphabetic() && b[1].is_ascii_digit()
    }

    /// `isExtensionSingleton(String s)`.
    pub fn is_extension_singleton(s: &str) -> bool {
        // singleton     = DIGIT               ; 0 - 9
        //               / %x41-57             ; A - W
        //               / %x59-5A             ; Y - Z
        //               / %x61-77             ; a - w
        //               / %x79-7A             ; y - z

        len16(s) == 1
            && AsciiUtil::is_alpha_numeric_string(s)
            && !AsciiUtil::case_ignore_match(Self::PRIVATEUSE, s)
    }

    /// `isExtensionSingletonChar(char c)`.
    pub fn is_extension_singleton_char(c: char) -> bool {
        Self::is_extension_singleton(c.encode_utf8(&mut [0u8; 4]))
    }

    /// `isExtensionSubtag(String s)`.
    pub fn is_extension_subtag(s: &str) -> bool {
        // extension     = singleton 1*("-" (2*8alphanum))
        let n = len16(s);
        (2..=8).contains(&n) && AsciiUtil::is_alpha_numeric_string(s)
    }

    /// `isPrivateusePrefix(String s)`.
    pub fn is_privateuse_prefix(s: &str) -> bool {
        // privateuse    = "x" 1*("-" (1*8alphanum))
        len16(s) == 1 && AsciiUtil::case_ignore_match(Self::PRIVATEUSE, s)
    }

    /// `isPrivateusePrefixChar(char c)`.
    pub fn is_privateuse_prefix_char(c: char) -> bool {
        AsciiUtil::case_ignore_match(Self::PRIVATEUSE, c.encode_utf8(&mut [0u8; 4]))
    }

    /// `isPrivateuseSubtag(String s)`.
    pub fn is_privateuse_subtag(s: &str) -> bool {
        // privateuse    = "x" 1*("-" (1*8alphanum))
        let n = len16(s);
        (1..=8).contains(&n) && AsciiUtil::is_alpha_numeric_string(s)
    }

    //
    // Language subtag canonicalization methods
    //

    /// `canonicalizeLanguage(String s)`.
    pub fn canonicalize_language(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }

    /// `canonicalizeExtlang(String s)`.
    pub fn canonicalize_extlang(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }

    /// `canonicalizeScript(String s)`.
    pub fn canonicalize_script(s: &str) -> String {
        AsciiUtil::to_title_string(s)
    }

    /// `canonicalizeRegion(String s)`.
    pub fn canonicalize_region(s: &str) -> String {
        AsciiUtil::to_upper_string(s)
    }

    /// `canonicalizeVariant(String s)`.
    pub fn canonicalize_variant(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }

    /// `canonicalizeExtension(String s)`.
    pub fn canonicalize_extension(s: &str) -> String {
        let mut s = AsciiUtil::to_lower_string(s);
        if s.starts_with("u-") {
            while s.ends_with("-true") {
                s.truncate(s.len() - 5); // length of "-true" is 5
            }
            while let Some(found) = s.find("-true-").filter(|&f| f > 0) {
                s = format!("{}{}", &s[..found], &s[found + 5..]); // length of "-true" is 5
            }
            while s.ends_with("-yes") {
                s.truncate(s.len() - 4); // length of "-yes" is 4
            }
            while let Some(found) = s.find("-yes-").filter(|&f| f > 0) {
                s = format!("{}{}", &s[..found], &s[found + 4..]); // length of "-yes" is 5
            }
        }
        s
    }

    /// `canonicalizeExtensionSingleton(String s)`.
    pub fn canonicalize_extension_singleton(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }

    /// `canonicalizeExtensionSubtag(String s)`.
    pub fn canonicalize_extension_subtag(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }

    /// `canonicalizePrivateuse(String s)`.
    pub fn canonicalize_privateuse(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }

    /// `canonicalizePrivateuseSubtag(String s)`.
    pub fn canonicalize_privateuse_subtag(s: &str) -> String {
        AsciiUtil::to_lower_string(s)
    }
}

/// `toString()`: the tag, subtags joined with `-`.
impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sb = String::new();

        if !self.language.is_empty() {
            sb.push_str(&self.language);

            for extlang in &self.extlangs {
                sb.push_str(Self::SEP);
                sb.push_str(extlang);
            }

            if !self.script.is_empty() {
                sb.push_str(Self::SEP);
                sb.push_str(&self.script);
            }

            if !self.region.is_empty() {
                sb.push_str(Self::SEP);
                sb.push_str(&self.region);
            }

            for variant in &self.variants {
                sb.push_str(Self::SEP);
                sb.push_str(variant);
            }

            for extension in &self.extensions {
                sb.push_str(Self::SEP);
                sb.push_str(extension);
            }
        }
        if !self.privateuse.is_empty() {
            if !sb.is_empty() {
                sb.push_str(Self::SEP);
            }
            sb.push_str(&self.privateuse);
        }

        f.write_str(&sb)
    }
}
