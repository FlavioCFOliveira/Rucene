// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Norm2AllModes`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::ByteBuffer;
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::normalizer2_impl::{Normalizer2Impl, ReorderingBuffer};
use crate::r#impl::soft_cache::SoftCache;
use crate::text::normalizer::QuickCheckResult;
use crate::text::normalizer2::Normalizer2;

/// A normalizer that leaves the text unchanged.
///
/// Equivalent to `Norm2AllModes.NoopNormalizer2`. Java's
/// `IllegalArgumentException` for `dest == src` cannot occur: Rust does not
/// allow the destination to alias the source.
#[derive(Debug, Default)]
pub struct NoopNormalizer2;

impl Normalizer2 for NoopNormalizer2 {
    fn normalize(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()> {
        dest.clear();
        dest.extend_from_slice(src);
        Ok(())
    }

    fn normalize_append(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()> {
        dest.extend_from_slice(src);
        Ok(())
    }

    fn normalize_second_and_append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()> {
        first.extend_from_slice(second);
        Ok(())
    }

    fn append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()> {
        first.extend_from_slice(second);
        Ok(())
    }

    fn get_decomposition(&self, _c: i32) -> Result<Option<Vec<u16>>> {
        Ok(None)
    }

    // No need to override the default getRawDecomposition().
    fn is_normalized(&self, _s: &[u16]) -> Result<bool> {
        Ok(true)
    }

    fn quick_check(&self, _s: &[u16]) -> Result<QuickCheckResult> {
        Ok(QuickCheckResult::Yes)
    }

    fn span_quick_check_yes(&self, s: &[u16]) -> Result<i32> {
        Ok(s.len() as i32)
    }

    fn has_boundary_before(&self, _c: i32) -> Result<bool> {
        Ok(true)
    }

    fn has_boundary_after(&self, _c: i32) -> Result<bool> {
        Ok(true)
    }

    fn is_inert(&self, _c: i32) -> Result<bool> {
        Ok(true)
    }
}

/// The concrete subclass of `Normalizer2WithImpl` (Java's
/// `DecomposeNormalizer2`, `ComposeNormalizer2` and `FCDNormalizer2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum N2Kind {
    /// `DecomposeNormalizer2`.
    Decompose,
    /// `ComposeNormalizer2(ni, fcc)`; `only_contiguous` is `fcc`.
    Compose {
        /// `onlyContiguous`.
        only_contiguous: bool,
    },
    /// `FCDNormalizer2`.
    Fcd,
}

/// Intermediate class: Has Normalizer2Impl and does boilerplate argument
/// checking and setup.
///
/// Equivalent to the abstract class `Norm2AllModes.Normalizer2WithImpl`
/// and its subclasses `DecomposeNormalizer2`, `ComposeNormalizer2` and
/// `FCDNormalizer2` (selected by [`N2Kind`]).
#[derive(Debug)]
pub struct Normalizer2WithImpl {
    /// `impl`.
    pub impl_: Arc<Normalizer2Impl>,
    kind: N2Kind,
}

impl Normalizer2WithImpl {
    /// `new DecomposeNormalizer2(ni)`, `new ComposeNormalizer2(ni, fcc)` or
    /// `new FCDNormalizer2(ni)`.
    pub fn new(ni: Arc<Normalizer2Impl>, kind: N2Kind) -> Self {
        Self { impl_: ni, kind }
    }

    /// The subclass.
    pub fn kind(&self) -> N2Kind {
        self.kind
    }

    /// `normalize(CharSequence src, ReorderingBuffer buffer)` (protected,
    /// abstract).
    fn normalize_buffer(&self, src: &[u16], buffer: &mut ReorderingBuffer<'_>) {
        match self.kind {
            N2Kind::Decompose => {
                self.impl_.decompose(src, 0, src.len() as i32, Some(buffer));
            }
            N2Kind::Compose { only_contiguous } => {
                self.impl_
                    .compose(src, 0, src.len() as i32, only_contiguous, true, buffer);
            }
            N2Kind::Fcd => {
                self.impl_.make_fcd(src, 0, src.len() as i32, Some(buffer));
            }
        }
    }

    /// `normalizeAndAppend(CharSequence src, boolean doNormalize,
    /// ReorderingBuffer buffer)` (protected, abstract).
    fn normalize_and_append(
        &self,
        src: &[u16],
        do_normalize: bool,
        buffer: &mut ReorderingBuffer<'_>,
    ) {
        match self.kind {
            N2Kind::Decompose => self.impl_.decompose_and_append(src, do_normalize, buffer),
            N2Kind::Compose { only_contiguous } => {
                self.impl_
                    .compose_and_append(src, do_normalize, only_contiguous, buffer)
            }
            N2Kind::Fcd => self.impl_.make_fcd_and_append(src, do_normalize, buffer),
        }
    }

