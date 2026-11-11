//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/TestJavaLoggingInfoStream.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::info_stream::log;
use rucene::util::{InfoStream, JavaLoggingInfoStream};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test};

/// `TestJavaLoggingInfoStream`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestJavaLoggingInfoStream",
        vec![test("test", test_logging)],
    )
}

fn test_logging() -> Result<()> {
    // `Logger.getGlobal().isLoggable(Level.WARNING)`: the `log` facade's
    // `global` target at `Warn`.
    LuceneTestCase::assume_true(
        "invalid logging configuration for testing",
        log::log_enabled!(target: "global", log::Level::Warn),
    );
    let component = format!("TESTCOMPONENT{}", random().next_int());
    let _logger_name = format!("org.apache.lucene.{component}");
    {
        // try (var stream = new JavaLoggingInfoStream(Level.WARNING))
        let stream = JavaLoggingInfoStream::new(log::Level::Warn);
        Assert::assert_true(stream.is_enabled(&component));
        stream.message(&component, "Test message to be logged.");
        stream.close();
    }
    // NOT EXPRESSIBLE: `LogManager.getLogManager().getLoggerNames()` contains
    // `loggerName` ("logger with correct name not found"). The `log` facade
    // has no registry of named loggers (`JavaLoggingInfoStream` maps the
    // component to a name it formats into the record), so the assertion has
    // no Rust counterpart; see the pass report.
    Ok(())
}
