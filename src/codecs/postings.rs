//! Postings SPI base ported from `org.apache.lucene.codecs`.
//!
//! This module contains the abstract factories and pull/push base classes that
//! concrete postings formats plug into the codec framework. It mirrors the
//! lifecycle of the Java originals (init, write/flush, merge, close) and the
//! field/term/state abstractions used by the default `Lucene104` codec.
//!
//! The key traits are:
//!
//! * [`PostingsFormat`] — named factory that creates per-segment readers and
//!   writers.
//! * [`FieldsConsumer`] / [`FieldsProducer`] — segment-level pull API for
//!   writing and reading inverted fields.
//! * [`PostingsReaderBase`] / [`PostingsWriterBase`] — low-level pull API used
//!   by term dictionaries to decode/encode postings metadata.
//! * [`PushPostingsWriterBase`] — push API that concrete postings writers may
//!   implement; a blanket [`PostingsWriterBase`] implementation turns the push
//!   callbacks into the pull lifecycle.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

use crate::util::spi::{NamedSPI, NamedSPILoader};

use crate::codecs::norms::NormsProducer;
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::{IndexOptions, NumericDocValues};
use crate::search::NO_MORE_DOCS;
use crate::store::{DataInput, DataOutput, IndexInput, IndexOutput};
use crate::util::{BytesRef, FixedBitSet};

// -----------------------------------------------------------------------------
// Postings flags (re-exported from the index module)
// -----------------------------------------------------------------------------

pub use crate::index::{
    feature_requested as postings_feature_requested, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};

// -----------------------------------------------------------------------------
// State abstractions
// -----------------------------------------------------------------------------

pub use crate::codecs::term_state::{
    BlockTermState, CompetitiveImpactAccumulator, Impact, TermStats,
};

/// State passed to [`FieldsConsumer::merge`] describing the source segments.
///
pub use crate::index::merge::MergeState;

// -----------------------------------------------------------------------------
// Field / term / postings iteration (re-exported from the index module)
// -----------------------------------------------------------------------------

pub use crate::index::{
    EmptyTerms, EmptyTermsEnum, Fields, ImpactsEnum, PostingsEnum, SeekStatus, TermState, Terms,
    TermsEnum,
};

/// Returns an empty terms iterator.
pub fn empty_terms_enum() -> Box<dyn TermsEnum> {
    Box::new(EmptyTermsEnum::new())
}

/// Terms iterator that always reports a single fixed term and delegates
/// postings to an underlying iterator.
///
/// Equivalent to `org.apache.lucene.index.SingleTermsEnum`.
pub struct SingleTermsEnum {
    inner: Box<dyn TermsEnum>,
    term: BytesRef,
    atts: crate::util::attribute::AttributeSource,
}

impl SingleTermsEnum {
    /// Creates a single-term enum wrapping `inner` and reporting `term`.
    pub fn new(inner: Box<dyn TermsEnum>, term: BytesRef) -> Self {
        Self {
            inner,
            term,
            atts: crate::util::attribute::AttributeSource::new(),
        }
    }
}

impl std::fmt::Debug for SingleTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleTermsEnum")
            .field("term", &self.term)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for SingleTermsEnum {
    /// `org.apache.lucene.index.SingleTermsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SingleTermsEnum",
            self,
        ))
    }
}

