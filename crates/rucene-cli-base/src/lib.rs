//! A command-line example of the `rucene` crate over the Portuguese public
//! procurement data of Portal BASE (IMPIC, dados.gov.pt): one Lucene index
//! per entity, each managed and searched individually, with text search
//! insensitive to case, accents and diacritics.
//!
//! The binary `rucene-base` is a thin wrapper over [`cli::run`]; the logic
//! lives in this library so the integration tests can drive it directly.
//! Everything goes through the public `rucene` API (`IndexWriter`,
//! `DirectoryReader`, `IndexSearcher`, the Lucene 10 field classes, points,
//! doc values, `Sort`, `QueryBuilder`) and the ported analysis components of
//! `rucene-analysis-common` (`PortugueseLightStemFilter`, `PatternReplaceFilter`,
//! `ASCIIFoldingFilter`, the Portuguese stop set).
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`analysis`] | The pt-PT analyzer and the reasons for its filter order |
//! | [`schema`] | Field kinds, record to document, stored read-back |
//! | [`entities`] | The ten index schemas, each with its source mapping |
//! | [`input`] | The JSON Lines bulk format |
//! | [`json`] | JSON parsing and string writing |
//! | [`date`] | `YYYY-MM-DD` / `DD/MM/YYYY` to and from epoch milliseconds |
//! | [`index_ops`] | Index layout under the root; `create`, `add`, `update`, `delete`, `get`, `stats` |
//! | [`search`] | Query and sort construction; `search`, `count` |
//! | [`cli`] | Argument parsing and dispatch |

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod analysis;
pub mod cli;
pub mod date;
pub mod entities;
pub mod error;
pub mod index_ops;
pub mod input;
pub mod json;
pub mod schema;
pub mod search;

pub use error::{CliError, Result};
