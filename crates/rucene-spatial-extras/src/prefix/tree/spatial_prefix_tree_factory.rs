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

//! The types declared in `SpatialPrefixTreeFactory.java`, of the Java
//! package `org.apache.lucene.spatial.prefix.tree`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use rucene::error::{LuceneError, Result};
use rucene::util::Version;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::java_map::StringMap;

use crate::jdk::{parse_double, parse_int};
use crate::prefix::tree::geohash_prefix_tree;
use crate::prefix::tree::packed_quad_prefix_tree;
use crate::prefix::tree::quad_prefix_tree;
use crate::prefix::tree::s2_prefix_tree;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// `DEFAULT_GEO_MAX_DETAIL_KM`: 1m.
const DEFAULT_GEO_MAX_DETAIL_KM: f64 = 0.001;

/// The fields of `SpatialPrefixTreeFactory`.
#[derive(Debug, Clone, Default)]
pub struct SpatialPrefixTreeFactoryBase {
    /// `args`: the arguments map given to `init`, kept by reference
    /// (`None` for `null`, before `init`).
    pub args: Option<Arc<dyn StringMap>>,
    /// `ctx`.
    pub ctx: Option<Arc<SpatialContext>>,
    /// `maxLevels` (`None` for `null`).
    pub max_levels: Option<i32>,
    /// `version` (private in Java).
    version: Option<Version>,
}

impl SpatialPrefixTreeFactoryBase {
    /// `args.get(key)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` before `init` (`args` is `null`); what the
    /// map's `get` throws.
    pub fn arg(&self, key: &str) -> Result<Option<String>> {
        match &self.args {
            Some(args) => args.get(key),
            None => Err(LuceneError::null_pointer()),
        }
    }

    /// `ctx`, set by `init`.
    ///
    /// # Panics
    ///
    /// Before `init` (Java's `NullPointerException`).
    pub fn ctx(&self) -> &Arc<SpatialContext> {
        self.ctx
            .as_ref()
            .expect("java.lang.NullPointerException: ctx is set by init()")
    }
}

/// A no-argument factory constructor, as `c.getConstructor().newInstance()`
/// would call it.
pub type FactoryCtor = fn() -> Box<dyn SpatialPrefixTreeFactory>;

/// The classes `makeSPT` can load by name.
///
/// **Divergence.** Java loads a factory class named in the `prefixTree`
/// argument with `classLoader.loadClass(cname)` and instantiates it by
/// reflection (`SpatialPrefixTreeFactory.java:61-79`); Rust has no
/// reflection, so the class is looked up in this process-wide registry,
/// which holds the four factories of this module under their Java binary
/// names and which a crate with its own factory adds to with
/// [`register_factory_class`]. The `ClassLoader` parameter is gone with it.
static FACTORY_CLASSES: LazyLock<RwLock<HashMap<String, FactoryCtor>>> = LazyLock::new(|| {
    let mut map: HashMap<String, FactoryCtor> = HashMap::new();
    map.insert(
        "org.apache.lucene.spatial.prefix.tree.GeohashPrefixTree$Factory".to_string(),
        || Box::new(geohash_prefix_tree::Factory::default()),
    );
    map.insert(
        "org.apache.lucene.spatial.prefix.tree.QuadPrefixTree$Factory".to_string(),
        || Box::new(quad_prefix_tree::Factory::default()),
    );
    map.insert(
        "org.apache.lucene.spatial.prefix.tree.PackedQuadPrefixTree$Factory".to_string(),
        || Box::new(packed_quad_prefix_tree::Factory::default()),
    );
    map.insert(
        "org.apache.lucene.spatial.prefix.tree.S2PrefixTree$Factory".to_string(),
        || Box::new(s2_prefix_tree::Factory::default()),
    );
    RwLock::new(map)
});

