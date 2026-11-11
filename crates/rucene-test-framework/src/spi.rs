/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The `META-INF/services` files of `lucene-test-framework` 10.5.0.
//!
//! Java finds `AssertingCodec`, `MockRandomPostingsFormat`, ... because
//! `lucene-test-framework.jar` carries
//! `META-INF/services/org.apache.lucene.codecs.{Codec,DocValuesFormat,KnnVectorsFormat,PostingsFormat}`,
//! which `java.util.ServiceLoader` reads from the class path. As
//! `rucene_codecs::class_loader()` does for `lucene-codecs`, this module is
//! the loader that sees this crate's service files; an application puts the
//! crate "on the class path" with the reload functions:
//!
//! ```
//! let loader = rucene_test_framework::class_loader();
//! rucene::codecs::reload_codecs(loader).unwrap();
//! rucene::codecs::postings::reload_postings_formats(loader).unwrap();
//! assert_eq!(rucene::codecs::for_name("Asserting").unwrap().name(), "Asserting");
//! assert_eq!(
//!     rucene::codecs::postings_for_name("MockRandom").unwrap().name(),
//!     "MockRandom"
//! );
//! ```

use std::sync::{Arc, LazyLock};

use rucene::codecs::{Codec, DocValuesFormat, KnnVectorsFormat, PostingsFormat};
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene::util::spi::NamedSPIProvider;

use crate::codecs::asserting::{
    AssertingCodec, AssertingDocValuesFormat, AssertingKnnVectorsFormat, AssertingPostingsFormat,
};
use crate::codecs::blockterms::{
    LuceneFixedGap, LuceneVarGapDocFreqInterval, LuceneVarGapFixedInterval,
};
use crate::codecs::bloom::TestBloomFilteredLucenePostings;
use crate::codecs::cheapbastard::CheapBastardCodec;
use crate::codecs::compressing::dummy::DummyCompressingCodec;
use crate::codecs::compressing::{
    CompressingCodec, DeflateWithPresetCompressingCodec, FastCompressingCodec,
    FastDecompressionCompressingCodec, HighCompressionCompressingCodec,
    LZ4WithPresetCompressingCodec,
};
use crate::codecs::mockrandom::MockRandomPostingsFormat;
use crate::codecs::ramonly::RAMOnlyPostingsFormat;
use crate::codecs::uniformsplit::sharedterms::STUniformSplitRot13PostingsFormat;
use crate::codecs::uniformsplit::UniformSplitRot13PostingsFormat;
use crate::codecs::vector::ConfigurableMCodec;

/// A compressing codec's public no-argument constructor, whose fixed
/// settings `CompressingCodec`'s validation accepts.
fn compressing(codec: rucene::error::Result<CompressingCodec>) -> Arc<dyn Codec> {
    Arc::new(codec.expect("INVARIANT: the no-argument constructors pass valid settings"))
}

/// One provider of the service `service`, constructed by `make` (the class's
/// public no-argument constructor).
fn provider<S: ?Sized + Send + Sync + 'static>(
    service: &str,
    class: String,
    make: impl Fn() -> Arc<S> + Send + Sync + 'static,
) -> ServiceProvider {
    ServiceProvider::new(service, class, NamedSPIProvider::new(make))
}

