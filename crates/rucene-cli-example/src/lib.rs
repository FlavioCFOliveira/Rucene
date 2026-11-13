//! A command-line example of the `rucene` crate: index the USGS Earthquake
//! Catalog and search it by text, keyword, numeric and date range, and
//! geographic box or distance, with multi-key sorting.
//!
//! The binary `rucene-cli` is a thin wrapper over [`cli::run`]; the logic
//! lives in this library so the integration tests can drive it directly.
//! Everything goes through the public `rucene` API (`IndexWriter`,
//! `DirectoryReader`, `IndexSearcher`, the Lucene 10 field classes, points,
//! doc values and `Sort`), so the example doubles as a workload for
//! profiling and correctness work.
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`schema`] | The field-by-field schema: the single source of truth |
//! | [`csv`] | RFC 4180 CSV reading |
//! | [`datetime`] | ISO 8601 to and from epoch milliseconds |
//! | [`index_ops`] | `create`, `add`, `update`, `delete` |
//! | [`search`] | Query and sort construction, sorted search |
//! | [`cli`] | Argument parsing and dispatch |

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cli;
pub mod csv;
pub mod datetime;
pub mod error;
pub mod index_ops;
pub mod schema;
pub mod search;

pub use error::{CliError, Result};
