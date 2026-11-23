// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Assert`.

use rucene::error::{LuceneError, Result};

/// Assertion helpers that throw `IllegalStateException`.
///
/// Equivalent to `com.ibm.icu.impl.Assert`.
pub struct Assert;

impl Assert {
    /// `fail(Exception e)`: fails with `e.toString()`.
    ///
    /// # Errors
    ///
    /// Always: `IllegalStateException("failure '<e>'")`.
    pub fn fail_exception(e: &LuceneError) -> Result<()> {
        Self::fail(&e.to_string())
    }

    /// `fail(String msg)`.
    ///
    /// # Errors
    ///
    /// Always: `IllegalStateException("failure '<msg>'")`.
    pub fn fail(msg: &str) -> Result<()> {
        Err(LuceneError::illegal_state(format!("failure '{msg}'")))
    }

    /// `assrt(boolean val)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("assert failed")` when `val` is false.
    pub fn assrt(val: bool) -> Result<()> {
        if !val {
            return Err(LuceneError::illegal_state("assert failed"));
        }
        Ok(())
    }

    /// `assrt(String msg, boolean val)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("assert '<msg>' failed")` when `val` is false.
    pub fn assrt_msg(msg: &str, val: bool) -> Result<()> {
        if !val {
            return Err(LuceneError::illegal_state(format!("assert '{msg}' failed")));
        }
        Ok(())
    }
}