/// The class loader that sees `lucene-test-framework`'s `META-INF/services`
/// files, a child of `lucene-codecs`' ([`rucene_codecs::class_loader`]), so
/// that a scan of it lists `lucene-core`'s and `lucene-codecs`' providers
/// first.
///
/// | service | providers |
/// | --- | --- |
/// | `org.apache.lucene.codecs.Codec` | `AssertingCodec`, `CheapBastardCodec`, `DeflateWithPresetCompressingCodec`, `FastCompressingCodec`, `FastDecompressionCompressingCodec`, `HighCompressionCompressingCodec`, `LZ4WithPresetCompressingCodec`, `DummyCompressingCodec`, `ConfigurableMCodec` |
/// | `org.apache.lucene.codecs.DocValuesFormat` | `AssertingDocValuesFormat` |
/// | `org.apache.lucene.codecs.KnnVectorsFormat` | `AssertingKnnVectorsFormat` |
/// | `org.apache.lucene.codecs.PostingsFormat` | `MockRandomPostingsFormat`, `RAMOnlyPostingsFormat`, `LuceneFixedGap`, `LuceneVarGapFixedInterval`, `LuceneVarGapDocFreqInterval`, `TestBloomFilteredLucenePostings`, `AssertingPostingsFormat`, `UniformSplitRot13PostingsFormat`, `STUniformSplitRot13PostingsFormat` |
///
/// Every line of the four service files of
/// `lucene/test-framework/src/resources/META-INF/services/`, in their order.
pub fn class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        const CODEC: &str = "org.apache.lucene.codecs.Codec";
        const DOC_VALUES: &str = "org.apache.lucene.codecs.DocValuesFormat";
        const KNN: &str = "org.apache.lucene.codecs.KnnVectorsFormat";
        const POSTINGS: &str = "org.apache.lucene.codecs.PostingsFormat";
        const T: &str = "org.apache.lucene.tests.codecs";
        let class = |name: &str| format!("{T}.{name}");
        ClassLoader::with_services(
            "lucene-test-framework",
            Some(Arc::clone(rucene_codecs::class_loader())),
            vec![
                // META-INF/services/org.apache.lucene.codecs.Codec
                provider(CODEC, class("asserting.AssertingCodec"), || {
                    Arc::new(AssertingCodec::new()) as Arc<dyn Codec>
                }),
                provider(CODEC, class("cheapbastard.CheapBastardCodec"), || {
                    Arc::new(CheapBastardCodec::new()) as Arc<dyn Codec>
                }),
                provider(
                    CODEC,
                    class("compressing.DeflateWithPresetCompressingCodec"),
                    || compressing(DeflateWithPresetCompressingCodec::default_instance()),
                ),
                provider(CODEC, class("compressing.FastCompressingCodec"), || {
                    compressing(FastCompressingCodec::default_instance())
                }),
                provider(
                    CODEC,
                    class("compressing.FastDecompressionCompressingCodec"),
                    || compressing(FastDecompressionCompressingCodec::default_instance()),
                ),
                provider(
                    CODEC,
                    class("compressing.HighCompressionCompressingCodec"),
                    || compressing(HighCompressionCompressingCodec::default_instance()),
                ),
                provider(
                    CODEC,
                    class("compressing.LZ4WithPresetCompressingCodec"),
                    || compressing(LZ4WithPresetCompressingCodec::default_instance()),
                ),
                provider(
                    CODEC,
                    class("compressing.dummy.DummyCompressingCodec"),
                    || compressing(DummyCompressingCodec::default_instance()),
                ),
                provider(CODEC, class("vector.ConfigurableMCodec"), || {
                    Arc::new(ConfigurableMCodec::new()) as Arc<dyn Codec>
                }),
                // META-INF/services/org.apache.lucene.codecs.DocValuesFormat
                provider(
                    DOC_VALUES,
                    class("asserting.AssertingDocValuesFormat"),
                    || Arc::new(AssertingDocValuesFormat::new()) as Arc<dyn DocValuesFormat>,
                ),
                // META-INF/services/org.apache.lucene.codecs.KnnVectorsFormat
                provider(KNN, class("asserting.AssertingKnnVectorsFormat"), || {
                    Arc::new(AssertingKnnVectorsFormat::new()) as Arc<dyn KnnVectorsFormat>
                }),
                // META-INF/services/org.apache.lucene.codecs.PostingsFormat
                provider(
                    POSTINGS,
                    class("mockrandom.MockRandomPostingsFormat"),
                    || Arc::new(MockRandomPostingsFormat::new()) as Arc<dyn PostingsFormat>,
                ),
                provider(POSTINGS, class("ramonly.RAMOnlyPostingsFormat"), || {
                    Arc::new(RAMOnlyPostingsFormat::new()) as Arc<dyn PostingsFormat>
                }),
                provider(POSTINGS, class("blockterms.LuceneFixedGap"), || {
                    Arc::new(LuceneFixedGap::new()) as Arc<dyn PostingsFormat>
                }),
                provider(
                    POSTINGS,
                    class("blockterms.LuceneVarGapFixedInterval"),
                    || Arc::new(LuceneVarGapFixedInterval::new()) as Arc<dyn PostingsFormat>,
                ),
                provider(
                    POSTINGS,
                    class("blockterms.LuceneVarGapDocFreqInterval"),
                    || Arc::new(LuceneVarGapDocFreqInterval::new()) as Arc<dyn PostingsFormat>,
                ),
                provider(
                    POSTINGS,
                    class("bloom.TestBloomFilteredLucenePostings"),
                    || Arc::new(TestBloomFilteredLucenePostings::new()) as Arc<dyn PostingsFormat>,
                ),
                provider(POSTINGS, class("asserting.AssertingPostingsFormat"), || {
                    Arc::new(AssertingPostingsFormat::new()) as Arc<dyn PostingsFormat>
                }),
                provider(
                    POSTINGS,
                    class("uniformsplit.UniformSplitRot13PostingsFormat"),
                    || Arc::new(UniformSplitRot13PostingsFormat::new()) as Arc<dyn PostingsFormat>,
                ),
                provider(
                    POSTINGS,
                    class("uniformsplit.sharedterms.STUniformSplitRot13PostingsFormat"),
                    || {
                        Arc::new(STUniformSplitRot13PostingsFormat::new())
                            as Arc<dyn PostingsFormat>
                    },
                ),
            ],
        )
    });
    &LOADER
}
