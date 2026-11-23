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

//! Port of `org.apache.lucene.benchmark.byTask.utils.TestStreamUtils`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene_benchmark::by_task::utils::stream_utils::StreamUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::benchmark_class;
use crate::benchmark_support::search_support::{ftest, set_up};

const TEXT: &str = "Some-Text...";

/// The instance of `TestStreamUtils`: `private Path testDir`.
#[derive(Default)]
pub struct StreamUtilsTest {
    test_dir: PathBuf,
}

/// `TestStreamUtils`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<StreamUtilsTest>(
        "org.apache.lucene.benchmark.byTask.utils.TestStreamUtils",
        vec![
            set_up(setup),
            ftest("testGetInputStreamPlainText", test_get_input_stream_plain_text),
            ftest("testGetInputStreamGzip", test_get_input_stream_gzip),
            ftest("testGetInputStreamBzip2", test_get_input_stream_bzip2),
            ftest("testGetOutputStreamBzip2", test_get_output_stream_bzip2),
            ftest("testGetOutputStreamGzip", test_get_output_stream_gzip),
            ftest("testGetOutputStreamPlain", test_get_output_stream_plain),
        ],
    )
}

fn setup(f: &mut StreamUtilsTest) -> Result<()> {
    f.test_dir = LuceneTestCase::create_temp_dir_prefix("ContentSourceTest").as_std_path().to_path_buf();
    Ok(())
}

fn test_get_input_stream_plain_text(f: &mut StreamUtilsTest) -> Result<()> {
    assert_read_text(&raw_text_file(f, "txt")?)?;
    assert_read_text(&raw_text_file(f, "TXT")?)
}

fn test_get_input_stream_gzip(f: &mut StreamUtilsTest) -> Result<()> {
    for ext in ["gz", "gzip", "GZ", "GZIP"] {
        assert_read_text(&raw_gzip_file(f, ext)?)?;
    }
    Ok(())
}

fn test_get_input_stream_bzip2(f: &mut StreamUtilsTest) -> Result<()> {
    for ext in ["bz2", "bzip", "BZ2", "BZIP"] {
        assert_read_text(&raw_bzip2_file(f, ext)?)?;
    }
    Ok(())
}

fn test_get_output_stream_bzip2(f: &mut StreamUtilsTest) -> Result<()> {
    for ext in ["bz2", "bzip", "BZ2", "BZIP"] {
        assert_read_text(&auto_out_file(f, ext)?)?;
    }
    Ok(())
}

fn test_get_output_stream_gzip(f: &mut StreamUtilsTest) -> Result<()> {
    for ext in ["gz", "gzip", "GZ", "GZIP"] {
        assert_read_text(&auto_out_file(f, ext)?)?;
    }
    Ok(())
}

fn test_get_output_stream_plain(f: &mut StreamUtilsTest) -> Result<()> {
    for ext in ["txt", "text", "TXT", "TEXT"] {
        assert_read_text(&auto_out_file(f, ext)?)?;
    }
    Ok(())
}

/// `TEXT` and `BufferedWriter.newLine()`.
fn text_line() -> String {
    format!("{TEXT}{}", rucene_benchmark::jdk::NEW_LINE)
}

fn raw_text_file(f: &StreamUtilsTest, ext: &str) -> Result<PathBuf> {
    let path = f.test_dir.join(format!("testfile.{ext}"));
    std::fs::write(&path, text_line())?;
    Ok(path)
}

/// `CompressorStreamFactory.GZIP` (commons-compress `GzipCompressorOutputStream`;
/// the port's `flate2` encoder, rmp #611 declared divergence on the bytes).
fn raw_gzip_file(f: &StreamUtilsTest, ext: &str) -> Result<PathBuf> {
    let path = f.test_dir.join(format!("testfile.{ext}"));
    let mut os = flate2::write::GzEncoder::new(std::fs::File::create(&path)?, flate2::Compression::default());
    os.write_all(text_line().as_bytes())?;
    os.finish()?;
    Ok(path)
}

/// `CompressorStreamFactory.BZIP2` (the port's `bzip2` encoder).
fn raw_bzip2_file(f: &StreamUtilsTest, ext: &str) -> Result<PathBuf> {
    let path = f.test_dir.join(format!("testfile.{ext}"));
    let mut os = bzip2::write::BzEncoder::new(std::fs::File::create(&path)?, bzip2::Compression::default());
    os.write_all(text_line().as_bytes())?;
    os.finish()?;
    Ok(path)
}

fn auto_out_file(f: &StreamUtilsTest, ext: &str) -> Result<PathBuf> {
    let path = f.test_dir.join(format!("testfile.{ext}"));
    let mut os = StreamUtils::output_stream(&path)?;
    os.write_all(text_line().as_bytes())?;
    os.close()?;
    Ok(path)
}

fn assert_read_text(f: &Path) -> Result<()> {
    let ir = StreamUtils::input_stream(f)?;
    let mut r = BufReader::new(ir);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let line = line.trim_end_matches(['\r', '\n']);
    crate::benchmark_support::assert_equals_msg(
        &format!("Wrong text found in {}", f.file_name().unwrap().to_string_lossy()),
        TEXT,
        line,
    );
    Ok(())
}
