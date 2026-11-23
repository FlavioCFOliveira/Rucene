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

//! The types declared in `ExtractWikipedia.java`, of the Java package
//! `org.apache.lucene.benchmark.utils`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::IOUtils;

use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_maker::{DocMaker, DocMakerClass};
use crate::by_task::feeds::enwiki_content_source::EnwikiContentSource;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::utils::config::Config;
use crate::jdk::properties::Properties;
use crate::jdk;

/// `BASE`.
pub(crate) const BASE: i32 = 10;

/// Equivalent to `org.apache.lucene.benchmark.utils.ExtractWikipedia`:
/// extract the downloaded Wikipedia dump into separate files for indexing.
pub struct ExtractWikipedia {
    output_dir: PathBuf,
    count: i32,
    /// `docMaker`.
    pub doc_maker: Arc<dyn DocMakerClass>,
}

impl ExtractWikipedia {
    /// `ExtractWikipedia(DocMaker docMaker, Path outputDir)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of deleting the output directory.
    pub fn new(doc_maker: Arc<dyn DocMakerClass>, output_dir: &Path) -> Result<Self> {
        println!("Deleting all files in {}", output_dir.display());
        IOUtils::rm([output_dir])?;
        Ok(ExtractWikipedia { output_dir: output_dir.to_path_buf(), count: 0, doc_maker })
    }

    /// `directory(int count, Path directory)`.
    pub fn directory(&self, count: i32, directory: Option<&Path>) -> PathBuf {
        let directory = directory.map(Path::to_path_buf).unwrap_or_else(|| self.output_dir.clone());
        let mut base = BASE;
        while base <= count {
            base = base.wrapping_mul(BASE);
        }
        if count < BASE {
            return directory;
        }
        let directory = directory.join((base / BASE).to_string());
        let directory = directory.join((count / (base / BASE)).to_string());
        self.directory(count % (base / BASE), Some(&directory))
    }

    /// `create(String id, String title, String time, String body)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of writing the file.
    pub fn create(&mut self, id: Option<&str>, title: Option<&str>, time: Option<&str>, body: Option<&str>) -> Result<()> {
        let d = self.directory(self.count, None);
        self.count += 1;
        std::fs::create_dir_all(&d)?;
        let f = d.join(format!("{}.txt", id.unwrap_or("null")));

        let mut contents = String::new();
        contents.push_str(time.unwrap_or("null"));
        contents.push_str("\n\n");
        contents.push_str(title.unwrap_or("null"));
        contents.push_str("\n\n");
        contents.push_str(body.unwrap_or("null"));
        contents.push('\n');

        let mut writer = std::io::BufWriter::new(std::fs::File::create(&f)?);
        writer.write_all(contents.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    /// `extract()`.
    ///
    /// # Errors
    ///
    /// The exception of the doc maker or of writing.
    pub fn extract(&mut self) -> Result<()> {
        println!("Starting Extraction");
        let start = jdk::current_time_millis();
        loop {
            match self.doc_maker.make_document() {
                Ok(doc) => {
                    self.create(
                        doc.get(DocMaker::ID_FIELD).as_deref(),
                        doc.get(DocMaker::TITLE_FIELD).as_deref(),
                        doc.get(DocMaker::DATE_FIELD).as_deref(),
                        doc.get(DocMaker::BODY_FIELD).as_deref(),
                    )?;
                }
                Err(e) if NoMoreDataException::is(&e) => {
                    // continue
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        let finish = jdk::current_time_millis();
        println!("Extraction took {} ms", finish - start);
        Ok(())
    }

    /// `main(String[] args)`.
    ///
    /// # Errors
    ///
    /// The exception of reading the dump or writing the files.
    pub fn main(args: &[String]) -> Result<()> {
        let mut wikipedia: Option<PathBuf> = None;
        let mut output_dir = PathBuf::from("enwiki");
        let mut keep_image_only_docs = true;
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            if arg == "--input" || arg == "-i" {
                wikipedia = Some(PathBuf::from(index_arg(args, i + 1)?));
                i += 1;
            } else if arg == "--output" || arg == "-o" {
                output_dir = PathBuf::from(index_arg(args, i + 1)?);
                i += 1;
            } else if arg == "--discardImageOnlyDocs" || arg == "-d" {
                keep_image_only_docs = false;
            }
            i += 1;
        }

        let wikipedia = wikipedia.ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let mut properties = Properties::new();
        let absolute = std::path::absolute(&wikipedia).unwrap_or_else(|_| wikipedia.clone());
        properties.set_property("docs.file", absolute.to_string_lossy().into_owned());
        properties.set_property("content.source.forever", "false");
        properties.set_property("keep.image.only.docs", keep_image_only_docs.to_string());
        let config = Arc::new(Config::from_properties(properties));

        let source: Arc<dyn ContentSource> = Arc::new(EnwikiContentSource::new());
        source.set_config(Arc::clone(&config))?;

        let doc_maker = Arc::new(DocMaker::new());
        doc_maker.set_config(Arc::clone(&config), source)?;
        DocMakerClass::reset_inputs(&*doc_maker)?;
        if wikipedia.exists() {
            println!("Extracting Wikipedia to: {} using EnwikiContentSource", output_dir.display());
            std::fs::create_dir_all(&output_dir)?;
            let mut extractor = ExtractWikipedia::new(doc_maker, &output_dir)?;
            extractor.extract()?;
        } else {
            Self::print_usage();
        }
        Ok(())
    }

    /// `printUsage()`.
    fn print_usage() {
        eprintln!(
            "Usage: java -cp <...> org.apache.lucene.benchmark.utils.ExtractWikipedia --input|-i <Path to Wikipedia XML file> [--output|-o <Output Path>] [--discardImageOnlyDocs|-d]"
        );
        eprintln!("--discardImageOnlyDocs tells the extractor to skip Wiki docs that contain only images");
    }
}

/// `args[i]`, Java's `ArrayIndexOutOfBoundsException` past the end.
fn index_arg(args: &[String], i: usize) -> Result<&str> {
    args.get(i).map(String::as_str).ok_or_else(|| {
        rucene::error::LuceneError::unchecked(
            "java.lang.ArrayIndexOutOfBoundsException",
            format!("Index {i} out of bounds for length {}", args.len()),
        )
    })
}