    /// `normalizeSecondAndAppend(StringBuilder first, CharSequence second,
    /// boolean doNormalize)`.
    pub fn normalize_second_and_append_do(
        &self,
        first: &mut Vec<u16>,
        second: &[u16],
        do_normalize: bool,
    ) {
        let cap = first.len() as i32 + second.len() as i32;
        let mut buffer = ReorderingBuffer::new(&self.impl_, first, cap);
        self.normalize_and_append(second, do_normalize, &mut buffer);
    }

    /// `getCombiningClass(int c)`.
    pub fn get_combining_class(&self, c: i32) -> i32 {
        self.impl_.get_cc(self.impl_.get_norm16(c))
    }

    /// `getQuickCheck(int c)` (abstract in Java).
    pub fn get_quick_check(&self, c: i32) -> i32 {
        match self.kind {
            N2Kind::Decompose | N2Kind::Fcd => {
                if self.impl_.is_decomp_yes(self.impl_.get_norm16(c)) {
                    1
                } else {
                    0
                }
            }
            N2Kind::Compose { .. } => self.impl_.get_comp_quick_check(self.impl_.get_norm16(c)),
        }
    }

    /// `isInert(int c)`.
    pub fn is_inert(&self, c: i32) -> bool {
        match self.kind {
            N2Kind::Decompose => self.impl_.is_decomp_inert(c),
            N2Kind::Compose { only_contiguous } => self.impl_.is_comp_inert(c, only_contiguous),
            N2Kind::Fcd => self.impl_.is_fcd_inert(c),
        }
    }

    /// `hasBoundaryBefore(int c)`.
    pub fn has_boundary_before(&self, c: i32) -> bool {
        match self.kind {
            N2Kind::Decompose => self.impl_.has_decomp_boundary_before(c),
            N2Kind::Compose { .. } => self.impl_.has_comp_boundary_before(c),
            N2Kind::Fcd => self.impl_.has_fcd_boundary_before(c),
        }
    }

    /// `hasBoundaryAfter(int c)`.
    pub fn has_boundary_after(&self, c: i32) -> bool {
        match self.kind {
            N2Kind::Decompose => self.impl_.has_decomp_boundary_after(c),
            N2Kind::Compose { only_contiguous } => {
                self.impl_.has_comp_boundary_after(c, only_contiguous)
            }
            N2Kind::Fcd => self.impl_.has_fcd_boundary_after(c),
        }
    }

