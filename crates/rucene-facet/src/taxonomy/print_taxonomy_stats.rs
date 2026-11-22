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

//! Port of `PrintTaxonomyStats.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::store::{Directory, FSDirectory};
use rucene::util::print_stream::PrintStream;

use crate::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use crate::taxonomy::taxonomy_reader::{TaxonomyReader, INVALID_ORDINAL, ROOT_ORDINAL};

/// Prints how many ords are under each dimension.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.PrintTaxonomyStats`.
#[derive(Debug, Default)]
pub struct PrintTaxonomyStats;

/// `System.out`.
struct StdOut;

impl PrintStream for StdOut {
    fn print(&self, s: &str) {
        print!("{s}");
    }
}

impl PrintTaxonomyStats {
    /// Sole constructor.
    ///
    /// Equivalent to `PrintTaxonomyStats()`.
    pub fn new() -> Self {
        Self
    }

    /// Command-line tool.
    ///
    /// Equivalent to `main(String[])`; Java's `System.exit(1)` on a usage
    /// error is the process exit here too.
    pub fn main(args: &[String]) -> Result<()> {
        let mut print_tree = false;
        let mut path: Option<&str> = None;
        for arg in args {
            if arg == "-printTree" {
                print_tree = true;
            } else {
                path = Some(arg);
            }
        }
        if args.len() != if print_tree { 2 } else { 1 } {
            println!(
                "\nUsage: java -classpath ... org.apache.lucene.facet.util.PrintTaxonomyStats [-printTree] /path/to/taxononmy/index\n"
            );
            std::process::exit(1);
        }
        let path = path.ok_or_else(LuceneError::null_pointer)?;
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(std::path::Path::new(path))?);
        let r = DirectoryTaxonomyReader::open(Arc::clone(&dir))?;
        Self::print_stats(&r, &StdOut, print_tree)?;
        r.close()?;
        dir.close()
    }

    /// Recursively prints stats for all ordinals.
    ///
    /// Equivalent to `printStats(TaxonomyReader, PrintStream, boolean)`.
    pub fn print_stats(r: &dyn TaxonomyReader, out: &dyn PrintStream, print_tree: bool) -> Result<()> {
        out.println(&format!("{} total categories.", r.get_size()?));
        let mut it = r.get_children(ROOT_ORDINAL)?;
        loop {
            let child = it.next();
            if child == INVALID_ORDINAL {
                break;
            }
            let mut chilren_it = r.get_children(child)?;
            let mut num_immediate_children = 0;
            while chilren_it.next() != INVALID_ORDINAL {
                num_immediate_children += 1;
            }
            let cp = r.get_path(child)?.ok_or_else(LuceneError::null_pointer)?;
            out.println(&format!(
                "/{}: {} immediate children; {} total categories",
                cp.components[0],
                num_immediate_children,
                1 + Self::count_all_children(r, child)?
            ));
            if print_tree {
                Self::print_all_children(out, r, child, "  ", 1)?;
            }
        }
        Ok(())
    }

    /// Equivalent to `countAllChildren(TaxonomyReader, int)`.
    fn count_all_children(r: &dyn TaxonomyReader, ord: i32) -> Result<i32> {
        let mut count = 0;
        let mut it = r.get_children(ord)?;
        loop {
            let child = it.next();
            if child == INVALID_ORDINAL {
                break;
            }
            count += 1 + Self::count_all_children(r, child)?;
        }
        Ok(count)
    }

    /// Equivalent to `printAllChildren(PrintStream, TaxonomyReader, int,
    /// String, int)`.
    fn print_all_children(
        out: &dyn PrintStream,
        r: &dyn TaxonomyReader,
        ord: i32,
        indent: &str,
        depth: usize,
    ) -> Result<()> {
        let mut it = r.get_children(ord)?;
        loop {
            let child = it.next();
            if child == INVALID_ORDINAL {
                break;
            }
            let path = r.get_path(child)?.ok_or_else(LuceneError::null_pointer)?;
            out.println(&format!("{indent}/{}", path.components[depth]));
            Self::print_all_children(out, r, child, &format!("{indent}  "), depth + 1)?;
        }
        Ok(())
    }
}
