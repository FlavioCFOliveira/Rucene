//! The error type of the example.

use std::fmt;

use rucene::LuceneError;

/// Everything that can make a command fail.
///
/// The variants separate a mistake on the command line (exit code 2) from a
/// failure while doing the work (exit code 1), the convention of most Unix
/// tools.
#[derive(Debug)]
pub enum CliError {
    /// The command line is malformed: unknown command or flag, missing or
    /// conflicting argument.
    Usage(String),
    /// An input value is invalid: a CSV that does not parse, a value that does
    /// not fit its field, a date that is not ISO 8601.
    Input(String),
    /// The `rucene` library reported an error.
    Lucene(LuceneError),
    /// An operating-system I/O error outside the library (reading a CSV file,
    /// writing the output).
    Io(std::io::Error),
}

impl CliError {
    /// The process exit code this error maps to: `2` for usage errors, `1` for
    /// everything else.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Input(_) | Self::Lucene(_) | Self::Io(_) => 1,
        }
    }

    /// Prefixes the message of an [`Input`](Self::Input) error with
    /// `context`, leaving the other variants unchanged.
    #[must_use]
    pub fn context(self, context: &str) -> Self {
        match self {
            Self::Input(message) => Self::Input(format!("{context}: {message}")),
            Self::Lucene(error) => Self::Input(format!("{context}: {error}")),
            other => other,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message} (run `rucene-cli help` for usage)"),
            Self::Input(message) => f.write_str(message),
            Self::Lucene(error) => write!(f, "rucene: {error}"),
            Self::Io(error) => write!(f, "I/O: {error}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lucene(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Usage(_) | Self::Input(_) => None,
        }
    }
}

impl From<LuceneError> for CliError {
    fn from(error: LuceneError) -> Self {
        Self::Lucene(error)
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// The result type of the example.
pub type Result<T> = std::result::Result<T, CliError>;
