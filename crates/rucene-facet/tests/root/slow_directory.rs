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

//! Port of `org.apache.lucene.facet.SlowDirectory` (test source): a
//! `FilterDirectory` over a `ByteBuffersDirectory` whose inputs and outputs
//! sleep every `IO_SLEEP_THRESHOLD` bytes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::Result;
use rucene::store::{
    BufferedChecksumIndexInput, ByteBuffersDirectory, DataInput, DataOutput, Directory,
    FilterDirectory, FilterIndexInput, FilterIndexOutput, IOContext, IndexInput, IndexOutput, Lock,
};

const IO_SLEEP_THRESHOLD: i32 = 50;

/// The state the directory and its streams share: `sleepMillis` (settable
/// while streams are open, as Java's field) and `random`.
struct Shared {
    sleep_millis: AtomicI32,
    random: Option<Arc<dyn Random>>,
}

impl Shared {
    /// `doSleep(Random random, int length)`.
    fn do_sleep(&self, random: Option<&JdkRandom>, length: usize) {
        let sleep_millis = self.sleep_millis.load(Ordering::SeqCst);
        let mut s_time = if length < 10 {
            sleep_millis
        } else {
            (f64::from(sleep_millis) * (length as f64).ln()) as i32
        };
        if let Some(random) = random {
            s_time = random.next_int_bound(s_time);
        }
        std::thread::sleep(std::time::Duration::from_millis(s_time.max(0) as u64));
    }

    /// `forkRandom()`: make a private random.
    fn fork_random(&self) -> Option<JdkRandom> {
        self.random.as_ref().map(|r| JdkRandom::new(r.next_long()))
    }
}

/// `SlowDirectory`.
pub struct SlowDirectory {
    r#in: FilterDirectory,
    shared: Arc<Shared>,
}

impl SlowDirectory {
    /// `SlowDirectory(int sleepMillis, Random random)`.
    pub fn new(sleep_millis: i32, random: Option<Arc<dyn Random>>) -> Self {
        Self {
            r#in: FilterDirectory::new(Box::new(ByteBuffersDirectory::new())),
            shared: Arc::new(Shared {
                sleep_millis: AtomicI32::new(sleep_millis),
                random,
            }),
        }
    }

    /// `setSleepMillis(int)`.
    pub fn set_sleep_millis(&self, sleep_millis: i32) {
        self.shared
            .sleep_millis
            .store(sleep_millis, Ordering::SeqCst);
    }
}

impl Directory for SlowDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let out = self.r#in.create_output(name, context)?;
        if self.shared.sleep_millis.load(Ordering::SeqCst) != -1 {
            return Ok(Box::new(SlowIndexOutput::new(
                Arc::clone(&self.shared),
                out,
            )));
        }
        Ok(out)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[&str]) -> Result<()> {
        self.r#in.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        // `Directory.openChecksumInput`: through this class's `openInput`.
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*rucene::store::READONCE_IO_CONTEXT)?,
        )))
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let input = self.r#in.open_input(name, context)?;
        if self.shared.sleep_millis.load(Ordering::SeqCst) != -1 {
            return Ok(Box::new(SlowIndexInput::new(
                Arc::clone(&self.shared),
                input,
            )));
        }
        Ok(input)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }

    fn close(&self) -> Result<()> {
        self.r#in.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }

    fn directory_to_string(&self) -> String {
        format!(
            "SlowDirectory({})",
            self.r#in.get_delegate().directory_to_string()
        )
    }

    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.get_delegate())
    }
}

/// `SlowIndexInput`: delays reading bytes by some specified time.
struct SlowIndexInput {
    r#in: FilterIndexInput,
    shared: Arc<Shared>,
    num_read: i32,
    rand: Option<JdkRandom>,
}

impl SlowIndexInput {
    fn new(shared: Arc<Shared>, ii: Box<dyn IndexInput>) -> Self {
        let description = format!("SlowIndexInput({})", ii.to_java_string());
        let rand = shared.fork_random();
        Self {
            r#in: FilterIndexInput::new(description, ii),
            shared,
            num_read: 0,
            rand,
        }
    }
}

impl DataInput for SlowIndexInput {
    fn to_java_string(&self) -> std::borrow::Cow<'_, str> {
        self.r#in.to_java_string()
    }

    fn read_byte(&mut self) -> Result<u8> {
        if self.num_read >= IO_SLEEP_THRESHOLD {
            self.shared.do_sleep(self.rand.as_ref(), 0);
            self.num_read = 0;
        }
        self.num_read += 1;
        self.r#in.read_byte()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        if self.num_read >= IO_SLEEP_THRESHOLD {
            self.shared.do_sleep(self.rand.as_ref(), len);
            self.num_read = 0;
        }
        self.num_read += len as i32;
        self.r#in.read_bytes(b, offset, len)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.r#in.skip_bytes(num_bytes)
    }
}

impl IndexInput for SlowIndexInput {
    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }

    fn file_pointer(&self) -> Result<i64> {
        self.r#in.file_pointer()
    }

    fn length(&self) -> i64 {
        self.r#in.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.r#in.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.r#in.slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.r#in.clone_input()
    }

    fn resource_description(&self) -> &str {
        self.r#in.resource_description()
    }

    fn as_filter_index_input(&self) -> Option<(std::any::TypeId, &dyn IndexInput)> {
        Some((
            std::any::TypeId::of::<SlowIndexInput>(),
            self.r#in.get_delegate(),
        ))
    }
}

/// `SlowIndexOutput`: delays writing bytes by some specified time.
struct SlowIndexOutput {
    out: FilterIndexOutput,
    shared: Arc<Shared>,
    num_wrote: i32,
    rand: Option<JdkRandom>,
}

impl SlowIndexOutput {
    fn new(shared: Arc<Shared>, out: Box<dyn IndexOutput>) -> Self {
        let description = format!("SlowIndexOutput({})", out.to_java_string());
        let name = out.name().to_string();
        let rand = shared.fork_random();
        Self {
            out: FilterIndexOutput::new(description, name, out),
            shared,
            num_wrote: 0,
            rand,
        }
    }
}

impl DataOutput for SlowIndexOutput {
    fn to_java_string(&self) -> String {
        self.out.to_java_string()
    }

    fn write_byte(&mut self, b: u8) -> Result<()> {
        if self.num_wrote >= IO_SLEEP_THRESHOLD {
            self.shared.do_sleep(self.rand.as_ref(), 0);
            self.num_wrote = 0;
        }
        self.num_wrote += 1;
        self.out.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        if self.num_wrote >= IO_SLEEP_THRESHOLD {
            self.shared.do_sleep(self.rand.as_ref(), length);
            self.num_wrote = 0;
        }
        self.num_wrote += length as i32;
        self.out.write_bytes(b, offset, length)
    }
}

impl IndexOutput for SlowIndexOutput {
    fn close(&mut self) -> Result<()> {
        self.out.close()
    }

    fn file_pointer(&self) -> i64 {
        self.out.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.out.checksum()
    }

    fn resource_description(&self) -> &str {
        self.out.resource_description()
    }

    fn name(&self) -> &str {
        self.out.name()
    }

    fn as_filter_index_output(&self) -> Option<&dyn IndexOutput> {
        Some(self.out.get_delegate())
    }
}