impl TermsEnum for SingleTermsEnum {
    fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
        &mut self.atts
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.term.clone()))
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.inner.postings(reuse, flags)
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Ok(false)
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Ok(SeekStatus::END)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Ok(())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Ok(())
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::unsupported_operation(
            "ord not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Err(LuceneError::unsupported_operation(
            "doc_freq not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Err(LuceneError::unsupported_operation(
            "total_term_freq not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
        Err(LuceneError::unsupported_operation(
            "impacts not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(LuceneError::unsupported_operation(
            "term_state not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        Ok(None)
    }
}

// -----------------------------------------------------------------------------
// Segment-level consumer / producer
// -----------------------------------------------------------------------------

/// Writes all fields, terms and postings for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.FieldsConsumer`.
///
/// `norms` is the segment's [`NormsProducer`], or `None` — Java's `null` — when
/// no field has norms; the postings writer reads a field's norms from it,
/// term by term, to compute the impacts it writes with its skip data.
pub trait FieldsConsumer {
    /// Writes every field/term/posting in `fields`.
    ///
    /// Equivalent to `FieldsConsumer.write(Fields, NormsProducer)`.
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()>;

    /// Merges in the fields from the readers in `merge_state`.
    ///
    /// Equivalent to `FieldsConsumer.merge(MergeState, NormsProducer)`
    /// (`FieldsConsumer.java:72-96`): every non-null postings producer is
    /// integrity-checked and given a slice of the composite doc-ID space —
    /// starting at the sum of the `maxDoc`s before it, null producers
    /// included — and the merged view of all of them, mapped around deleted
    /// documents ([`MappedMultiFields`](crate::index::mapped_multi_fields::MappedMultiFields)
    /// over [`MultiFields`](crate::index::multi_fields::MultiFields)), is
    /// handed to [`write`](Self::write). A format that can do better — bulk-
    /// copying bytes, say — overrides this.
    ///
    /// `MappedMultiFields` shares the merge state with every terms and
    /// postings enum it hands out, as Java's reference does; it receives a
    /// [`Clone`] of `merge_state`, which is that same state: every doc map,
    /// reader and live-docs set in it is the original's own object.
    ///
    /// # Errors
    ///
    /// Propagates whatever the integrity checks, the merged view or
    /// [`write`](Self::write) raise.
    fn merge(&mut self, merge_state: &MergeState, norms: Option<&dyn NormsProducer>) -> Result<()> {
        default_merge(self, merge_state, norms)
    }

    /// Closes this consumer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

/// The body of the default `FieldsConsumer.merge(MergeState, NormsProducer)`
/// (`FieldsConsumer.java:72-96`), which [`FieldsConsumer::merge`] runs.
///
/// A consumer that overrides `merge` and falls back to the default, as
/// `STUniformSplitTermsWriter.merge` does with `super.merge(mergeState,
/// normsProducer)`, calls this: Rust cannot call a trait's default body from
/// an implementation that overrides it.
///
/// # Errors
///
/// Propagates whatever the integrity checks, the merged view or
/// [`FieldsConsumer::write`] raise.
pub fn default_merge<C: FieldsConsumer + ?Sized>(
    consumer: &mut C,
    merge_state: &MergeState,
    norms: Option<&dyn NormsProducer>,
) -> Result<()> {
    let mut fields: Vec<Box<dyn Fields>> = Vec::new();
    let mut slices: Vec<crate::index::multi_reader::ReaderSlice> = Vec::new();

    let mut doc_base = 0i32;

    for (reader_index, producer) in merge_state.fields_producers.iter().enumerate() {
        let max_doc = merge_state.max_docs[reader_index];
        if let Some(producer) = producer {
            merge_state.check_aborted()?;
            producer.check_integrity()?;
            slices.push(crate::index::multi_reader::ReaderSlice::new(
                doc_base,
                max_doc,
                reader_index as i32,
            ));
            fields.push(Box::new(SharedFieldsProducer(Arc::clone(producer))));
        }
        doc_base += max_doc;
    }

    let merged_fields = crate::index::mapped_multi_fields::MappedMultiFields::new(
        Arc::new(merge_state.clone()),
        crate::index::multi_fields::MultiFields::new(fields, slices),
    );
    consumer.write(&merged_fields, norms)
}

/// A postings producer of a merge state, seen as the [`Fields`] it is.
///
/// Java's `FieldsConsumer.merge` adds each `FieldsProducer` itself to the list
/// it hands `MultiFields` (`FieldsConsumer.java:86`). `MultiFields` here owns
/// boxed subs while the merge state shares its producers behind an [`Arc`], so
/// this adapter forwards to the shared producer and is otherwise that same
/// object.
struct SharedFieldsProducer(Arc<dyn FieldsProducer>);

impl Fields for SharedFieldsProducer {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.0.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.0.terms(field)
    }

    fn size(&self) -> i32 {
        self.0.size()
    }
}

/// Reads all fields, terms and postings for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.FieldsProducer`.
pub trait FieldsProducer: Fields + Send + Sync {
    /// Verifies the integrity of the underlying data files.
    fn check_integrity(&self) -> Result<()>;

    /// Returns an instance optimized for merging.
    ///
    /// The default implementation returns a clone of `self`. Implementations
    /// may override this to return a lightweight merge-only view.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>>;

    /// Closes this producer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Postings format factory
// -----------------------------------------------------------------------------

/// Encodes and decodes postings (inverted-index term-document lists).
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.PostingsFormat`.
pub trait PostingsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Creates a writer for a new segment.
    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>>;

    /// Creates a reader for an existing segment.
    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>>;

    /// This format as [`Any`](std::any::Any), for the `instanceof` tests Java
    /// code makes on a `PostingsFormat` (`TestUtil.getPostingsFormat(Codec, String)` narrows
    /// to `PerFieldPostingsFormat`, `TestUtil.java:1396-1403`).
    ///
    /// **This has no counterpart in Lucene 10.5.0.** Java tests the run-time
    /// class of any object; a Rust trait object exposes its concrete type only
    /// through `Any`, which an implementation opts into by returning
    /// `Some(self)`. The default, `None`, answers `false` to every such test.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// `PostingsFormat.toString()`: `"PostingsFormat(name=" + name + ")"`,
    /// which a format overrides as Java's subclass overrides `toString()`
    /// (`FSTPostingsFormat`, `BlockTreeOrdsPostingsFormat`, the bloom
    /// formats).
    fn to_java_string(&self) -> String {
        format!("PostingsFormat(name={})", self.name())
    }
}

impl fmt::Display for dyn PostingsFormat + '_ {
    /// `toString()` of the format behind the trait object:
    /// [`PostingsFormat::to_java_string`], Java's virtual call, so an override
    /// is seen through `dyn PostingsFormat` too.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

/// A registry mapping postings-format short names to [`PostingsFormat`]
/// implementations.
///
/// Rust has no run-time service discovery, so formats
/// are registered explicitly with [`PostingsFormatRegistry::register`] and
/// looked up by name with [`PostingsFormatRegistry::for_name`].
#[derive(Debug, Clone)]
pub struct PostingsFormatRegistry {
    /// The providers, kept by a [`NamedSPILoader`] exactly as Java's
    /// `PostingsFormat.Holder.LOADER` keeps them: first registration for a
    /// name wins, lookups fail with the loader's message, and iteration is in
    /// insertion order.
    formats: Arc<RwLock<NamedSPILoader<dyn PostingsFormat>>>,
}

impl Default for PostingsFormatRegistry {
    fn default() -> Self {
        Self {
            formats: Arc::new(RwLock::new(NamedSPILoader::empty(
                "org.apache.lucene.codecs.PostingsFormat",
            ))),
        }
    }
}

impl NamedSPI for dyn PostingsFormat {
    /// Equivalent to `NamedSPILoader.NamedSPI.getName()`, which every
    /// `PostingsFormat` implements by returning the name it was constructed with.
    fn get_name(&self) -> &str {
        self.name()
    }
}

impl PostingsFormatRegistry {
    /// Looks up a postings format by name.
    ///
    /// Returns `None` if no format has been registered under the given name.
    pub fn for_name(&self, name: &str) -> Option<Arc<dyn PostingsFormat>> {
        self.formats
            .read()
            .map_err(|_| {
                LuceneError::illegal_state("postings format registry lock was poisoned".to_string())
            })
            .ok()?
            .lookup(name)
            .ok()
    }

    /// Looks up a postings format by name, failing as Lucene does.
    ///
    /// Equivalent to `PostingsFormat.forName(String)`, which is
    /// `NamedSPILoader.lookup(String)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) with the loader's message,
    /// listing the available names, when no format is registered under
    /// `name`, and [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when the registry lock is poisoned.
    pub fn lookup(&self, name: &str) -> Result<Arc<dyn PostingsFormat>> {
        self.formats
            .read()
            .map_err(|_| {
                LuceneError::illegal_state("postings format registry lock was poisoned".to_string())
            })?
            .lookup(name)
    }

    /// Returns the names of all registered postings formats, in registration order.
    ///
    /// Equivalent to `NamedSPILoader.availableServices()`, the key set of
    /// Java's insertion-ordered `LinkedHashMap`.
    pub fn available_postings_formats(&self) -> Vec<String> {
        let Ok(formats) = self.formats.read() else {
            return Vec::new();
        };
        formats
            .available_services()
            .into_iter()
            .map(str::to_string)
            .collect()
    }
}

