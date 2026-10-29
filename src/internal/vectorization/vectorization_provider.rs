//! Port of `org.apache.lucene.internal.vectorization.VectorizationProvider`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::panic::Location;
use std::sync::{Arc, LazyLock};

use crate::codecs::hnsw::FlatVectorsScorer;
use crate::error::{LuceneError, Result};
use crate::internal::vectorization::{
    DefaultDocValuesRangeSupport, DefaultVectorizationProvider, DocValuesBulkDecodeSupport,
    DocValuesRangeSupport, PostingDecodingUtil, VectorUtilSupport,
};
use crate::store::IndexInput;
use crate::util::jvm::HotspotVMOptions;

/// A provider of vectorization implementations.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.VectorizationProvider`.
///
/// Lucene picks an implementation at run time: an optimized one built on the
/// `jdk.incubator.vector` (Panama) API when the JVM makes it available, and the
/// scalar [`DefaultVectorizationProvider`] otherwise. Use [`get_instance`] to
/// obtain the singleton for the current runtime; see the [module docs](super)
/// for which implementations exist in this port.
///
/// # Divergences from Lucene 10.5.0
///
/// * **Trait, not an abstract class.** Java hides the constructor so that only
///   classes in the same package can extend `VectorizationProvider`; Rust has
///   no package visibility, so any crate could implement this trait. The
///   restriction is documentary here.
/// * **Free functions for the statics.** Java exposes
///   `VectorizationProvider.getInstance()` and the package-private
///   `lookup(boolean)` as static methods on the class. A `dyn`-compatible Rust
///   trait cannot carry them, so they are the module-level [`get_instance`] and
///   [`lookup`] functions.
/// * **`Send + Sync + Debug` bounds.** Not in the Java class; Rust needs them
///   because the singleton lives in a `static`.
pub trait VectorizationProvider: Send + Sync + Debug {
    /// Returns a singleton (stateless) [`VectorUtilSupport`] to support SIMD
    /// usage in [`crate::util::vector_util`].
    ///
    /// Equivalent to `VectorizationProvider.getVectorUtilSupport()`.
    fn get_vector_util_support(&self) -> &dyn VectorUtilSupport;

    /// Returns a [`FlatVectorsScorer`] that supports the Lucene99 format.
    ///
    /// Equivalent to `VectorizationProvider.getLucene99FlatVectorsScorer()`.
    fn get_lucene99_flat_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer>;

    /// Returns a [`FlatVectorsScorer`] that supports the Lucene99 scalar
    /// quantized format.
    ///
    /// Equivalent to
    /// `VectorizationProvider.getLucene99ScalarQuantizedVectorsScorer()`.
    fn get_lucene99_scalar_quantized_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer>;

    /// Creates a new [`PostingDecodingUtil`] for the given input.
    ///
    /// Equivalent to `VectorizationProvider.newPostingDecodingUtil(IndexInput)`.
    ///
    /// # Errors
    ///
    /// Returns any error raised while inspecting the input. Lucene declares
    /// `throws IOException` for the same reason: a provider may need to map the
    /// input before it can decide which decoder to hand back.
    fn new_posting_decoding_util(&self, input: Box<dyn IndexInput>) -> Result<PostingDecodingUtil>;

    /// Returns a [`DocValuesRangeSupport`] for SIMD-accelerated range
    /// evaluation.
    ///
    /// Equivalent to `VectorizationProvider.getDocValuesRangeSupport()`,
    /// including the default implementation that returns the scalar singleton.
    fn get_doc_values_range_support(&self) -> &'static dyn DocValuesRangeSupport {
        static DEFAULT: DefaultDocValuesRangeSupport = DefaultDocValuesRangeSupport::INSTANCE;
        &DEFAULT
    }

    /// Returns a [`DocValuesBulkDecodeSupport`] instance for bulk numeric value
    /// decode.
    ///
    /// Equivalent to `VectorizationProvider.getDocValuesBulkDecodeSupport()`.
    fn get_doc_values_bulk_decode_support(&self) -> &'static dyn DocValuesBulkDecodeSupport;
}

/// The classes Lucene allows to call `VectorizationProvider.getInstance()`.
///
/// Equivalent to the private `VectorizationProvider.VALID_CALLERS` set. Lucene
/// enforces it in `ensureCaller()` with a `StackWalker`, throwing
/// `IllegalCallerException` for anybody else.
///
/// # Divergence from Lucene 10.5.0
///
/// Rust has no stack walker that can report the *caller's* type, so the list
/// itself is documentation; [`ensure_caller`] enforces the rule against the
/// parallel list of Rucene source files, [`VALID_CALLER_FILES`].
pub const VALID_CALLERS: &[&str] = &[
    "org.apache.lucene.codecs.hnsw.FlatVectorScorerUtil",
    "org.apache.lucene.util.VectorUtil",
    "org.apache.lucene.codecs.lucene104.Lucene104PostingsReader",
    "org.apache.lucene.codecs.lucene104.PostingIndexInput",
    "org.apache.lucene.codecs.lucene90.Lucene90DocValuesProducer",
    "org.apache.lucene.tests.util.TestSysoutsLimits",
];

