//! Port of `org.apache.lucene.internal.tests.TestSecrets`.

#![deny(unsafe_code)]

use std::panic::Location;
use std::sync::{Arc, LazyLock, OnceLock};

use crate::error::{LuceneError, Result};
use crate::internal::tests_hooks::{
    ConcurrentMergeSchedulerAccess, FilterIndexInputAccess, IndexPackageAccess, IndexWriterAccess,
    SegmentReaderAccess,
};

/// A set of static methods returning accessors for internal functionality.
///
/// Equivalent to `org.apache.lucene.internal.tests.TestSecrets`.
///
/// In Lucene the getters may only be called by the test-framework module, and
/// each setter is called exactly once, from the static initializer of the class
/// whose internals it exposes.
///
/// # Divergences from Lucene 10.5.0
///
/// * **The caller is identified by source file, not by class.** Java's
///   `ensureCallerForGetter` and `ensureCallerForSetter` run a `StackWalker`
///   over `s.skip(2).limit(1)` and compare the calling *class name* against
///   `org.apache.lucene.tests.*` (getters) or against the owning class
///   (setters). Rust exposes no runtime class of a stack frame, so the port
///   uses the one caller identity stable Rust does provide: `#[track_caller]`
///   on the getters and setters makes `std::panic::Location::caller()` inside
///   the two checks resolve to the call site of the getter or setter, which is
///   the very frame Java's `skip(2).limit(1)` selects. The comparison is then
///   made against a *source file* rather than a class name — `TEST_FRAMEWORK_DIRS`
///   for the getters, `Owner::file` for the setters, both relative to the
///   rucene tree in whatever form the compiler spells its paths
///   (`CRATE_ROOT_PREFIX`), so the checks also hold when rucene and the test
///   framework are dependencies of another workspace. The *set-once* half of
///   `ensureCallerForSetter` is reproduced exactly.
/// * **Registration is run on first use, not by the class loader.** Java calls
///   `MethodHandles.Lookup.ensureInitialized(Class)` so that the owning class's
///   static initializer runs and registers its accessor on first use. Rust has
///   no per-type static initializer, so each owner exposes the body of its
///   `static {}` block as a function guarded by a [`std::sync::Once`], and
///   `TestSecrets::ensure_initialized` runs it: the accessors are registered
///   lazily, by their owners, exactly once — the same moment and the same
///   registrar as Java's.
/// * **`Result` instead of exceptions.** A missing accessor is
///   Java's `NullPointerException` from `Objects.requireNonNull`
///   ([`LuceneError::null_pointer`]); a second registration is
///   Java's `IllegalCallerException`.
/// * **`Arc` instead of a bare reference.** The accessors are shared, immutable
///   singletons; `Arc` is how this crate shares them across threads.
#[derive(Debug, Clone, Copy)]
pub struct TestSecrets;

/// Registry slot for [`IndexPackageAccess`].
static INDEX_PACKAGE_ACCESS: OnceLock<Arc<dyn IndexPackageAccess>> = OnceLock::new();
/// Registry slot for [`ConcurrentMergeSchedulerAccess`].
static CMS_ACCESS: OnceLock<Arc<dyn ConcurrentMergeSchedulerAccess>> = OnceLock::new();
/// Registry slot for [`SegmentReaderAccess`].
static SEGMENT_READER_ACCESS: OnceLock<Arc<dyn SegmentReaderAccess>> = OnceLock::new();
/// Registry slot for [`IndexWriterAccess`].
static INDEX_WRITER_ACCESS: OnceLock<Arc<dyn IndexWriterAccess>> = OnceLock::new();
/// Registry slot for [`FilterIndexInputAccess`].
static FILTER_INDEX_INPUT_ACCESS: OnceLock<Arc<dyn FilterIndexInputAccess>> = OnceLock::new();

/// Builds the error Java raises from `Objects.requireNonNull` when an accessor
/// was never registered: a `NullPointerException` without a message
/// (`TestSecrets.java:68`), so `accessor` is not part of it.
fn missing(_accessor: &str) -> LuceneError {
    LuceneError::null_pointer()
}

/// The `IllegalCallerException` Java's `ensureCallerForSetter` throws.
fn already_set(owner: &str) -> LuceneError {
    LuceneError::from(crate::error::LuceneErrorKind::Unchecked {
        class: "java.lang.IllegalCallerException",
        message: Some(format!("The accessor can only be set once by {owner}.")),
        cause: None,
    })
}