/// Adds a factory class that [`make_spt`] can load by its binary name.
pub fn register_factory_class(name: &str, ctor: FactoryCtor) {
    FACTORY_CLASSES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_string(), ctor);
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.SpatialPrefixTreeFactory`
/// (`public abstract class`): abstract factory for creating
/// [`SpatialPrefixTree`] instances with useful defaults and passed on
/// configurations defined in a `Map`.
pub trait SpatialPrefixTreeFactory: Send + Sync {
    /// The `SpatialPrefixTreeFactory` fields.
    fn base(&self) -> &SpatialPrefixTreeFactoryBase;

    /// The `SpatialPrefixTreeFactory` fields, mutably.
    fn base_mut(&mut self) -> &mut SpatialPrefixTreeFactoryBase;

    /// `init(Map<String, String> args, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// The errors of `initVersion` and `initMaxLevels`.
    fn init(&mut self, args: Arc<dyn StringMap>, ctx: Arc<SpatialContext>) -> Result<()> {
        self.base_mut().args = Some(args);
        self.base_mut().ctx = Some(ctx);
        self.init_version()?;
        self.init_max_levels()
    }

    /// `initVersion()`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` wrapping the `ParseException` of an invalid
    /// `version` argument.
    fn init_version(&mut self) -> Result<()> {
        let version = match self.base().arg(VERSION)? {
            None => Version::LATEST,
            Some(version_str) => {
                Version::parse_leniently(&version_str).map_err(LuceneError::runtime_exception)?
            }
        };
        self.set_version(version);
        Ok(())
    }

    /// `initMaxLevels()`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for an invalid `maxLevels` or `maxDistErr`
    /// argument; the errors of `getLevelForDistance`.
    fn init_max_levels(&mut self) -> Result<()> {
        if let Some(ml_str) = self.base().arg(MAX_LEVELS)? {
            self.base_mut().max_levels = Some(parse_int(&ml_str)?);
            return Ok(());
        }
        let degrees = match self.base().arg(MAX_DIST_ERR)? {
            None => {
                if !self.base().ctx().is_geo() {
                    return Ok(()); // let default to max
                }
                DistanceUtils::dist2_degrees(
                    DEFAULT_GEO_MAX_DETAIL_KM,
                    DistanceUtils::EARTH_MEAN_RADIUS_KM,
                )
            }
            Some(max_detail_dist_str) => parse_double(&max_detail_dist_str)?,
        };
        let level = self.get_level_for_distance(degrees)?;
        self.base_mut().max_levels = Some(level);
        Ok(())
    }

    /// `setVersion(Version v)`: set the version of Lucene this tree should
    /// mimic the behavior for for analysis.
    fn set_version(&mut self, v: Version) {
        self.base_mut().version = Some(v);
    }

    /// `getVersion()`: the version of Lucene this tree is mimicking (`None`
    /// before `init`).
    fn get_version(&self) -> Option<Version> {
        self.base().version
    }

    /// `getLevelForDistance(double degrees)`: calls
    /// `SpatialPrefixTree.getLevelForDistance(double)`.
    ///
    /// # Errors
    ///
    /// The errors of building the tree.
    fn get_level_for_distance(&self, degrees: f64) -> Result<i32>;

    /// `newSPT()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of the tree constructor.
    fn new_spt(&self) -> Result<Arc<dyn SpatialPrefixTree>>;
}

/// `PREFIX_TREE`.
pub const PREFIX_TREE: &str = "prefixTree";
/// `MAX_LEVELS`.
pub const MAX_LEVELS: &str = "maxLevels";
/// `MAX_DIST_ERR`.
pub const MAX_DIST_ERR: &str = "maxDistErr";
/// `VERSION`.
pub const VERSION: &str = "version";

/// `SpatialPrefixTreeFactory.makeSPT(Map<String, String> args, ClassLoader
/// classLoader, SpatialContext ctx)`: the factory method to create a prefix
/// tree based on the arguments. The `prefixTree` argument names `geohash`,
/// `quad`, `packedQuad` or `s2` (default `geohash` for a geo context, else
/// `quad`), or a factory class; see [`FACTORY_CLASSES`] for the class
/// lookup.
///
/// # Errors
///
/// `RuntimeException` when the factory class cannot be loaded; the errors of
/// `init` and `newSPT`.
pub fn make_spt(
    args: &Arc<dyn StringMap>,
    ctx: &Arc<SpatialContext>,
) -> Result<Arc<dyn SpatialPrefixTree>> {
    // TODO refactor to use Java SPI like how Lucene already does for codecs/postingsFormats, etc
    let cname = match args.get(PREFIX_TREE)? {
        Some(c) => c,
        None => (if ctx.is_geo() { "geohash" } else { "quad" }).to_string(),
    };
    let mut instance: Box<dyn SpatialPrefixTreeFactory> = if "geohash".eq_ignore_ascii_case(&cname)
    {
        Box::new(geohash_prefix_tree::Factory::default())
    } else if "quad".eq_ignore_ascii_case(&cname) {
        Box::new(quad_prefix_tree::Factory::default())
    } else if "packedQuad".eq_ignore_ascii_case(&cname) {
        Box::new(packed_quad_prefix_tree::Factory::default())
    } else if "s2".eq_ignore_ascii_case(&cname) {
        Box::new(s2_prefix_tree::Factory::default())
    } else {
        let ctor = FACTORY_CLASSES
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cname)
            .copied();
        match ctor {
            Some(ctor) => ctor(),
            None => {
                return Err(LuceneError::runtime_exception(LuceneError::unchecked(
                    "java.lang.ClassNotFoundException",
                    cname,
                )))
            }
        }
    };
    instance.init(Arc::clone(args), Arc::clone(ctx))?;
    instance.new_spt()
}