    /// `spanQuickCheckYes(CharSequence s)`.
    pub fn span_quick_check_yes_infallible(&self, s: &[u16]) -> i32 {
        match self.kind {
            N2Kind::Decompose => self.impl_.decompose(s, 0, s.len() as i32, None),
            N2Kind::Compose { only_contiguous } => {
                ((self
                    .impl_
                    .compose_quick_check(s, 0, s.len() as i32, only_contiguous, true))
                    as u32
                    >> 1) as i32
            }
            N2Kind::Fcd => self.impl_.make_fcd(s, 0, s.len() as i32, None),
        }
    }
}

impl Normalizer2 for Normalizer2WithImpl {
    // normalize
    fn normalize(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()> {
        dest.clear();
        let mut buffer = ReorderingBuffer::new(&self.impl_, dest, src.len() as i32);
        self.normalize_buffer(src, &mut buffer);
        Ok(())
    }

    fn normalize_append(&self, src: &[u16], dest: &mut Vec<u16>) -> Result<()> {
        let mut buffer = ReorderingBuffer::new(&self.impl_, dest, src.len() as i32);
        self.normalize_buffer(src, &mut buffer);
        buffer.flush();
        Ok(())
    }

    // normalize and append
    fn normalize_second_and_append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()> {
        self.normalize_second_and_append_do(first, second, true);
        Ok(())
    }

    fn append(&self, first: &mut Vec<u16>, second: &[u16]) -> Result<()> {
        self.normalize_second_and_append_do(first, second, false);
        Ok(())
    }

    fn get_decomposition(&self, c: i32) -> Result<Option<Vec<u16>>> {
        Ok(self.impl_.get_decomposition(c))
    }

    fn get_raw_decomposition(&self, c: i32) -> Result<Option<Vec<u16>>> {
        Ok(self.impl_.get_raw_decomposition(c))
    }

    fn compose_pair(&self, a: i32, b: i32) -> Result<i32> {
        Ok(self.impl_.compose_pair(a, b))
    }

    fn get_combining_class(&self, c: i32) -> Result<i32> {
        Ok(Normalizer2WithImpl::get_combining_class(self, c))
    }

    // quick checks
    fn is_normalized(&self, s: &[u16]) -> Result<bool> {
        match self.kind {
            N2Kind::Compose { only_contiguous } => {
                // 5: small destCapacity for substring normalization
                let mut sb = Vec::new();
                let mut buffer = ReorderingBuffer::new(&self.impl_, &mut sb, 5);
                Ok(self
                    .impl_
                    .compose(s, 0, s.len() as i32, only_contiguous, false, &mut buffer))
            }
            _ => Ok(s.len() as i32 == self.span_quick_check_yes_infallible(s)),
        }
    }

    fn quick_check(&self, s: &[u16]) -> Result<QuickCheckResult> {
        match self.kind {
            N2Kind::Compose { only_contiguous } => {
                let span_length_and_maybe =
                    self.impl_
                        .compose_quick_check(s, 0, s.len() as i32, only_contiguous, false);
                if (span_length_and_maybe & 1) != 0 {
                    Ok(QuickCheckResult::Maybe)
                } else if ((span_length_and_maybe as u32) >> 1) as i32 == s.len() as i32 {
                    Ok(QuickCheckResult::Yes)
                } else {
                    Ok(QuickCheckResult::No)
                }
            }
            _ => Ok(if self.is_normalized(s)? {
                QuickCheckResult::Yes
            } else {
                QuickCheckResult::No
            }),
        }
    }

    fn span_quick_check_yes(&self, s: &[u16]) -> Result<i32> {
        Ok(self.span_quick_check_yes_infallible(s))
    }

    fn has_boundary_before(&self, c: i32) -> Result<bool> {
        Ok(Normalizer2WithImpl::has_boundary_before(self, c))
    }

    fn has_boundary_after(&self, c: i32) -> Result<bool> {
        Ok(Normalizer2WithImpl::has_boundary_after(self, c))
    }

    fn is_inert(&self, c: i32) -> Result<bool> {
        Ok(Normalizer2WithImpl::is_inert(self, c))
    }

    fn as_with_impl(&self) -> Option<&Normalizer2WithImpl> {
        Some(self)
    }
}

/// The normalizers of one set of normalization data in all modes.
///
/// Equivalent to `com.ibm.icu.impl.Norm2AllModes`. The built-in
/// singletons keep the exception of a failed load and return it on every
/// request, as Java's `Norm2AllModesSingleton` does.
#[derive(Debug)]
pub struct Norm2AllModes {
    /// `impl`.
    pub impl_: Arc<Normalizer2Impl>,
    /// `comp`.
    pub comp: Arc<Normalizer2WithImpl>,
    /// `decomp`.
    pub decomp: Arc<Normalizer2WithImpl>,
    /// `fcd`.
    pub fcd: Arc<Normalizer2WithImpl>,
    /// `fcc`.
    pub fcc: Arc<Normalizer2WithImpl>,
}

/// `Norm2AllModesSingleton(String name)`: the modes or the exception.
type Singleton = std::result::Result<Arc<Norm2AllModes>, Arc<LuceneError>>;

fn singleton(name: &str) -> Singleton {
    let mut impl_ = Normalizer2Impl::new();
    match impl_.load_name(&format!("{name}.nrm")) {
        Ok(_) => Ok(Arc::new(Norm2AllModes::new(Arc::new(impl_)))),
        Err(e) => Err(Arc::new(e)),
    }
}

static NFC_SINGLETON: LazyLock<Singleton> = LazyLock::new(|| singleton("nfc"));
static NFKC_SINGLETON: LazyLock<Singleton> = LazyLock::new(|| singleton("nfkc"));
static NFKC_CF_SINGLETON: LazyLock<Singleton> = LazyLock::new(|| singleton("nfkc_cf"));
static NFKC_SCF_SINGLETON: LazyLock<Singleton> = LazyLock::new(|| singleton("nfkc_scf"));

/// `cache`: the modes of other data, by name.
static CACHE: LazyLock<SoftCache<String, Arc<Norm2AllModes>, Option<ByteBuffer>>> =
    LazyLock::new(|| {
        SoftCache::new(|key: &String, bytes: Option<ByteBuffer>| {
            let mut impl_ = Normalizer2Impl::new();
            match bytes {
                None => {
                    impl_.load_name(&format!("{key}.nrm"))?;
                }
                Some(mut b) => {
                    impl_.load(&mut b)?;
                }
            }
            Ok(Some(Arc::new(Norm2AllModes::new(Arc::new(impl_)))))
        })
    });

/// `NOOP_NORMALIZER2`.
pub static NOOP_NORMALIZER2: LazyLock<Arc<NoopNormalizer2>> =
    LazyLock::new(|| Arc::new(NoopNormalizer2));

impl Norm2AllModes {
    fn new(ni: Arc<Normalizer2Impl>) -> Self {
        Self {
            comp: Arc::new(Normalizer2WithImpl::new(
                ni.clone(),
                N2Kind::Compose {
                    only_contiguous: false,
                },
            )),
            decomp: Arc::new(Normalizer2WithImpl::new(ni.clone(), N2Kind::Decompose)),
            fcd: Arc::new(Normalizer2WithImpl::new(ni.clone(), N2Kind::Fcd)),
            fcc: Arc::new(Normalizer2WithImpl::new(
                ni.clone(),
                N2Kind::Compose {
                    only_contiguous: true,
                },
            )),
            impl_: ni,
        }
    }