/// Reloads the global registry from `classloader`, adding the
/// formats it has not seen.
///
/// Equivalent to `PostingsFormat.reloadPostingsFormats(ClassLoader)`, which is
/// `Holder.getLoader().reload(classloader)`: see
/// [`NamedSPILoader::reload`](crate::util::spi::NamedSPILoader::reload). Only
/// new names are added; an existing one is never removed or replaced.
///
/// # Errors
///
/// Returns whatever [`NamedSPILoader::reload`](crate::util::spi::NamedSPILoader::reload)
/// returns, and [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when the registry lock is
/// poisoned.
pub fn reload_postings_formats(
    classloader: &Arc<crate::util::resource_loader::ClassLoader>,
) -> Result<()> {
    GLOBAL_POSTINGS_REGISTRY
        .formats
        .write()
        .map_err(|_| {
            LuceneError::illegal_state("postings format registry lock was poisoned".to_string())
        })?
        .reload(classloader)
}

/// The global registry.
///
/// Lucene discovers postings formats through the Java service loader:
/// `PostingsFormat.forName` consults a `NamedSPILoader` that reads
/// `META-INF/services/org.apache.lucene.codecs.PostingsFormat`, and the
/// `lucene-core` jar lists exactly one entry there,
/// `org.apache.lucene.codecs.lucene104.Lucene104PostingsFormat`. Nothing in
/// Lucene registers it explicitly, so a caller that merely has `lucene-core` on
/// the classpath can already resolve the name `Lucene104` — which is what
/// `PerFieldPostingsFormat.FieldsReader` does when it reads a segment back and
/// looks up the format named by each field's
/// `PerFieldPostingsFormat.format` attribute.
///
/// Equivalent to `PostingsFormat.Holder.LOADER`,
/// `new NamedSPILoader<>(PostingsFormat.class)`: it scans `lucene-core`'s
/// loader ([`lucene_core_class_loader`](crate::util::spi::lucene_core_class_loader)),
/// which carries that service file. [`reload_postings_formats`] is
/// `reloadPostingsFormats(ClassLoader)`.
static GLOBAL_POSTINGS_REGISTRY: LazyLock<PostingsFormatRegistry> =
    LazyLock::new(|| PostingsFormatRegistry {
        formats: Arc::new(RwLock::new(
            NamedSPILoader::new(
                &crate::util::spi::ServiceClass::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    Arc::clone(crate::util::spi::lucene_core_class_loader()),
                ),
                None,
            )
            .expect("INVARIANT: lucene-core's postings format names are legal service names"),
        )),
    });

/// Looks up a postings format by name from the global registry.
///
/// Returns `None` if no format has been registered under the given name.
pub fn postings_for_name(name: &str) -> Option<Arc<dyn PostingsFormat>> {
    GLOBAL_POSTINGS_REGISTRY.for_name(name)
}

/// Looks up a postings format by name from the global registry, failing as Lucene
/// does.
///
/// Equivalent to `PostingsFormat.forName(String)`.
///
/// # Errors
///
/// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) with the loader's message when no
/// format is registered under `name`.
pub fn postings_lookup(name: &str) -> Result<Arc<dyn PostingsFormat>> {
    GLOBAL_POSTINGS_REGISTRY.lookup(name)
}

/// Returns the names of all postings formats registered in the global
/// registry.
pub fn available_postings_formats() -> Vec<String> {
    GLOBAL_POSTINGS_REGISTRY.available_postings_formats()
}

// -----------------------------------------------------------------------------
// Pull postings reader / writer base
// -----------------------------------------------------------------------------

/// Low-level postings reader used by term dictionaries.
///
/// Equivalent to `org.apache.lucene.codecs.PostingsReaderBase`.
/// Java's `Lucene103BlockTreeTermsReader.toString()` concatenates the postings
/// reader it delegates to, so [`fmt::Display`] is a supertrait here.
pub trait PostingsReaderBase: Send + Sync + fmt::Display {
    /// Performs initialization, such as reading and verifying the header from
    /// the terms dictionary input.
    fn init(&mut self, terms_in: &mut dyn IndexInput, state: &SegmentReadState) -> Result<()>;

    /// Returns a newly created empty term state.
    fn new_term_state(&self) -> Result<BlockTermState>;

    /// Decodes metadata for the next term.
    ///
    /// Takes `&self`: Java's `decodeTerm` writes into the `state` it is handed
    /// and never touches the reader, and the same is true of `postings` and
    /// `impacts` below, which only read the reader's inputs to clone them. The
    /// `&mut` this port used instead was not Lucene's shape, and it was what
    /// made the reader unshareable — `SegmentTermsEnum` could not hold it while
    /// `Lucene103BlockTreeTermsReader` owned it behind `&mut self`, so the
    /// terms dictionary had no way to decode a term's metadata and every term
    /// query over an on-disk segment silently found nothing.
    fn decode_term(
        &self,
        input: &mut dyn DataInput,
        field_info: &FieldInfo,
        state: &mut BlockTermState,
        absolute: bool,
    ) -> Result<()>;

    /// Returns a postings iterator for the given term state.
    fn postings(
        &self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>>;

    /// Returns an impacts iterator for the given term state.
    fn impacts(
        &self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        flags: i32,
    ) -> Result<Box<dyn ImpactsEnum>>;

    /// Verifies the integrity of the underlying postings data.
    fn check_integrity(&self) -> Result<()>;

    /// Closes this reader.
    fn close(&mut self) -> Result<()>;
}

/// Low-level postings writer used by term dictionaries.
///
/// Equivalent to `org.apache.lucene.codecs.PostingsWriterBase`.
pub trait PostingsWriterBase {
    /// Performs one-time initialization, typically writing a header.
    fn init(&mut self, terms_out: &mut dyn IndexOutput, state: &SegmentWriteState) -> Result<()>;

    /// Writes all postings for the current term.
    ///
    /// The provided `terms_enum` is already positioned on the term to write.
    /// This method must set a bit in `docs_seen` for every document written.
    /// If no documents contain the term, it returns `Ok(None)` and the terms
    /// dictionary will skip the term.
    fn write_term(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: Option<&dyn NormsProducer>,
    ) -> Result<Option<BlockTermState>>;

    /// Encodes term metadata to the provided output.
    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()>;

    /// Sets the current field for writing.
    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()>;

