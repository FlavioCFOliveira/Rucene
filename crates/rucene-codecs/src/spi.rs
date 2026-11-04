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

use rucene::codecs::{Codec, KnnVectorsFormat, PostingsFormat};
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene::util::spi::{lucene_core_class_loader, NamedSPIProvider};

use crate::bitvectors::HnswBitVectorsFormat;
use crate::blocktreeords::BlockTreeOrdsPostingsFormat;
use crate::bloom::BloomFilteringPostingsFormat;
use crate::memory::{DirectPostingsFormat, FSTPostingsFormat};
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
/// | `org.apache.lucene.codecs.KnnVectorsFormat` | `HnswBitVectorsFormat` |
/// | `org.apache.lucene.codecs.PostingsFormat` | `BlockTreeOrdsPostingsFormat`, `BloomFilteringPostingsFormat`, `DirectPostingsFormat`, `FSTPostingsFormat` |
///
/// **Divergence.** `lucene-codecs` 10.5.0 also lists
/// `UniformSplitPostingsFormat` and `STUniformSplitPostingsFormat` under
/// `org.apache.lucene.codecs.PostingsFormat`
/// (`lucene/codecs/src/resources/META-INF/services/`). Those classes are not
/// ported yet, and a service line whose class cannot be loaded makes every
/// scan of the service fail (`ServiceConfigurationError: Provider ... not
/// found`), so their lines are left out until the tasks that port the
/// classes add them; the lines present keep the file's order.
pub fn class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "lucene-codecs",
            Some(Arc::clone(lucene_core_class_loader())),
            vec![
                ServiceProvider::new(
                    "org.apache.lucene.codecs.Codec",
                    "org.apache.lucene.codecs.simpletext.SimpleTextCodec",
                    NamedSPIProvider::new(|| Arc::new(SimpleTextCodec::new()) as Arc<dyn Codec>),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.KnnVectorsFormat",
                    "org.apache.lucene.codecs.bitvectors.HnswBitVectorsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(HnswBitVectorsFormat::new()) as Arc<dyn KnnVectorsFormat>
                    }),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "org.apache.lucene.codecs.blocktreeords.BlockTreeOrdsPostingsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(BlockTreeOrdsPostingsFormat::new()) as Arc<dyn PostingsFormat>
                    }),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "org.apache.lucene.codecs.bloom.BloomFilteringPostingsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(BloomFilteringPostingsFormat::for_reading())
                            as Arc<dyn PostingsFormat>
                    }),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "org.apache.lucene.codecs.memory.DirectPostingsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(DirectPostingsFormat::new()) as Arc<dyn PostingsFormat>
                    }),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.codecs.PostingsFormat",
                    "org.apache.lucene.codecs.memory.FSTPostingsFormat",
                    NamedSPIProvider::new(|| {
                        Arc::new(FSTPostingsFormat::new()) as Arc<dyn PostingsFormat>
                    }),
                ),
            ],
        )
    });
    &LOADER
}