/// Stands in for the `Class<?>` Lucene hands to `ensureCallerForSetter` and to
/// `ensureInitialized`.
///
/// Rust has no runtime class object, so the owning type is carried as the pair
/// the two checks actually need: the name Lucene prints in its message, and the
/// source file its registration is allowed to come from.
#[derive(Clone, Copy, Debug)]
struct Owner {
    /// The owning type's name, as Java's `Class.getName()` supplies it to the
    /// `IllegalCallerException` message.
    name: &'static str,
    /// The source file the registration must be made from, standing in for the
    /// class name Java's `StackWalker` compares against.
    file: &'static str,
    /// The owner's static initializer: the body of its `static {}` block,
    /// which registers its accessors and runs at most once.
    init: fn(),
}

/// `IndexWriter`, which installs both [`IndexWriterAccess`] and
/// [`IndexPackageAccess`].
const INDEX_WRITER: Owner = Owner {
    name: "IndexWriter",
    file: "src/index/index_writer.rs",
    init: crate::index::index_writer::register_test_secrets,
};

/// `ConcurrentMergeScheduler`, which installs
/// [`ConcurrentMergeSchedulerAccess`].
const CONCURRENT_MERGE_SCHEDULER: Owner = Owner {
    name: "ConcurrentMergeScheduler",
    file: "src/index/merge_scheduler.rs",
    init: crate::index::merge_scheduler::register_test_secrets,
};

/// `SegmentReader`, which installs [`SegmentReaderAccess`].
const SEGMENT_READER: Owner = Owner {
    name: "SegmentReader",
    file: "src/index/segment_reader.rs",
    init: crate::index::segment_reader::register_test_secrets,
};

/// `FilterIndexInput`, which installs [`FilterIndexInputAccess`].
const FILTER_INDEX_INPUT: Owner = Owner {
    name: "FilterIndexInput",
    file: "src/store.rs",
    init: crate::store::register_test_secrets,
};

/// The source paths a getter's caller may live in, standing in for the
/// callers `ensureCallerForGetter` accepts (`TestSecrets.java:152-169`): the
/// `org.apache.lucene.tests.` package — the test framework crate's sources,
/// `crates/rucene-test-framework/src/`, the port of `lucene/test-framework`
/// (for example `MockIndexInputWrapper`'s static initializer, which calls
/// `getFilterInputIndexAccess()`) — and the one named class
/// `org.apache.lucene.index.TestClassloadingDeadlock`; plus this crate's own
/// `tests/`. The ported core tests under
/// `crates/rucene-test-framework/tests/` are not test-framework classes, so
/// `TestTestSecrets.testCallerOfGetter` is refused as Java refuses it.
const TEST_FRAMEWORK_DIRS: &[&str] = &[
    "tests/",
    "crates/rucene-test-framework/src/",
    "crates/rucene-test-framework/tests/core_index/test_classloading_deadlock.rs",
];

/// Normalises the path `Location::caller()` reports so that a host which spells
/// it with backslashes compares equal to the forward-slash paths recorded above.
fn caller_file(location: &Location<'static>) -> String {
    location.file().replace('\\', "/")
}

/// This file's path relative to the rucene crate root, forward-slashed.
const THIS_FILE_IN_CRATE: &str = "src/internal/tests_hooks/test_secrets.rs";

/// The prefix the compiler puts before every path of the rucene source tree.
///
/// [`Location::caller`] and [`file!`] report paths in the same form: relative
/// to the workspace root when rucene is a workspace member (`""`, since rucene
/// is the root), and absolute when rucene and the test framework are built as
/// dependencies of another workspace (`/…/rucene/`). The paths of
/// [`TEST_FRAMEWORK_DIRS`] and of every [`Owner::file`] are relative to that
/// root, so a caller matches exactly when its path is this prefix followed by
/// one of them.
static CRATE_ROOT_PREFIX: LazyLock<String> = LazyLock::new(|| {
    let this_file = file!().replace('\\', "/");
    match this_file.strip_suffix(THIS_FILE_IN_CRATE) {
        Some(prefix) => prefix.to_owned(),
        // Unreachable while this file stays at `THIS_FILE_IN_CRATE` (a unit
        // test pins it); every caller is then refused, as Java would refuse an
        // unknown class.
        None => this_file,
    }
});

