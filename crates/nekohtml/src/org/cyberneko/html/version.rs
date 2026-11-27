/* (C) Copyright 2002-2012, Andy Clark, Marc Guillemot. All rights reserved. */

//! Port of `Version.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

/// This class holds version information for the NekoHTML parser.
///
/// Equivalent to `org.cyberneko.html.Version`.
#[derive(Debug, Default)]
pub struct Version;

impl Version {
    /// Returns the version string.
    ///
    /// Equivalent to `getVersion()`.
    pub fn get_version() -> &'static str {
        "NekoHTML 1.9.17"
    }

    /// Prints the version string to standard output.
    ///
    /// Equivalent to `main(String[])`.
    pub fn main() {
        println!("{}", Self::get_version());
    }
}
