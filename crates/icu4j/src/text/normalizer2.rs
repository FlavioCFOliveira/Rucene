// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Normalizer2`.

use std::fmt::Debug;
use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::InputStream;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::norm2_all_modes::{Norm2AllModes, Normalizer2WithImpl};
use crate::text::normalizer::QuickCheckResult;
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;

/// Constants for normalization modes.
///
/// Equivalent to the enum `Normalizer2.Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// `COMPOSE`: decomposition followed by composition (NFC, NFKC, ...).
    Compose,
    /// `DECOMPOSE`: map and reorder canonically (NFD, NFKD, ...).
    Decompose,
    /// `FCD`: "Fast C or D" form.
    Fcd,
    /// `COMPOSE_CONTIGUOUS`: compose only contiguously (FCC).
    ComposeContiguous,
}

/// Unicode normalization functionality for standard Unicode normalization
/// or for using custom mapping tables.
///
/// Equivalent to the abstract class `com.ibm.icu.text.Normalizer2`.
/// Java's `StringBuilder` and `Appendable` destinations are
/// `&mut Vec<u16>`: [`normalize`](Self::normalize) is
/// `normalize(CharSequence, StringBuilder)` (which clears the destination)
/// and [`normalize_append`](Self::normalize_append) is
/// `normalize(CharSequence, Appendable)`. Java's
/// `IllegalArgumentException` for a destination that is the source cannot
/// occur (Rust does not allow the aliasing). The methods that take a code
/// point return `Result` because `FilteredNormalizer2` rejects an invalid
/// code point (through `UnicodeSet.contains`).
pub trait Normalizer2: Debug + Send + Sync {
    /// `normalize(CharSequence src)`: the normalized form of the source
    /// string (Java's `String` branch, which spans the already-normalized
    /// prefix first).
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn normalize_str(&self, src: &[u16]) -> Result<Vec<u16>> {
        // Fastpath: Do not construct a new String if the src is a String
        // and is already normalized.
        let span_length = self.span_quick_check_yes(src)?;
        if span_length == src.len() as i32 {
            return Ok(src.to_vec());
        }
        if span_length != 0 {
            let mut sb = Vec::with_capacity(src.len());
            sb.extend_from_slice(&src[..span_length as usize]);
            self.normalize_second_and_append(&mut sb, &src[span_length as usize..])?;
            return Ok(sb);
        }
        let mut dest = Vec::with_capacity(src.len());
        self.normalize(src, &mut dest)?;
        Ok(dest)
    }

    /// `normalize(CharSequence src, StringBuilder dest)`: writes the
    /// normalized form of the source string to the destination (replacing
    /// its contents).
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn normalize(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()>;

    /// `normalize(CharSequence src, Appendable dest)`: appends the
    /// normalized form of the source string to the destination.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn normalize_append(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()>;

    /// `normalizeSecondAndAppend(StringBuilder first, CharSequence second)`:
    /// appends the normalized form of the second string to the first
    /// string (merging them at the boundary).
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn normalize_second_and_append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()>;

    /// `append(StringBuilder first, CharSequence second)`: appends the
    /// second string to the first string (merging them at the boundary).
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()>;

    /// `getDecomposition(int c)`: the decomposition mapping of c, `None`
    /// when none.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn get_decomposition(&self, c: i32) -> Result<Option<Vec<u16>>>;

    /// `getRawDecomposition(int c)`: the raw decomposition mapping of c.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn get_raw_decomposition(&self, _c: i32) -> Result<Option<Vec<u16>>> {
        Ok(None)
    }

