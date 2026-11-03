//! The JDK behaviour that `com.carrotsearch.randomizedtesting` is written
//! against.
//!
//! randomizedtesting 2.8.1 runs on the Java platform: its generators extend
//! `java.util.Random`, its context is keyed by `java.lang.ThreadGroup`, and its
//! helpers read `java.lang.System` properties and parse them with
//! `Integer.parseInt` or `Double.parseDouble`. Rust has none of these, so each
//! module here reproduces the part of the JDK 25 behaviour the port calls,
//! from the JDK's specification (javadoc) and verified against a JDK 25 oracle
//! (`tests/fixtures/oracle/`). No JDK source is copied:
//! the JDK is GPL with the Classpath Exception.
//!
//! **Divergence.** These modules are emulations, not ports of a
//! Lucene or randomizedtesting file: each one states, at its point, what the
//! JDK does and where the emulation differs.

pub mod exception;
pub mod format;
pub mod io;
pub mod lang;
pub mod locale;
pub mod logging;
pub mod math;
pub mod random;
pub mod reflect;
pub mod runtime;
pub mod strict_math;
pub mod system;
pub mod thread;
pub mod util;
