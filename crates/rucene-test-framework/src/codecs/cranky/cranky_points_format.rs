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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyPointsFormat`.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{
    MergeState, PointsFormat, PointsReader, PointsWriter, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::{FieldInfo, IntersectVisitor, PointTree, PointValues};

use super::maybe_fail;
use crate::codecs::{CodecFormat, PointsKind};

/// Java's message of every cranky points failure.
const FAKE: &str = "Fake IOException";

/// Port of `CrankyPointsFormat`: the writer's and the reader's calls, and the
/// point values' and trees' reads, fail one time in 100.
pub struct CrankyPointsFormat {
    delegate: CodecFormat<PointsKind>,
    random: Arc<dyn Random>,
}

impl CrankyPointsFormat {
    pub(crate) fn new(delegate: CodecFormat<PointsKind>, random: Arc<dyn Random>) -> Self {
        CrankyPointsFormat { delegate, random }
    }
}

impl fmt::Debug for CrankyPointsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CrankyPointsFormat")
    }
}

impl PointsFormat for CrankyPointsFormat {
    /// Java's `PointsFormat` has no name; the delegate's.
    fn name(&self) -> &str {
        self.delegate.get().name()
    }

    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(CrankyPointsWriter {
            delegate: self.delegate.get().fields_writer(state)?,
            random: Arc::clone(&self.random),
        }))
    }

    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(CrankyPointsReader {
            delegate: Arc::new(Mutex::new(self.delegate.get().fields_reader(state)?)),
            random: Arc::clone(&self.random),
        }))
    }
}

/// Port of `CrankyPointsFormat.CrankyPointsWriter`.
struct CrankyPointsWriter {
    delegate: Box<dyn PointsWriter>,
    random: Arc<dyn Random>,
}

impl fmt::Debug for CrankyPointsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrankyPointsWriter({:?})", self.delegate)
    }
}

impl PointsWriter for CrankyPointsWriter {
    fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.write_field(field_info, values)
    }

    fn finish(&mut self) -> Result<()> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.finish()?;
        maybe_fail(&*self.random, 100, FAKE)
    }

    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.merge(merge_state)?;
        maybe_fail(&*self.random, 100, FAKE)
    }

    fn close(&mut self) -> Result<()> {
        self.delegate.close()?;
        maybe_fail(&*self.random, 100, FAKE)
    }
}

/// Port of `CrankyPointsFormat.CrankyPointsReader`.
///
/// Java's class does not override `getMergeInstance()`, so
/// `PointsReader`'s default returns the reader itself. A Rust merge instance
/// is a new box, so the reader keeps its delegate shared: the merge instance
/// is another handle on this reader (same delegate, same `random`), not a
/// cranky reader over the delegate's merge instance.
struct CrankyPointsReader {
    delegate: Arc<Mutex<Box<dyn PointsReader>>>,
    random: Arc<dyn Random>,
}

impl CrankyPointsReader {
    fn delegate(&self) -> MutexGuard<'_, Box<dyn PointsReader>> {
        self.delegate.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl fmt::Debug for CrankyPointsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrankyPointsReader({:?})", *self.delegate())
    }
}

impl PointsReader for CrankyPointsReader {
    fn check_integrity(&self) -> Result<()> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate().check_integrity()?;
        maybe_fail(&*self.random, 100, FAKE)
    }

    fn get_values(&self, field_name: &str) -> Result<Option<Box<dyn PointValues>>> {
        let Some(delegate) = self.delegate().get_values(field_name)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(CrankyPointValues {
            delegate,
            random: Arc::clone(&self.random),
        })))
    }

    /// `PointsReader.getMergeInstance()`'s default: `this`.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(CrankyPointsReader {
            delegate: Arc::clone(&self.delegate),
            random: Arc::clone(&self.random),
        }))
    }

    fn close(&mut self) -> Result<()> {
        self.delegate().close()?;
        maybe_fail(&*self.random, 100, FAKE)
    }
}

/// The anonymous `PointValues` of `CrankyPointsReader.getValues`.
struct CrankyPointValues {
    delegate: Box<dyn PointValues>,
    random: Arc<dyn Random>,
}

impl PointValues for CrankyPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(CrankyPointTree {
            point_tree: self.delegate.point_tree()?,
            random: Arc::clone(&self.random),
        }))
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.min_packed_value()
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.max_packed_value()
    }

    fn num_dimensions(&self) -> Result<i32> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.num_dimensions()
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.num_index_dimensions()
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.delegate.bytes_per_dimension()
    }

    fn size(&self) -> Result<i64> {
        self.delegate.size()
    }

    fn doc_count(&self) -> Result<i32> {
        self.delegate.doc_count()
    }
}

/// The anonymous `PointTree` of the cranky point values' `getPointTree()`.
///
/// Java's `clone()` returns the delegate tree's clone, not a cranky one;
/// reproduced.
struct CrankyPointTree {
    point_tree: Box<dyn PointTree>,
    random: Arc<dyn Random>,
}

impl PointTree for CrankyPointTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        self.point_tree.clone_tree()
    }

    fn move_to_child(&mut self) -> Result<bool> {
        self.point_tree.move_to_child()
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        self.point_tree.move_to_sibling()
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        self.point_tree.move_to_parent()
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        self.point_tree.min_packed_value()
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        self.point_tree.max_packed_value()
    }

    fn size(&self) -> Result<i64> {
        self.point_tree.size()
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.point_tree.visit_doc_ids(visitor)?;
        maybe_fail(&*self.random, 100, FAKE)
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        maybe_fail(&*self.random, 100, FAKE)?;
        self.point_tree.visit_doc_values(visitor)?;
        maybe_fail(&*self.random, 100, FAKE)
    }
}