/// The Rucene source files whose call sites stand in for [`VALID_CALLERS`].
///
/// Equivalent to the same set, expressed the way [`ensure_caller`] can check
/// it. Java compares the calling class's fully qualified name; Rust exposes no
/// runtime type for a stack frame, so the port compares the caller's *source
/// file*, which `#[track_caller]` does report. Each entry is the Rucene file
/// holding the port of the Lucene class on the same line of [`VALID_CALLERS`].
///
/// `org.apache.lucene.tests.util.TestSysoutsLimits`, the last of Lucene's valid
/// callers, is a class of the test framework with no counterpart in this crate;
/// [`TEST_DIR`] stands in for it, exactly as
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets) substitutes a
/// directory for Lucene's test-framework package.
pub const VALID_CALLER_FILES: &[&str] = &[
    "src/codecs/hnsw/flat_vectors.rs",
    "src/util/vector_util.rs",
    "src/codecs/lucene104/postings_reader.rs",
    "src/codecs/lucene104/posting_index_input.rs",
    "src/codecs/lucene90/doc_values.rs",
];

/// The directory that stands in for Lucene's test framework in
/// [`VALID_CALLER_FILES`].
const TEST_DIR: &str = "tests/";

/// The system property that raises the Java feature version this class will
/// provide optimized implementations for.
///
/// Equivalent to the private
/// `VectorizationProvider.UPPER_JAVA_FEATURE_VERSION_SYSPROP`.
pub const UPPER_JAVA_FEATURE_VERSION_SYSPROP: &str =
    "org.apache.lucene.vectorization.upperJavaFeatureVersion";

/// The highest Java feature version Lucene 10.5.0 ships support for.
///
/// Equivalent to the private
/// `VectorizationProvider.DEFAULT_UPPER_JAVA_FEATURE_VERSION`.
pub const DEFAULT_UPPER_JAVA_FEATURE_VERSION: i32 = 26;

/// The warning Lucene logs when the runtime is not a HotSpot VM.
const NOT_HOTSPOT_VM_WARNING: &str =
    "Java runtime is not using Hotspot VM; Java vector incubator API can't be enabled.";

/// The warning Lucene logs when the vector incubator module is not readable.
const VECTOR_MODULE_NOT_READABLE_WARNING: &str = "Java vector incubator module is not readable. For optimal vector performance, pass '--add-modules jdk.incubator.vector' to enable Vector API.";

/// Reads the Java feature version up to which optimized implementations are
/// offered.
///
/// Equivalent to the private static
/// `org.apache.lucene.internal.vectorization.VectorizationProvider#getUpperJavaFeatureVersion`,
/// which backs the `UPPER_JAVA_FEATURE_VERSION` field. Java reads the property
/// with `System.getProperty` and raises the default with `Math.max`, keeping
/// the default and logging a warning when the value cannot be read or parsed;
/// this port reads the process environment, which is where this crate resolves
/// every system property, and reproduces the same three outcomes.
pub fn get_upper_java_feature_version() -> i32 {
    let mut runtime_version = DEFAULT_UPPER_JAVA_FEATURE_VERSION;
    match std::env::var(UPPER_JAVA_FEATURE_VERSION_SYSPROP) {
        Ok(value) => match value.parse::<i32>() {
            Ok(parsed) => runtime_version = parsed.max(runtime_version),
            Err(_) => log::warn!(
                "Cannot read sysprop {UPPER_JAVA_FEATURE_VERSION_SYSPROP}, so the default value will be used."
            ),
        },
        Err(std::env::VarError::NotPresent) => {}
        Err(_) => log::warn!(
            "Cannot read sysprop {UPPER_JAVA_FEATURE_VERSION_SYSPROP}, so the default value will be used."
        ),
    }
    runtime_version
}

/// Looks up the incubating vector module, returning its name when the runtime
/// makes it readable.
///
/// Equivalent to the private static
/// `org.apache.lucene.internal.vectorization.VectorizationProvider#lookupVectorModule`,
/// which asks Lucene's own `ModuleLayer` — or the boot layer, when Lucene is
/// unnamed — for `jdk.incubator.vector`, so that [`lookup`] can refuse the
/// Panama provider when the JVM was built or launched without it.
///
/// # Divergence from Lucene 10.5.0
///
/// A Rust binary has no module layer and no incubating module to find, so the
/// answer is always `None` — the same permanently negative capability probe as
/// [`HotspotVMOptions::is_hotspot_vm`](crate::util::jvm::HotspotVMOptions::is_hotspot_vm),
/// and it leads [`lookup`] down the same branch Lucene takes on a JVM launched
/// without `--add-modules jdk.incubator.vector`.
pub fn lookup_vector_module() -> Option<&'static str> {
    None
}