    fn get_instance_from_singleton(singleton: &Singleton) -> Result<Arc<Norm2AllModes>> {
        match singleton {
            Ok(m) => Ok(m.clone()),
            Err(e) => Err(e.duplicate()),
        }
    }

    /// `getNFCInstance()`.
    ///
    /// # Errors
    ///
    /// The exception of loading `nfc.nrm`.
    pub fn get_nfc_instance() -> Result<Arc<Norm2AllModes>> {
        Self::get_instance_from_singleton(&NFC_SINGLETON)
    }
    /// `getNFKCInstance()`.
    ///
    /// # Errors
    ///
    /// The exception of loading `nfkc.nrm`.
    pub fn get_nfkc_instance() -> Result<Arc<Norm2AllModes>> {
        Self::get_instance_from_singleton(&NFKC_SINGLETON)
    }
    /// `getNFKC_CFInstance()`.
    ///
    /// # Errors
    ///
    /// The exception of loading `nfkc_cf.nrm`.
    pub fn get_nfkc_cf_instance() -> Result<Arc<Norm2AllModes>> {
        Self::get_instance_from_singleton(&NFKC_CF_SINGLETON)
    }
    /// `getNFKC_SCFInstance()`.
    ///
    /// # Errors
    ///
    /// The exception of loading `nfkc_scf.nrm`.
    pub fn get_nfkc_scf_instance() -> Result<Arc<Norm2AllModes>> {
        Self::get_instance_from_singleton(&NFKC_SCF_SINGLETON)
    }

    // For use in properties APIs.
    /// `getN2WithImpl(int index)`: NFD, NFKD, NFC, NFKC for 0..3; `None`
    /// otherwise.
    ///
    /// # Errors
    ///
    /// The exception of loading the data.
    pub fn get_n2_with_impl(index: i32) -> Result<Arc<Normalizer2WithImpl>> {
        match index {
            0 => Ok(Self::get_nfc_instance()?.decomp.clone()), // NFD
            1 => Ok(Self::get_nfkc_instance()?.decomp.clone()), // NFKD
            2 => Ok(Self::get_nfc_instance()?.comp.clone()),   // NFC
            3 => Ok(Self::get_nfkc_instance()?.comp.clone()),  // NFKC
            _ => Err(LuceneError::null_pointer_with_message(format!(
                "getN2WithImpl({index}) is null"
            ))),
        }
    }

    /// `getInstance(ByteBuffer bytes, String name)`.
    ///
    /// # Errors
    ///
    /// The exception of loading the data; `MissingResourceException` for
    /// an unknown name without data.
    pub fn get_instance(bytes: Option<ByteBuffer>, name: &str) -> Result<Arc<Norm2AllModes>> {
        if bytes.is_none() {
            let singleton: Option<&Singleton> = match name {
                "nfc" => Some(&NFC_SINGLETON),
                "nfkc" => Some(&NFKC_SINGLETON),
                "nfkc_cf" => Some(&NFKC_CF_SINGLETON),
                _ => None,
            };
            if let Some(s) = singleton {
                return Self::get_instance_from_singleton(s);
            }
        }
        Ok(CACHE
            .get_instance(name.to_string(), bytes)?
            .expect("createInstance never returns null"))
    }

    /// `getFCDNormalizer2()`.
    ///
    /// # Errors
    ///
    /// The exception of loading `nfc.nrm`.
    pub fn get_fcd_normalizer2() -> Result<Arc<Normalizer2WithImpl>> {
        Ok(Self::get_nfc_instance()?.fcd.clone())
    }
}
