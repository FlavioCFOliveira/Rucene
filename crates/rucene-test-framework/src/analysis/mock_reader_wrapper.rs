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

//! Port of `org.apache.lucene.tests.analysis.MockReaderWrapper`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::analysis::CharReader;
use rucene::error::{LuceneError, Result};

use crate::util::TestUtil;

const FAKE_EXCEPTION: &str = "fake exception now!";

/// Wraps a Reader, and can throw random or fixed exceptions, and spoon feed
/// read chars.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockReaderWrapper`. The
/// counts are UTF-16 code units, as Java's `char`s: a supplementary
/// character read whole adds two to the characters read so far.
pub struct MockReaderWrapper {
    input: Box<dyn CharReader>,
    random: Arc<dyn Random>,
    exc_at_char: i32,
    read_so_far: i32,
    throw_exc_next: bool,
}

impl std::fmt::Debug for MockReaderWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockReaderWrapper")
            .field("input", &self.input)
            .field("exc_at_char", &self.exc_at_char)
            .field("read_so_far", &self.read_so_far)
            .finish_non_exhaustive()
    }
}

impl MockReaderWrapper {
    /// `MockReaderWrapper(Random random, Reader in)`.
    pub fn new(random: Arc<dyn Random>, input: Box<dyn CharReader>) -> MockReaderWrapper {
        MockReaderWrapper {
            input,
            random,
            exc_at_char: -1,
            read_so_far: 0,
            throw_exc_next: false,
        }
    }

    /// Throw an exception after reading this many chars.
    pub fn throw_exc_after_char(&mut self, char_upto: i32) {
        self.exc_at_char = char_upto;
        // You should only call this on init!:
        java_assert_unchecked!(self.read_so_far == 0);
    }

    /// `throwExcNext()`.
    pub fn throw_exc_next(&mut self) {
        self.throw_exc_next = true;
    }

    /// `isMyEvilException(Throwable)`: a `RuntimeException` with the
    /// message this wrapper throws.
    pub fn is_my_evil_exception(t: &LuceneError) -> bool {
        matches!(t, LuceneError::Unchecked { class, message: Some(m), .. }
            if *class == "java.lang.RuntimeException" && m == FAKE_EXCEPTION)
    }
}

impl CharReader for MockReaderWrapper {
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        if self.throw_exc_next || (self.exc_at_char != -1 && self.read_so_far >= self.exc_at_char) {
            return Err(LuceneError::unchecked(
                "java.lang.RuntimeException",
                FAKE_EXCEPTION,
            ));
        }
        let len = buf.len() as i32;
        let real_len = if len == 1 {
            1
        } else {
            // Spoon-feed: intentionally maybe return less than
            // the consumer asked for
            TestUtil::next_int(&*self.random, 1, len)
        };
        let read;
        if self.exc_at_char != -1 {
            let left = self.exc_at_char - self.read_so_far;
            java_assert_unchecked!(left != 0);
            read = self.input.read(&mut buf[..real_len.min(left) as usize])?;
            java_assert_unchecked!(read != 0);
            self.read_so_far += read as i32;
        } else {
            read = self.input.read(&mut buf[..real_len as usize])?;
        }
        Ok(read)
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }
}