/// Decides whether `file`, a caller path normalised by [`caller_file`], lies
/// under one of [`TEST_FRAMEWORK_DIRS`] of the rucene tree whose compiler
/// prefix is `crate_root_prefix`.
fn is_test_framework_file(file: &str, crate_root_prefix: &str) -> bool {
    file.strip_prefix(crate_root_prefix).is_some_and(|in_tree| {
        TEST_FRAMEWORK_DIRS
            .iter()
            .any(|dir| in_tree.starts_with(dir))
    })
}

/// Decides whether `file`, a caller path normalised by [`caller_file`], is
/// exactly `owner_file` of the rucene tree whose compiler prefix is
/// `crate_root_prefix`.
fn is_owner_file(file: &str, owner_file: &str, crate_root_prefix: &str) -> bool {
    file.strip_prefix(crate_root_prefix) == Some(owner_file)
}

impl TestSecrets {
    /// Forces the type that owns an accessor to register it, the way Java's
    /// `MethodHandles.Lookup.ensureInitialized(Class)` forces a class's static
    /// initializer to run.
    ///
    /// Equivalent to `TestSecrets.ensureInitialized(Class)`
    /// (`TestSecrets.java:52-58`): the owner's static initializer runs — at most
    /// once, however many threads ask — and calls the matching setter. Java
    /// turns the `IllegalAccessException` of a class it may not initialize
    /// into an `AssertionError`; every owner here is the crate's own, so there
    /// is no such case.
    ///
    /// # Errors
    ///
    /// Never fails; the `Result` keeps the getters' shape.
    fn ensure_initialized(owner: Owner) -> Result<()> {
        (owner.init)();
        Ok(())
    }

    /// Checks that a setter was called from the type whose internals it
    /// exposes, and that no accessor is registered yet.
    ///
    /// Equivalent to `TestSecrets.ensureCallerForSetter(Class, Object)`.
    /// `already_registered` carries what Java tests as `needsNull != null`: the
    /// accessor currently held by the slot being written.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when the call did not come from `owner`'s
    /// source file or when an accessor is already registered.
    #[track_caller]
    fn ensure_caller_for_setter(owner: Owner, already_registered: bool) -> Result<()> {
        let caller = caller_file(Location::caller());
        let valid_caller = is_owner_file(&caller, owner.file, &CRATE_ROOT_PREFIX);
        if !valid_caller || already_registered {
            return Err(already_set(owner.name));
        }
        Ok(())
    }

    /// Checks that a getter was called from the test framework.
    ///
    /// Equivalent to `TestSecrets.ensureCallerForGetter()`. Java accepts a
    /// caller whose class name starts with `org.apache.lucene.tests.`, plus the
    /// single class `org.apache.lucene.index.TestClassloadingDeadlock`; this
    /// port accepts a call site under one of [`TEST_FRAMEWORK_DIRS`], which
    /// names that class's port too.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when the caller is outside the test
    /// framework.
    #[track_caller]
    fn ensure_caller_for_getter() -> Result<()> {
        let caller = caller_file(Location::caller());
        let valid_caller = is_test_framework_file(&caller, &CRATE_ROOT_PREFIX);
        if !valid_caller {
            return Err(LuceneError::from(
                crate::error::LuceneErrorKind::Unchecked {
                    class: "java.lang.IllegalCallerException",
                    message: Some(
                        "Lucene TestSecrets can only be used by the test-framework.".to_string(),
                    ),
                    cause: None,
                },
            ));
        }
        Ok(())
    }

