//! Port of `com.carrotsearch.randomizedtesting.TempPathResource`
//! (randomizedtesting 2.8.1, `TempPathResource.java`).

use std::path::{Path, PathBuf};

use crate::jdk::io::Closeable;
use crate::randomized_test::RandomizedTest;

/// A temporary path deleted at the end of a lifecycle phase:
/// `TempPathResource implements Closeable`.
#[derive(Debug, Clone)]
pub struct TempPathResource {
    location: PathBuf,
}

impl TempPathResource {
    /// `new TempPathResource(location)`.
    pub fn new(location: impl AsRef<Path>) -> TempPathResource {
        TempPathResource {
            location: location.as_ref().to_path_buf(),
        }
    }
}

impl Closeable for TempPathResource {
    /// `close()`: a directory is removed recursively
    /// (`RandomizedTest.rmDir`), anything else with
    /// `Files.deleteIfExists`.
    fn close(&self) -> std::io::Result<()> {
        if self.location.is_dir() {
            RandomizedTest::rm_dir(&self.location)
        } else {
            match std::fs::remove_file(&self.location) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }
}
