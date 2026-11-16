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

//! Fixtures the `SimpleText` unit tests share: a directory, a segment
//! info, and hand-written plain-text files with a valid checksum footer.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind};
use rucene::index::segment_info::SegmentInfo;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::Version;

use super::util;

/// A fresh in-memory directory.
pub(crate) fn directory() -> Arc<dyn Directory> {
    Arc::new(ByteBuffersDirectory::new())
}

/// A segment named `name` with `max_doc` documents and an all-zero id.
pub(crate) fn segment_info(dir: &Arc<dyn Directory>, name: &str, max_doc: i32) -> SegmentInfo {
    SegmentInfo::new_without_codec(
        Arc::clone(dir),
        Version::LUCENE_10_5_0,
        None,
        name.to_string(),
        max_doc,
        false,
        false,
        HashMap::new(),
        [0u8; 16],
        HashMap::new(),
        None,
    )
    .expect("a valid segment info")
}

/// Writes `lines`, each followed by a newline, and — when `checksum` — the
/// `SimpleTextUtil.writeChecksum` footer.
pub(crate) fn write_file(dir: &dyn Directory, name: &str, lines: &[&str], checksum: bool) {
    let mut out = dir
        .create_output(name, &*DEFAULT_IO_CONTEXT)
        .expect("create output");
    for line in lines {
        out.write_bytes(line.as_bytes(), 0, line.len()).unwrap();
        util::write_newline(out.as_mut()).unwrap();
    }
    if checksum {
        util::write_checksum(out.as_mut()).unwrap();
    }
    out.close().unwrap();
}

/// The Java class and message of an unchecked exception.
pub(crate) fn unchecked(error: &LuceneError) -> (&'static str, String) {
    match &error.kind {
        LuceneErrorKind::Unchecked { class, message, .. } => {
            (class, message.clone().unwrap_or_default())
        }
        other => panic!("not an unchecked exception: {other:?}"),
    }
}

/// An output whose writes fail with `write failed` and whose `close` fails
/// with `close failed`.
#[derive(Debug)]
pub(crate) struct FailingOutput;

impl rucene::store::DataOutput for FailingOutput {
    fn write_byte(&mut self, _b: u8) -> rucene::error::Result<()> {
        Err(LuceneError::illegal_state("write failed".to_string()))
    }

    fn write_bytes(&mut self, _b: &[u8], _offset: usize, _len: usize) -> rucene::error::Result<()> {
        Err(LuceneError::illegal_state("write failed".to_string()))
    }
}

impl rucene::store::IndexOutput for FailingOutput {
    fn close(&mut self) -> rucene::error::Result<()> {
        Err(LuceneError::illegal_state("close failed".to_string()))
    }

    fn file_pointer(&self) -> i64 {
        0
    }

    fn checksum(&self) -> rucene::error::Result<i64> {
        Ok(0)
    }

    fn resource_description(&self) -> &str {
        "FailingOutput"
    }

    fn name(&self) -> &str {
        "FailingOutput"
    }
}