    /// Returns the accessor to internal secrets for an index reader.
    ///
    /// Equivalent to `TestSecrets.getIndexPackageAccess()`.
    ///
    /// # Errors
    ///
    /// Returns Java's `NullPointerException` ([`LuceneError::null_pointer`])
    /// when no accessor has been registered.
    #[track_caller]
    pub fn get_index_package_access() -> Result<Arc<dyn IndexPackageAccess>> {
        Self::ensure_caller_for_getter()?;
        if INDEX_WRITER_ACCESS.get().is_none() {
            Self::ensure_initialized(INDEX_WRITER)?;
        }
        INDEX_PACKAGE_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("IndexPackageAccess"))
    }

    /// Returns the accessor to internal secrets for a concurrent merge
    /// scheduler.
    ///
    /// Equivalent to `TestSecrets.getConcurrentMergeSchedulerAccess()`.
    ///
    /// # Errors
    ///
    /// Returns Java's `NullPointerException` ([`LuceneError::null_pointer`])
    /// when no accessor has been registered.
    #[track_caller]
    pub fn get_concurrent_merge_scheduler_access() -> Result<Arc<dyn ConcurrentMergeSchedulerAccess>>
    {
        Self::ensure_caller_for_getter()?;
        if CMS_ACCESS.get().is_none() {
            Self::ensure_initialized(CONCURRENT_MERGE_SCHEDULER)?;
        }
        CMS_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("ConcurrentMergeSchedulerAccess"))
    }

    /// Returns the accessor to internal secrets for a segment reader.
    ///
    /// Equivalent to `TestSecrets.getSegmentReaderAccess()`.
    ///
    /// # Errors
    ///
    /// Returns Java's `NullPointerException` ([`LuceneError::null_pointer`])
    /// when no accessor has been registered.
    #[track_caller]
    pub fn get_segment_reader_access() -> Result<Arc<dyn SegmentReaderAccess>> {
        Self::ensure_caller_for_getter()?;
        if SEGMENT_READER_ACCESS.get().is_none() {
            Self::ensure_initialized(SEGMENT_READER)?;
        }
        SEGMENT_READER_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("SegmentReaderAccess"))
    }

    /// Returns the accessor to internal secrets for an index writer.
    ///
    /// Equivalent to `TestSecrets.getIndexWriterAccess()`.
    ///
    /// # Errors
    ///
    /// Returns Java's `NullPointerException` ([`LuceneError::null_pointer`])
    /// when no accessor has been registered.
    #[track_caller]
    pub fn get_index_writer_access() -> Result<Arc<dyn IndexWriterAccess>> {
        Self::ensure_caller_for_getter()?;
        if INDEX_WRITER_ACCESS.get().is_none() {
            Self::ensure_initialized(INDEX_WRITER)?;
        }
        INDEX_WRITER_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("IndexWriterAccess"))
    }

    /// Returns the accessor to internal secrets for a filtering index input.
    ///
    /// Equivalent to `TestSecrets.getFilterInputIndexAccess()`, keeping
    /// Lucene's own word order.
    ///
    /// # Errors
    ///
    /// Returns Java's `NullPointerException` ([`LuceneError::null_pointer`])
    /// when no accessor has been registered.
    #[track_caller]
    pub fn get_filter_input_index_access() -> Result<Arc<dyn FilterIndexInputAccess>> {
        Self::ensure_caller_for_getter()?;
        if FILTER_INDEX_INPUT_ACCESS.get().is_none() {
            Self::ensure_initialized(FILTER_INDEX_INPUT)?;
        }
        FILTER_INDEX_INPUT_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("FilterIndexInputAccess"))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to `TestSecrets.setIndexWriterAccess(IndexWriterAccess)`.
    /// A Java `null` argument is `None`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when an accessor is already
    /// registered; Java allows exactly one registration too.
    #[track_caller]
    pub fn set_index_writer_access(access: Option<Arc<dyn IndexWriterAccess>>) -> Result<()> {
        Self::ensure_caller_for_setter(INDEX_WRITER, INDEX_WRITER_ACCESS.get().is_some())?;
        // A Java `null` (`None`) leaves the field `null`, as it was.
        let Some(access) = access else {
            return Ok(());
        };
        INDEX_WRITER_ACCESS
            .set(access)
            .map_err(|_| already_set(INDEX_WRITER.name))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to `TestSecrets.setIndexPackageAccess(IndexPackageAccess)`.
    /// A Java `null` argument is `None`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when an accessor is already
    /// registered.
    #[track_caller]
    pub fn set_index_package_access(access: Option<Arc<dyn IndexPackageAccess>>) -> Result<()> {
        Self::ensure_caller_for_setter(INDEX_WRITER, INDEX_PACKAGE_ACCESS.get().is_some())?;
        // A Java `null` (`None`) leaves the field `null`, as it was.
        let Some(access) = access else {
            return Ok(());
        };
        INDEX_PACKAGE_ACCESS
            .set(access)
            .map_err(|_| already_set(INDEX_WRITER.name))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to
    /// `TestSecrets.setConcurrentMergeSchedulerAccess(ConcurrentMergeSchedulerAccess)`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when an accessor is already
    /// registered.
    #[track_caller]
    pub fn set_concurrent_merge_scheduler_access(
        access: Option<Arc<dyn ConcurrentMergeSchedulerAccess>>,
    ) -> Result<()> {
        Self::ensure_caller_for_setter(CONCURRENT_MERGE_SCHEDULER, CMS_ACCESS.get().is_some())?;
        // A Java `null` (`None`) leaves the field `null`, as it was.
        let Some(access) = access else {
            return Ok(());
        };
        CMS_ACCESS
            .set(access)
            .map_err(|_| already_set(CONCURRENT_MERGE_SCHEDULER.name))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to `TestSecrets.setSegmentReaderAccess(SegmentReaderAccess)`.
    /// A Java `null` argument is `None`.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when an accessor is already
    /// registered.
    #[track_caller]
    pub fn set_segment_reader_access(access: Option<Arc<dyn SegmentReaderAccess>>) -> Result<()> {
        Self::ensure_caller_for_setter(SEGMENT_READER, SEGMENT_READER_ACCESS.get().is_some())?;
        // A Java `null` (`None`) leaves the field `null`, as it was.
        let Some(access) = access else {
            return Ok(());
        };
        SEGMENT_READER_ACCESS
            .set(access)
            .map_err(|_| already_set(SEGMENT_READER.name))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to
    /// `TestSecrets.setFilterInputIndexAccess(FilterIndexInputAccess)`, keeping
    /// Lucene's own word order.
    ///
    /// # Errors
    ///
    /// Returns Java's `IllegalCallerException` when an accessor is already
    /// registered.
    #[track_caller]
    pub fn set_filter_input_index_access(
        access: Option<Arc<dyn FilterIndexInputAccess>>,
    ) -> Result<()> {
        Self::ensure_caller_for_setter(
            FILTER_INDEX_INPUT,
            FILTER_INDEX_INPUT_ACCESS.get().is_some(),
        )?;
        // A Java `null` (`None`) leaves the field `null`, as it was.
        let Some(access) = access else {
            return Ok(());
        };
        FILTER_INDEX_INPUT_ACCESS
            .set(access)
            .map_err(|_| already_set(FILTER_INDEX_INPUT.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `THIS_FILE_IN_CRATE` names this file, so the prefix derivation holds.
    #[test]
    fn this_file_in_crate_is_a_suffix_of_file_macro() {
        let this_file = file!().replace('\\', "/");
        assert_eq!(
            format!("{}{THIS_FILE_IN_CRATE}", *CRATE_ROOT_PREFIX),
            this_file
        );
    }

    /// Regression for the caller checks outside the workspace: Cargo reports
    /// absolute source paths for path or registry dependencies, and the checks
    /// compared them with workspace-relative ones, so every getter called from
    /// `rucene-test-framework` built as a dependency failed.
    #[test]
    fn callers_match_relative_and_absolute_tree_paths() {
        for prefix in ["", "/data/checkout/Rucene/"] {
            let framework = format!("{prefix}crates/rucene-test-framework/src/store/mock.rs");
            assert!(is_test_framework_file(&framework, prefix));
            assert!(is_test_framework_file(
                &format!("{prefix}tests/a.rs"),
                prefix
            ));
            let core_test = format!("{prefix}crates/rucene-test-framework/tests/core_x.rs");
            assert!(!is_test_framework_file(&core_test, prefix));
            let owner = format!("{prefix}{}", INDEX_WRITER.file);
            assert!(is_owner_file(&owner, INDEX_WRITER.file, prefix));
            assert!(!is_owner_file(&owner, SEGMENT_READER.file, prefix));
        }
    }

    /// Not weakened to a suffix match: another tree whose paths end like the
    /// test framework's or an owner's is refused, and so is a relative path
    /// when the tree's paths are absolute.
    #[test]
    fn callers_from_another_tree_are_refused() {
        let prefix = "/data/checkout/Rucene/";
        let foreign = "/home/u/app/crates/rucene-test-framework/src/x.rs";
        assert!(!is_test_framework_file(foreign, prefix));
        assert!(!is_test_framework_file("/home/u/app/tests/t.rs", prefix));
        assert!(!is_test_framework_file("tests/t.rs", prefix));
        let foreign_owner = "/home/u/app/src/index/index_writer.rs";
        assert!(!is_owner_file(foreign_owner, INDEX_WRITER.file, prefix));
        assert!(!is_owner_file(INDEX_WRITER.file, INDEX_WRITER.file, prefix));
    }

    /// The live getter check reads the real call site: this module is not
    /// part of the test framework, so it is refused with Lucene's message.
    #[test]
    fn getter_check_refuses_a_core_call_site() {
        let error = TestSecrets::ensure_caller_for_getter()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Lucene TestSecrets can only be used by the test-framework."),
            "{error}"
        );
    }
}