/// Normalises the path [`Location::caller`] reports so that a host which spells
/// it with backslashes compares equal to the forward-slash paths recorded in
/// [`VALID_CALLER_FILES`].
fn caller_file(location: &Location<'static>) -> String {
    location.file().replace('\\', "/")
}

/// Checks that the singleton getter was called from a known Lucene class.
///
/// Equivalent to the private static
/// `org.apache.lucene.internal.vectorization.VectorizationProvider#ensureCaller`.
/// Java runs a `StackWalker` over `s.skip(2).limit(1)` — that is, the caller of
/// `getInstance()` — and matches its class name against [`VALID_CALLERS`]. Rust
/// exposes no runtime type for a stack frame, so this port uses the one caller
/// identity stable Rust does provide: `#[track_caller]` on both this function
/// and [`get_instance`] makes [`Location::caller`] resolve to the call site of
/// `get_instance`, the very frame Java's `skip(2).limit(1)` selects, and the
/// comparison is then made against [`VALID_CALLER_FILES`] rather than against
/// class names.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] — this crate's stand-in for Java's
/// `IllegalCallerException` — with Lucene's message, when the caller is not one
/// of the known Lucene classes.
#[track_caller]
pub fn ensure_caller() -> Result<()> {
    let file = caller_file(Location::caller());
    let valid_caller = VALID_CALLER_FILES.contains(&file.as_str()) || file.starts_with(TEST_DIR);
    if !valid_caller {
        return Err(LuceneError::illegal_state(
            "VectorizationProvider is internal and can only be used by known Lucene classes."
                .to_string(),
        ));
    }
    Ok(())
}

/// Holds the singleton, initialized on first use.
///
/// Equivalent to the private `VectorizationProvider.Holder` class, which exists
/// in Java to prevent a classloading deadlock. [`LazyLock`] gives the same
/// initialize-once-on-first-access behaviour.
static HOLDER: LazyLock<Arc<dyn VectorizationProvider>> = LazyLock::new(|| lookup(false));

/// Returns the default instance of the provider matching the vectorization
/// possibilities of the actual runtime.
///
/// Equivalent to `VectorizationProvider.getInstance()`.
///
/// # Panics
///
/// Panics when the caller is not one of the known Lucene classes, which is
/// what [`ensure_caller`] decides. Java throws `IllegalCallerException`, an
/// unchecked exception, from a method whose declared result is the provider
/// itself; a panic is that contract's Rust counterpart, since returning
/// [`Result`] here would change the signature Lucene specifies.
#[track_caller]
pub fn get_instance() -> Arc<dyn VectorizationProvider> {
    if let Err(error) = ensure_caller() {
        panic!("{error}");
    }
    Arc::clone(&HOLDER)
}

/// Selects the provider that matches the current runtime.
///
/// Equivalent to the package-private `VectorizationProvider.lookup(boolean)`,
/// which Lucene exposes for its own tests.
///
/// Lucene's first gate is `Constants.IS_HOTSPOT_VM`: the Java vector incubator
/// module only works on HotSpot, and every other case falls back to the scalar
/// provider with a logged warning. `HotspotVMOptions::is_hotspot_vm()`
/// reproduces Lucene's own non-HotSpot branch and is permanently `false` in a
/// Rust binary, so this port always takes that branch and returns
/// [`DefaultVectorizationProvider`]. Lucene's third gate, the readability of
/// `jdk.incubator.vector`, is reproduced below through
/// [`lookup_vector_module`] and would reject the Panama provider on its own;
/// the gates between and after them — the JVMCI check, the
/// `tests.vectorsize` / `tests.forceintegervectors` system properties, and the
/// client-VM check — have no counterpart here (Rust has no JVMCI compiler and
/// no client VM) and are described in the [module docs](super).
///
/// `test_mode` is therefore unused: in Lucene it only relaxes two of those
/// unreachable gates.
pub fn lookup(test_mode: bool) -> Arc<dyn VectorizationProvider> {
    let _ = test_mode;
    if !HotspotVMOptions::is_hotspot_vm() {
        log::warn!("{NOT_HOTSPOT_VM_WARNING}");
    } else if lookup_vector_module().is_none() {
        log::warn!("{VECTOR_MODULE_NOT_READABLE_WARNING}");
    }
    Arc::new(DefaultVectorizationProvider::new())
}