    /// Closes this writer.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Push postings writer base
// -----------------------------------------------------------------------------

/// Mutable state shared by the default push writer implementation.
///
/// Concrete push writers embed this struct and expose it through
/// [`PushPostingsWriterBase::push_state`] /
/// [`PushPostingsWriterBase::push_state_mut`].
///
/// Equivalent to the fields of `PushPostingsWriterBase`. The `protected` ones
/// are public here; `postingsEnum` and `enumFlags`, `private` in Java
/// (`PushPostingsWriterBase.java:42-43`), are private:
///
/// ```compile_fail,E0616
/// let state = rucene::codecs::postings::PushPostingsWriterState::new();
/// let _flags = state.enum_flags;
/// ```
#[derive(Default)]
pub struct PushPostingsWriterState {
    /// The postings enum the last [`write_term`](PushPostingsWriterBase::write_term)
    /// pulled the postings from, offered to the next one as `reuse`.
    ///
    /// Equivalent to `PushPostingsWriterBase.postingsEnum` ("Reused in
    /// writeTerm", `PushPostingsWriterBase.java:41-42`), which no method ever
    /// resets: it is kept across terms and fields alike.
    postings_enum: Option<Box<dyn PostingsEnum>>,
    /// The field currently being written.
    pub field_info: Option<FieldInfo>,
    /// Index options of the current field.
    pub index_options: IndexOptions,
    /// Whether frequencies are written for the current field.
    pub write_freqs: bool,
    /// Whether positions are written for the current field.
    pub write_positions: bool,
    /// Whether payloads are written for the current field.
    pub write_payloads: bool,
    /// Whether offsets are written for the current field.
    pub write_offsets: bool,
    /// Flags used to request the correct postings enum for the current field.
    ///
    /// Equivalent to the private `PushPostingsWriterBase.enumFlags`
    /// (`PushPostingsWriterBase.java:43`): only
    /// [`set_field`](Self::set_field) writes it and only
    /// [`write_term`](PushPostingsWriterBase::write_term) reads it.
    enum_flags: i32,
}

impl PushPostingsWriterState {
    /// Creates an empty push writer state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the current field and derives the per-field flags from it.
    ///
    /// Equivalent to the body of `PushPostingsWriterBase.setField(FieldInfo)`
    /// (`PushPostingsWriterBase.java:84-112`): it stores the field, derives
    /// `write_freqs`, `write_positions`, `write_offsets` and `write_payloads`,
    /// and the flags `write_term` requests the postings enum with. A Rust
    /// trait default cannot be called from the override that replaces it, so
    /// this is the method an override calls where Java's calls
    /// `super.setField(fieldInfo)`, as
    /// `Lucene104PostingsWriter.setField` does (`Lucene104PostingsWriter.java:233-237`).
    pub fn set_field(&mut self, field_info: &FieldInfo) {
        self.field_info = Some(field_info.clone());
        self.index_options = field_info.index_options;
        self.write_freqs = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS);
        self.write_positions = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        self.write_offsets = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        self.write_payloads = field_info.has_payloads();

        if !self.write_freqs {
            self.enum_flags = POSTINGS_ENUM_NONE;
        } else if !self.write_positions {
            self.enum_flags = POSTINGS_ENUM_FREQS;
        } else if !self.write_offsets {
            if self.write_payloads {
                self.enum_flags = POSTINGS_ENUM_PAYLOADS;
            } else {
                self.enum_flags = POSTINGS_ENUM_POSITIONS;
            }
        } else if self.write_payloads {
            self.enum_flags = POSTINGS_ENUM_PAYLOADS | POSTINGS_ENUM_OFFSETS;
        } else {
            self.enum_flags = POSTINGS_ENUM_OFFSETS;
        }
    }
}

impl fmt::Debug for PushPostingsWriterState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushPostingsWriterState")
            .field(
                "postings_enum",
                &self
                    .postings_enum
                    .as_ref()
                    .map(|postings_enum| postings_enum.to_string()),
            )
            .field("field_info", &self.field_info)
            .field("index_options", &self.index_options)
            .field("write_freqs", &self.write_freqs)
            .field("write_positions", &self.write_positions)
            .field("write_payloads", &self.write_payloads)
            .field("write_offsets", &self.write_offsets)
            .field("enum_flags", &self.enum_flags)
            .finish()
    }
}

/// Push-style postings writer.
///
/// Equivalent to `org.apache.lucene.codecs.PushPostingsWriterBase`. Types that
/// implement this trait automatically implement [`PostingsWriterBase`] via the
/// blanket implementation below; they only need to supply the lifecycle,
/// metadata encoding and push callbacks.
///
/// Like Java's, a writer keeps the norms iterator of the term it is writing
/// (`startTerm(NumericDocValues)`); that iterator is not thread-safe, so this
/// trait, [`PostingsWriterBase`] and [`FieldsConsumer`] are not `Send` or
/// `Sync`. A postings writer lives on the thread that flushes or merges.
pub trait PushPostingsWriterBase {
    /// Returns the shared mutable state.
    fn push_state(&self) -> &PushPostingsWriterState;

    /// Returns the shared mutable state.
    fn push_state_mut(&mut self) -> &mut PushPostingsWriterState;

    /// Performs one-time initialization, typically writing a header.
    fn init(&mut self, terms_out: &mut dyn IndexOutput, state: &SegmentWriteState) -> Result<()>;

    /// Encodes term metadata to the provided output.
    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()>;

    /// Closes this writer.
    fn close(&mut self) -> Result<()>;

    /// Returns a newly created empty term state.
    fn new_term_state(&self) -> Result<BlockTermState>;

    /// Starts a new term; `norms` are the field's norms, or `None` when the
    /// field has none.
    ///
    /// Equivalent to `PushPostingsWriterBase.startTerm(NumericDocValues)`,
    /// whose implementations keep the reference for the term's documents
    /// (`Lucene104PostingsWriter.java:240-257`); the writer receives them
    /// here to keep until the next term.
    fn start_term(&mut self, norms: Option<Box<dyn NumericDocValues>>) -> Result<()>;

    /// Finishes the current term.
    fn finish_term(&mut self, state: &mut BlockTermState) -> Result<()>;

    /// Starts a new document within the current term.
    fn start_doc(&mut self, doc_id: i32, freq: i32) -> Result<()>;

    /// Adds a position, payload and offsets for the current document.
    fn add_position(
        &mut self,
        position: i32,
        payload: Option<&[u8]>,
        start_offset: i32,
        end_offset: i32,
    ) -> Result<()>;

    /// Finishes the current document.
    fn finish_doc(&mut self) -> Result<()>;

    /// Sets the current field and computes per-field flags.
    ///
    /// Equivalent to `PushPostingsWriterBase.setField(FieldInfo)`: the default
    /// implementation is [`PushPostingsWriterState::set_field`], which an
    /// override calls where Java's calls `super.setField`.
    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()> {
        self.push_state_mut().set_field(field_info);
        Ok(())
    }