    /// `composePair(int a, int b)`: the composite of a+b, or -1.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn compose_pair(&self, _a: i32, _b: i32) -> Result<i32> {
        Ok(-1)
    }

    /// `getCombiningClass(int c)`: the combining class of c.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn get_combining_class(&self, _c: i32) -> Result<i32> {
        Ok(0)
    }

    /// `isNormalized(CharSequence s)`.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn is_normalized(&self, s: &[u16]) -> Result<bool>;

    /// `quickCheck(CharSequence s)`.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn quick_check(&self, s: &[u16]) -> Result<QuickCheckResult>;

    /// `spanQuickCheckYes(CharSequence s)`: the end of the normalized
    /// prefix.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn span_quick_check_yes(&self, s: &[u16]) -> Result<i32>;

    /// `hasBoundaryBefore(int c)`.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn has_boundary_before(&self, c: i32) -> Result<bool>;

    /// `hasBoundaryAfter(int c)`.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn has_boundary_after(&self, c: i32) -> Result<bool>;

    /// `isInert(int c)`.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn is_inert(&self, c: i32) -> Result<bool>;

    /// This normalizer as a `Norm2AllModes.Normalizer2WithImpl`, for Java's
    /// casts; `None` for other classes.
    fn as_with_impl(&self) -> Option<&Normalizer2WithImpl> {
        None
    }
}

/// `getNFCInstance()`: the NFC instance.
///
/// # Errors
///
/// The exception of loading the data.
pub fn get_nfc_instance() -> Result<Arc<Normalizer2WithImpl>> {
    Ok(Norm2AllModes::get_nfc_instance()?.comp.clone())
}

/// `getNFDInstance()`: the NFD instance.
///
/// # Errors
///
/// The exception of loading the data.
pub fn get_nfd_instance() -> Result<Arc<Normalizer2WithImpl>> {
    Ok(Norm2AllModes::get_nfc_instance()?.decomp.clone())
}

/// `getNFKCInstance()`: the NFKC instance.
///
/// # Errors
///
/// The exception of loading the data.
pub fn get_nfkc_instance() -> Result<Arc<Normalizer2WithImpl>> {
    Ok(Norm2AllModes::get_nfkc_instance()?.comp.clone())
}

/// `getNFKDInstance()`: the NFKD instance.
///
/// # Errors
///
/// The exception of loading the data.
pub fn get_nfkd_instance() -> Result<Arc<Normalizer2WithImpl>> {
    Ok(Norm2AllModes::get_nfkc_instance()?.decomp.clone())
}

/// `getNFKCCasefoldInstance()`: the NFKC_Casefold instance.
///
/// # Errors
///
/// The exception of loading the data.
pub fn get_nfkc_casefold_instance() -> Result<Arc<Normalizer2WithImpl>> {
    Ok(Norm2AllModes::get_nfkc_cf_instance()?.comp.clone())
}

/// `getNFKCSimpleCasefoldInstance()`: the NFKC_Simple_Casefold instance.
///
/// # Errors
///
/// The exception of loading the data.
pub fn get_nfkc_simple_casefold_instance() -> Result<Arc<Normalizer2WithImpl>> {
    Ok(Norm2AllModes::get_nfkc_scf_instance()?.comp.clone())
}

/// `getInstance(InputStream data, String name, Mode mode)`: a normalizer
/// for custom data (`data`, closed after reading) or the named ICU data
/// (`data` `None`).
///
/// # Errors
///
/// `ICUUncheckedIOException` wrapping a read error; the errors of loading
/// the data.
pub fn get_instance(data: Option<&mut dyn InputStream>, name: &str, mode: Mode) -> Result<Arc<Normalizer2WithImpl>> {
    let mut bytes = None;
    if let Some(data) = data {
        match ICUBinary::get_byte_buffer_from_input_stream_and_close_stream(data) {
            Ok(b) => bytes = Some(b),
            Err(e) if e.is_io_exception() => return Err(ICUUncheckedIOException::with_cause(e).error()),
            Err(e) => return Err(e),
        }
    }
    let all2_modes = Norm2AllModes::get_instance(bytes, name)?;
    Ok(match mode {
        Mode::Compose => all2_modes.comp.clone(),
        Mode::Decompose => all2_modes.decomp.clone(),
        Mode::Fcd => all2_modes.fcd.clone(),
        Mode::ComposeContiguous => all2_modes.fcc.clone(),
    })
}
