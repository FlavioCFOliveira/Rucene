//! Port of `lucene/core/src/test/org/apache/lucene/util/TestClassLoaderUtils.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{ClassLoader, ClassLoaderUtils};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, test};

/// `TestClassLoaderUtils`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestClassLoaderUtils",
        vec![test("testParentChild", test_parent_child)],
    )
}

fn test_parent_child() -> Result<()> {
    // `getClass().getClassLoader()`: the application class loader.
    let parent = ClassLoader::new("app");
    // `URLClassLoader.newInstance(new URL[0], parent)`.
    let child = ClassLoader::with_parent("java.net.URLClassLoader", Arc::clone(&parent));
    Assert::assert_true(check_no_perms(&parent, &parent));
    Assert::assert_true(check_no_perms(&child, &child));
    Assert::assert_true(check_no_perms(&parent, &child));
    Assert::assert_false(check_no_perms(&child, &parent));
    Ok(())
}

/// `checkNoPerms(parent, child)`.
fn check_no_perms(parent: &Arc<ClassLoader>, child: &Arc<ClassLoader>) -> bool {
    LuceneTestCase::run_with_restricted_permissions(|| {
        ClassLoaderUtils::is_parent_class_loader(parent, child)
    })
}
