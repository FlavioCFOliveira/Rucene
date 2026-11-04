//! The `META-INF/services` files of `lucene-codecs` 10.5.0.
//!
//! Java finds `SimpleTextCodec` because `lucene-codecs.jar` carries
//! `META-INF/services/org.apache.lucene.codecs.Codec`, which
//! `java.util.ServiceLoader` reads from the class path. Rucene models class
//! loaders and their service files explicitly
//! ([`ClassLoader`](rucene::util::resource_loader::ClassLoader)); this module
//! is the loader that sees this crate's service files, and an application
//! puts the crate "on the class path" the way Java does at run time, with
//! `Codec.reloadCodecs(ClassLoader)`:
//!
//! ```
//! rucene::codecs::reload_codecs(rucene_codecs::class_loader()).unwrap();
//! assert_eq!(rucene::codecs::for_name("SimpleText").unwrap().name(), "SimpleText");
//! ```

use std::sync::{Arc, LazyLock};

use rucene::codecs::Codec;
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene::util::spi::{lucene_core_class_loader, NamedSPIProvider};

use crate::simpletext::SimpleTextCodec;

/// The class loader that sees `lucene-codecs`' `META-INF/services` files, a
/// child of `lucene-core`'s
/// ([`lucene_core_class_loader`](rucene::util::spi::lucene_core_class_loader)),
/// so that a scan of it lists `lucene-core`'s providers first, as a class
/// path holding both jars in that order does.
///
/// | service | providers |
/// | --- | --- |
/// | `org.apache.lucene.codecs.Codec` | `SimpleTextCodec` |
///
/// **Divergence.** `lucene-codecs` 10.5.0 also lists
/// `HnswBitVectorsFormat` under `org.apache.lucene.codecs.KnnVectorsFormat`
/// and five postings formats (`BlockTreeOrdsPostingsFormat`,
/// `BloomFilteringPostingsFormat`, `DirectPostingsFormat`,
/// `FSTPostingsFormat`, `UniformSplitPostingsFormat`,
/// `STUniformSplitPostingsFormat`) under
/// `org.apache.lucene.codecs.PostingsFormat`
/// (`lucene/codecs/src/resources/META-INF/services/`). Those classes are not
/// ported yet, and a service line whose class cannot be loaded makes every
/// scan of the service fail (`ServiceConfigurationError: Provider ... not
/// found`), so the two files are left out until the tasks that port the
/// classes add them.
pub fn class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "lucene-codecs",
            Some(Arc::clone(lucene_core_class_loader())),
            vec![ServiceProvider::new(
                "org.apache.lucene.codecs.Codec",
                "org.apache.lucene.codecs.simpletext.SimpleTextCodec",
                NamedSPIProvider::new(|| Arc::new(SimpleTextCodec::new()) as Arc<dyn Codec>),
            )],
        )
    });
    &LOADER
}
