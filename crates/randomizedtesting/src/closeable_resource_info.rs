//! Port of `com.carrotsearch.randomizedtesting.CloseableResourceInfo`
//! (randomizedtesting 2.8.1, `CloseableResourceInfo.java`).

use std::backtrace::Backtrace;
use std::sync::Arc;

use crate::jdk::io::Closeable;
use crate::jdk::thread::Thread;
use crate::lifecycle_scope::LifecycleScope;
use crate::threads::Threads;

/// Allocation information (thread, allocation stack) of a disposable
/// resource: `CloseableResourceInfo` (package-private).
///
/// **Divergence.** Java keeps the registering thread's
/// `StackTraceElement[]`; the port keeps a captured
/// `std::backtrace::Backtrace` (empty unless backtrace capture is enabled).
pub(crate) struct CloseableResourceInfo {
    resource: Arc<dyn Closeable>,
    scope: LifecycleScope,
    allocation_stack: Arc<Backtrace>,
    thread_name: String,
}

impl CloseableResourceInfo {
    /// `new CloseableResourceInfo(resource, scope, t, allocationStack)`.
    pub(crate) fn new(
        resource: Arc<dyn Closeable>,
        scope: LifecycleScope,
        t: &Thread,
        allocation_stack: Arc<Backtrace>,
    ) -> CloseableResourceInfo {
        CloseableResourceInfo {
            resource,
            thread_name: Threads::thread_name(t),
            allocation_stack,
            scope,
        }
    }

    /// `getResource()`.
    pub(crate) fn get_resource(&self) -> &Arc<dyn Closeable> {
        &self.resource
    }

    /// `getAllocationStack()`.
    pub(crate) fn get_allocation_stack(&self) -> &Arc<Backtrace> {
        &self.allocation_stack
    }

    /// `getScope()`.
    pub(crate) fn get_scope(&self) -> LifecycleScope {
        self.scope
    }

    /// `getThreadName()`: the allocating thread's name when the resource was
    /// registered (`Threads.threadName`).
    pub(crate) fn get_thread_name(&self) -> &str {
        &self.thread_name
    }
}