    /// Pulls postings from `terms_enum` and invokes the push callbacks.
    ///
    /// The default implementation mirrors `PushPostingsWriterBase.writeTerm` in
    /// Java: it calls `start_term`, iterates over the postings, calls
    /// `start_doc` / `add_position` / `finish_doc`, and finally `finish_term`.
    ///
    /// The postings enum is Java's: `postingsEnum = termsEnum.postings(postingsEnum,
    /// enumFlags)` (`PushPostingsWriterBase.java:125`) offers the enum of the
    /// previous term, whatever its field, and keeps the one returned for the
    /// next term — also when walking it fails, as Java's field keeps it.
    /// Rust moves the offered enum into `postings`, so an error raised by
    /// `postings` itself drops it, where Java's field would still hold it.
    fn write_term(
        &mut self,
        _term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: Option<&dyn NormsProducer>,
    ) -> Result<Option<BlockTermState>> {
        let field_info = self
            .push_state()
            .field_info
            .as_ref()
            .ok_or_else(|| LuceneError::illegal_state("set_field not called".to_string()))?
            .clone();

        // A fresh, forward-only iterator over the field's norms for every term
        // (`PushPostingsWriterBase.java:118-124`): the writer reads only this
        // term's documents from it, with `advanceExact`.
        let norm_values = if !field_info.has_norms() {
            None
        } else {
            let norms = norms.ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "field \"{}\" has norms, but no norms producer was given",
                    field_info.name
                ))
            })?;
            norms.get_norms(&field_info)?
        };
        self.start_term(norm_values)?;

        let enum_flags = self.push_state().enum_flags;
        let write_freqs = self.push_state().write_freqs;
        let write_positions = self.push_state().write_positions;
        let write_payloads = self.push_state().write_payloads;
        let write_offsets = self.push_state().write_offsets;

        let reuse = self.push_state_mut().postings_enum.take();
        let postings_enum = terms_enum.postings(reuse, enum_flags)?;
        debug_assert!(postings_enum.is_some());
        let mut postings_enum = postings_enum.ok_or_else(LuceneError::null_pointer)?;
        let walked = push_postings(
            self,
            postings_enum.as_mut(),
            docs_seen,
            write_freqs,
            write_positions,
            write_payloads,
            write_offsets,
        );
        self.push_state_mut().postings_enum = Some(postings_enum);
        let (doc_freq, total_term_freq) = walked?;

        if doc_freq == 0 {
            Ok(None)
        } else {
            let mut state = self.new_term_state()?;
            state.doc_freq = doc_freq;
            state.total_term_freq = if write_freqs { total_term_freq } else { -1 };
            self.finish_term(&mut state)?;
            Ok(Some(state))
        }
    }
}

/// Pushes every document, frequency and position of `postings_enum` and
/// returns the term's `docFreq` and `totalTermFreq`.
///
/// The `while (true)` loop of `PushPostingsWriterBase.writeTerm`
/// (`PushPostingsWriterBase.java:130-164`), split out so that
/// [`write_term`](PushPostingsWriterBase::write_term) keeps the enum whether or not the
/// loop fails.
fn push_postings<W: PushPostingsWriterBase + ?Sized>(
    writer: &mut W,
    postings_enum: &mut dyn PostingsEnum,
    docs_seen: &mut FixedBitSet,
    write_freqs: bool,
    write_positions: bool,
    write_payloads: bool,
    write_offsets: bool,
) -> Result<(i32, i64)> {
    let mut doc_freq = 0i32;
    let mut total_term_freq = 0i64;
    loop {
        let doc_id = postings_enum.next_doc()?;
        if doc_id == NO_MORE_DOCS {
            break;
        }
        doc_freq += 1;
        // `docsSeen.set(docID)` (`PushPostingsWriterBase.java:136`): a
        // document past the bit set fails, as Java's `assert index < numBits`
        // or array bound does.
        docs_seen.set(doc_id as usize);
        let freq = if write_freqs {
            let freq = postings_enum.freq()?;
            total_term_freq += i64::from(freq);
            freq
        } else {
            -1
        };
        writer.start_doc(doc_id, freq)?;

        if write_positions {
            for _ in 0..freq {
                let position = postings_enum.next_position()?;
                let payload = if write_payloads {
                    postings_enum.get_payload()?
                } else {
                    None
                };
                let (start_offset, end_offset) = if write_offsets {
                    (postings_enum.start_offset(), postings_enum.end_offset())
                } else {
                    (-1, -1)
                };
                writer.add_position(position, payload, start_offset, end_offset)?;
            }
        }
        writer.finish_doc()?;
    }
    Ok((doc_freq, total_term_freq))
}

impl<T: PushPostingsWriterBase + ?Sized> PostingsWriterBase for T {
    fn init(&mut self, terms_out: &mut dyn IndexOutput, state: &SegmentWriteState) -> Result<()> {
        PushPostingsWriterBase::init(self, terms_out, state)
    }

    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()> {
        PushPostingsWriterBase::encode_term(self, out, field_info, state, absolute)
    }

    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()> {
        PushPostingsWriterBase::set_field(self, field_info)
    }

    fn write_term(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: Option<&dyn NormsProducer>,
    ) -> Result<Option<BlockTermState>> {
        PushPostingsWriterBase::write_term(self, term, terms_enum, docs_seen, norms)
    }

    fn close(&mut self) -> Result<()> {
        PushPostingsWriterBase::close(self)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;
    use std::sync::Arc;

    // Stub Fields -----------------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubFields;

    impl Fields for StubFields {
        fn size(&self) -> i32 {
            0
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty::<String>())
        }
    }

    // Stub TermsEnum --------------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubTermsEnum {
        atts: crate::util::attribute::AttributeSource,
    }

    impl std::fmt::Display for StubTermsEnum {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubTermsEnum")
        }
    }

    impl TermsEnum for StubTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> Result<Option<BytesRef>> {
            Ok(Some(BytesRef::default()))
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Option<Box<dyn PostingsEnum>>> {
            Ok(Some(Box::new(StubPostingsEnum::default())))
        }

        fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
            Ok(false)
        }

        fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
            Ok(SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> Result<()> {
            Ok(())
        }

        fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
            Ok(())
        }

        fn ord(&self) -> Result<i64> {
            Ok(-1)
        }

        fn doc_freq(&mut self) -> Result<i32> {
            Ok(0)
        }

        fn total_term_freq(&mut self) -> Result<i64> {
            Ok(0)
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
            Ok(Box::new(StubImpactsEnum::default()))
        }

        fn term_state(&mut self) -> Result<Box<dyn TermState>> {
            Ok(Box::new(
                crate::codecs::term_state::BlockTermState::default(),
            ))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            Ok(None)
        }
    }

    // Stub PostingsEnum -----------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsEnum {
        doc: i32,
    }

    impl DocIdSetIterator for StubPostingsEnum {
        fn doc_id(&self) -> i32 {
            self.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.doc = NO_MORE_DOCS;
            Ok(NO_MORE_DOCS)
        }

        fn advance(&mut self, _target: i32) -> Result<i32> {
            self.doc = NO_MORE_DOCS;
            Ok(NO_MORE_DOCS)
        }

        fn cost(&self) -> i64 {
            0
        }
    }

    impl std::fmt::Display for StubPostingsEnum {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubPostingsEnum")
        }
    }

    impl PostingsEnum for StubPostingsEnum {
        fn freq(&self) -> Result<i32> {
            Ok(0)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    // Stub ImpactsEnum ------------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubImpactsEnum;

    impl DocIdSetIterator for StubImpactsEnum {
        fn doc_id(&self) -> i32 {
            NO_MORE_DOCS
        }

        fn next_doc(&mut self) -> Result<i32> {
            Ok(NO_MORE_DOCS)
        }

        fn advance(&mut self, _target: i32) -> Result<i32> {
            Ok(NO_MORE_DOCS)
        }

        fn cost(&self) -> i64 {
            0
        }
    }

    impl std::fmt::Display for StubImpactsEnum {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubImpactsEnum")
        }
    }

    impl PostingsEnum for StubImpactsEnum {
        fn freq(&self) -> Result<i32> {
            Ok(0)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    impl crate::index::ImpactsSource for StubImpactsEnum {
        fn advance_shallow(&mut self, _target: i32) -> Result<()> {
            Ok(())
        }

        fn get_impacts(&mut self) -> Result<Box<dyn crate::index::Impacts>> {
            Ok(Box::new(StubImpacts::default()))
        }
    }

    impl ImpactsEnum for StubImpactsEnum {}

    #[derive(Debug, Default, Clone)]
    struct StubImpacts;

    impl crate::index::Impacts for StubImpacts {
        fn num_levels(&self) -> i32 {
            1
        }

        fn doc_id_up_to(&self, _level: i32) -> i32 {
            i32::MAX
        }

        fn get_impacts(&self, _level: i32) -> crate::index::FreqAndNormBuffer {
            let mut buf = crate::index::FreqAndNormBuffer::new();
            buf.add(1, 1);
            buf
        }
    }

    // Stub FieldsConsumer ---------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubFieldsConsumer;

    impl FieldsConsumer for StubFieldsConsumer {
        fn write(
            &mut self,
            _fields: &dyn Fields,
            _norms: Option<&dyn NormsProducer>,
        ) -> Result<()> {
            Ok(())
        }

        fn merge(
            &mut self,
            _merge_state: &MergeState,
            _norms: Option<&dyn NormsProducer>,
        ) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub FieldsProducer ---------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubFieldsProducer;

    impl Fields for StubFieldsProducer {
        fn size(&self) -> i32 {
            0
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty::<String>())
        }
    }

    impl FieldsProducer for StubFieldsProducer {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub PostingsFormat ---------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsFormat;

    impl PostingsFormat for StubPostingsFormat {
        fn name(&self) -> &str {
            "StubPostingsFormat"
        }

        fn fields_consumer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>> {
            Ok(Box::new(StubFieldsConsumer))
        }

        fn fields_producer<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(StubFieldsProducer))
        }
    }

    // Stub PostingsReaderBase ----------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsReader;

    impl fmt::Display for StubPostingsReader {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("StubPostingsReader()")
        }
    }

    impl PostingsReaderBase for StubPostingsReader {
        fn init(
            &mut self,
            _terms_in: &mut dyn IndexInput,
            _state: &SegmentReadState,
        ) -> Result<()> {
            Ok(())
        }

        fn new_term_state(&self) -> Result<BlockTermState> {
            Ok(BlockTermState::default())
        }

        fn decode_term(
            &self,
            _input: &mut dyn DataInput,
            _field_info: &FieldInfo,
            _state: &mut BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn postings(
            &self,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(StubPostingsEnum::default()))
        }

        fn impacts(
            &self,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _flags: i32,
        ) -> Result<Box<dyn ImpactsEnum>> {
            Ok(Box::new(StubImpactsEnum))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub PushPostingsWriterBase -------------------------------------------
    #[derive(Debug, Default)]
    struct StubPushWriter {
        state: PushPostingsWriterState,
    }

    impl PushPostingsWriterBase for StubPushWriter {
        fn push_state(&self) -> &PushPostingsWriterState {
            &self.state
        }

        fn push_state_mut(&mut self) -> &mut PushPostingsWriterState {
            &mut self.state
        }

        fn init(
            &mut self,
            _terms_out: &mut dyn IndexOutput,
            _state: &SegmentWriteState,
        ) -> Result<()> {
            Ok(())
        }

        fn encode_term(
            &mut self,
            _out: &mut dyn DataOutput,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn new_term_state(&self) -> Result<BlockTermState> {
            Ok(BlockTermState::default())
        }

        fn start_term(&mut self, _norms: Option<Box<dyn NumericDocValues>>) -> Result<()> {
            Ok(())
        }

        fn finish_term(&mut self, _state: &mut BlockTermState) -> Result<()> {
            Ok(())
        }

        fn start_doc(&mut self, _doc_id: i32, _freq: i32) -> Result<()> {
            Ok(())
        }

        fn add_position(
            &mut self,
            _position: i32,
            _payload: Option<&[u8]>,
            _start_offset: i32,
            _end_offset: i32,
        ) -> Result<()> {
            Ok(())
        }

        fn finish_doc(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub standalone PostingsWriterBase -------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsWriter;

    impl PostingsWriterBase for StubPostingsWriter {
        fn init(
            &mut self,
            _terms_out: &mut dyn IndexOutput,
            _state: &SegmentWriteState,
        ) -> Result<()> {
            Ok(())
        }

        fn write_term(
            &mut self,
            _term: &BytesRef,
            _terms_enum: &mut dyn TermsEnum,
            _docs_seen: &mut FixedBitSet,
            _norms: Option<&dyn NormsProducer>,
        ) -> Result<Option<BlockTermState>> {
            Ok(None)
        }

        fn encode_term(
            &mut self,
            _out: &mut dyn DataOutput,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn set_field(&mut self, _field_info: &FieldInfo) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stub_postings_format_compiles_and_runs() {
        let format = StubPostingsFormat;
        assert_eq!(format.name(), "StubPostingsFormat");

        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let dir_ref: &dyn crate::store::Directory = &*dir;
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::ByteBuffersDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let seg_updates = crate::codecs::stub::BufferedUpdates::default();
        let info_stream = &crate::util::NoOutputInfoStream;
        let write_state = SegmentWriteState::new(
            info_stream,
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &seg_updates,
            context,
        );
        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);

        let mut consumer = format.fields_consumer(&write_state).unwrap();
        consumer.write(&StubFields, None).unwrap();
        consumer
            .merge(
                &MergeState::from_fields_producers(Vec::new(), Vec::new()),
                None,
            )
            .unwrap();
        consumer.close().unwrap();

        let mut producer = format.fields_producer(&read_state).unwrap();
        assert_eq!(producer.size(), 0);
        producer.check_integrity().unwrap();
        let _merge_instance = producer.get_merge_instance().unwrap();
        producer.close().unwrap();
    }

    #[test]
    fn stub_postings_reader_base_compiles_and_runs() {
        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let dir_ref: &dyn crate::store::Directory = &*dir;
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::ByteBuffersDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);

        let mut reader = StubPostingsReader;
        reader
            .init(
                &mut crate::store::MockIndexInput::new(vec![], "test"),
                &read_state,
            )
            .unwrap();
        let state = reader.new_term_state().unwrap();
        assert_eq!(state.doc_freq, 0);
        reader
            .decode_term(
                &mut crate::store::ByteArrayDataInput::empty(),
                &FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false),
                &mut BlockTermState::default(),
                true,
            )
            .unwrap();
        let _enum_ = reader
            .postings(
                &FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false),
                &BlockTermState::default(),
                None,
                POSTINGS_ENUM_NONE,
            )
            .unwrap();
        let _impacts = reader
            .impacts(
                &FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false),
                &BlockTermState::default(),
                POSTINGS_ENUM_NONE,
            )
            .unwrap();
        reader.check_integrity().unwrap();
        reader.close().unwrap();
    }

    #[test]
    fn stub_push_postings_writer_blanket_impl_runs() {
        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::ByteBuffersDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let seg_updates = crate::codecs::stub::BufferedUpdates::default();
        let info_stream = &crate::util::NoOutputInfoStream;
        let write_state = SegmentWriteState::new(
            info_stream,
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &seg_updates,
            context,
        );

        let mut writer = StubPushWriter::default();
        // `StubPushWriter` implements `PushPostingsWriterBase`; the blanket
        // impl provides `PostingsWriterBase`.
        PostingsWriterBase::init(
            &mut writer,
            &mut crate::store::MockIndexOutput::new("test", "test"),
            &write_state,
        )
        .unwrap();

        let field_info =
            FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false);
        crate::codecs::postings::PostingsWriterBase::set_field(&mut writer, &field_info).unwrap();

        let mut docs_seen = FixedBitSet::new(8);
        let result = PostingsWriterBase::write_term(
            &mut writer,
            &BytesRef::default(),
            &mut StubTermsEnum::default(),
            &mut docs_seen,
            None,
        )
        .unwrap();
        assert!(result.is_none());

        PostingsWriterBase::encode_term(
            &mut writer,
            &mut crate::store::VecDataOutput::new(),
            &field_info,
            &BlockTermState::default(),
            true,
        )
        .unwrap();
        PostingsWriterBase::close(&mut writer).unwrap();
    }

    #[test]
    fn stub_postings_writer_base_compiles_and_runs() {
        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::default());
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::ByteBuffersDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let seg_updates = crate::codecs::stub::BufferedUpdates::default();
        let info_stream = &crate::util::NoOutputInfoStream;
        let write_state = SegmentWriteState::new(
            info_stream,
            Arc::clone(&dir),
            &segment_info,
            &field_infos,
            &seg_updates,
            context,
        );

        let mut writer = StubPostingsWriter;
        writer
            .init(
                &mut crate::store::MockIndexOutput::new("test", "test"),
                &write_state,
            )
            .unwrap();
        let field_info =
            FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false);
        writer.set_field(&field_info).unwrap();
        let mut docs_seen = FixedBitSet::new(8);
        let result = writer
            .write_term(
                &BytesRef::default(),
                &mut StubTermsEnum::default(),
                &mut docs_seen,
                None,
            )
            .unwrap();
        assert!(result.is_none());
        writer
            .encode_term(
                &mut crate::store::VecDataOutput::new(),
                &field_info,
                &BlockTermState::default(),
                true,
            )
            .unwrap();
        writer.close().unwrap();
    }

    /// A terms enum whose `postings` records the enum it is offered and
    /// answers a new one, keeping every offered enum alive so that no address
    /// is handed out twice.
    #[derive(Default)]
    struct OfferRecordingTermsEnum {
        atts: crate::util::attribute::AttributeSource,
        offered: Vec<Option<usize>>,
        returned: Vec<usize>,
        kept: Vec<Box<dyn PostingsEnum>>,
    }

    fn identity(postings: &dyn PostingsEnum) -> usize {
        postings as *const dyn PostingsEnum as *const () as usize
    }

    impl std::fmt::Display for OfferRecordingTermsEnum {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("OfferRecordingTermsEnum")
        }
    }

    impl TermsEnum for OfferRecordingTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> Result<Option<BytesRef>> {
            Ok(Some(BytesRef::default()))
        }

        fn postings(
            &mut self,
            reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Option<Box<dyn PostingsEnum>>> {
            self.offered.push(reuse.as_deref().map(identity));
            self.kept.extend(reuse);
            let postings: Box<dyn PostingsEnum> = Box::new(StubPostingsEnum::default());
            self.returned.push(identity(postings.as_ref()));
            Ok(Some(postings))
        }

        fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
            Ok(SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> Result<()> {
            Ok(())
        }

        fn ord(&self) -> Result<i64> {
            Ok(-1)
        }

        fn doc_freq(&mut self) -> Result<i32> {
            Ok(0)
        }

        fn total_term_freq(&mut self) -> Result<i64> {
            Ok(0)
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
            Ok(Box::new(StubImpactsEnum))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            Ok(None)
        }
    }

    /// `PushPostingsWriterBase.writeTerm` offers each term the enum the
    /// previous term got — `postingsEnum = termsEnum.postings(postingsEnum,
    /// enumFlags)` (`PushPostingsWriterBase.java:125`) — across fields too:
    /// nothing ever resets the field.
    #[test]
    fn write_term_offers_each_term_the_enum_the_previous_term_got() {
        let mut writer = StubPushWriter::default();
        let mut terms = OfferRecordingTermsEnum::default();
        let mut docs_seen = FixedBitSet::new(8);
        let docs =
            FieldInfo::new("docs", 0).with_postings_options(IndexOptions::DOCS, false, false);
        let freqs = FieldInfo::new("freqs", 1).with_postings_options(
            IndexOptions::DOCS_AND_FREQS,
            false,
            false,
        );
        PushPostingsWriterBase::set_field(&mut writer, &docs).unwrap();
        for _ in 0..2 {
            PostingsWriterBase::write_term(
                &mut writer,
                &BytesRef::default(),
                &mut terms,
                &mut docs_seen,
                None,
            )
            .unwrap();
        }
        PushPostingsWriterBase::set_field(&mut writer, &freqs).unwrap();
        PostingsWriterBase::write_term(
            &mut writer,
            &BytesRef::default(),
            &mut terms,
            &mut docs_seen,
            None,
        )
        .unwrap();
        assert_eq!(
            terms.offered,
            vec![None, Some(terms.returned[0]), Some(terms.returned[1])]
        );
    }

    #[test]
    fn push_writer_set_field_computes_flags() {
        let mut writer = StubPushWriter::default();
        let field_info = FieldInfo::new("text", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            false,
            true,
        );
        crate::codecs::postings::PushPostingsWriterBase::set_field(&mut writer, &field_info)
            .unwrap();
        assert_eq!(writer.push_state().enum_flags, POSTINGS_ENUM_PAYLOADS);
        assert!(writer.push_state().write_positions);
        assert!(writer.push_state().write_payloads);
    }

    /// A postings enum over one document, `doc`, whose frequency is `freq`.
    struct OneDocPostingsEnum {
        doc: i32,
        current: i32,
        freq: i32,
    }

    impl DocIdSetIterator for OneDocPostingsEnum {
        fn doc_id(&self) -> i32 {
            self.current
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.current = if self.current == -1 {
                self.doc
            } else {
                NO_MORE_DOCS
            };
            Ok(self.current)
        }

        fn advance(&mut self, _target: i32) -> Result<i32> {
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            1
        }
    }

    impl std::fmt::Display for OneDocPostingsEnum {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("OneDocPostingsEnum")
        }
    }

    impl PostingsEnum for OneDocPostingsEnum {
        fn freq(&self) -> Result<i32> {
            Ok(self.freq)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(0)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    /// A terms enum whose `postings` is one document, `doc`, of frequency
    /// `freq`.
    #[derive(Default)]
    struct OneDocTermsEnum {
        atts: crate::util::attribute::AttributeSource,
        doc: i32,
        freq: i32,
    }

    impl std::fmt::Display for OneDocTermsEnum {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("OneDocTermsEnum")
        }
    }

    impl TermsEnum for OneDocTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> Result<Option<BytesRef>> {
            Ok(Some(BytesRef::default()))
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Option<Box<dyn PostingsEnum>>> {
            Ok(Some(Box::new(OneDocPostingsEnum {
                doc: self.doc,
                current: -1,
                freq: self.freq,
            })))
        }

        fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
            Ok(SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> Result<()> {
            Ok(())
        }

        fn ord(&self) -> Result<i64> {
            Ok(-1)
        }

        fn doc_freq(&mut self) -> Result<i32> {
            Ok(1)
        }

        fn total_term_freq(&mut self) -> Result<i64> {
            Ok(1)
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
            Ok(Box::new(StubImpactsEnum))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            Ok(None)
        }
    }

    /// `writeTerm` calls `docsSeen.set(docID)` for every document
    /// (`PushPostingsWriterBase.java:136`), with no bound check of its own: a
    /// document past the bit set fails exactly as `docsSeen.set(docID)` does
    /// on its own — `FixedBitSet.set`'s `assert index >= 0 && index <
    /// numBits` (`FixedBitSet.java:298`) — rather than being skipped or
    /// refused by `writeTerm`.
    #[test]
    fn write_term_hands_every_document_to_docs_seen_set() {
        fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default()
        }
        let direct = std::panic::catch_unwind(|| FixedBitSet::new(8).set(9))
            .expect_err("FixedBitSet::set refuses a bit past its length");
        let through_write_term = std::panic::catch_unwind(|| {
            let mut writer = StubPushWriter::default();
            let docs =
                FieldInfo::new("docs", 0).with_postings_options(IndexOptions::DOCS, false, false);
            PushPostingsWriterBase::set_field(&mut writer, &docs).unwrap();
            let mut terms = OneDocTermsEnum {
                doc: 9,
                freq: 1,
                ..OneDocTermsEnum::default()
            };
            let mut docs_seen = FixedBitSet::new(8);
            PostingsWriterBase::write_term(
                &mut writer,
                &BytesRef::default(),
                &mut terms,
                &mut docs_seen,
                None,
            )
            .map(|_| ())
        })
        .expect_err("writeTerm reaches docsSeen.set(9)");
        assert_eq!(panic_text(through_write_term), panic_text(direct));
    }

    /// `writeTerm` adds every frequency it reads to `totalTermFreq`
    /// (`PushPostingsWriterBase.java:139-141`) and sets the document's bit,
    /// without filtering what the enum reports.
    #[test]
    fn write_term_sums_every_frequency_it_reads() {
        let mut writer = StubPushWriter::default();
        let freqs = FieldInfo::new("freqs", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS,
            false,
            false,
        );
        PushPostingsWriterBase::set_field(&mut writer, &freqs).unwrap();
        let mut terms = OneDocTermsEnum {
            doc: 3,
            freq: -1,
            ..OneDocTermsEnum::default()
        };
        let mut docs_seen = FixedBitSet::new(8);
        let state = PostingsWriterBase::write_term(
            &mut writer,
            &BytesRef::default(),
            &mut terms,
            &mut docs_seen,
            None,
        )
        .unwrap()
        .expect("one document");
        assert_eq!(state.doc_freq, 1);
        assert_eq!(state.total_term_freq, -1);
        assert!(docs_seen.get(3));
    }
}
