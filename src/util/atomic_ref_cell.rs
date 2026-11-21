//! A thread-safe mutable memory location with dynamically checked borrow
//! rules: the `Sync` counterpart of [`std::cell::RefCell`].
//!
//! **This has no counterpart in Lucene 10.5.0.** Java shares any object
//! between threads by reference: a `Document` built on one thread is indexed by
//! several others (`TestIndexWriterDelete.testDeleteAllRepeated`), and the
//! token streams, readers and attribute implementations it reaches are plain
//! objects whose mutation Java leaves to the caller's discipline. Rust needs
//! the shared state to be `Send + Sync`; a `RefCell` is neither `Sync` nor,
//! behind an `Rc`, `Send`. This cell gives the same borrow-checked interior
//! mutability as `RefCell` — the same `borrow`, `borrow_mut`, `try_borrow`,
//! `try_borrow_mut` and mappable guards — with an atomic borrow flag, so that
//! an `Arc<AtomicRefCell<T>>` can stand where Java holds a shared reference.
//!
//! The design is the one of the `atomic_refcell` crate by Bobby Holley
//! (version 0.1.13, MIT OR Apache-2.0, `src/lib.rs`): one `AtomicUsize` whose
//! high bit marks the exclusive borrow and whose remaining bits count shared
//! borrows, guards that hold a `NonNull` (not a reference, so that a guard
//! passed by value never asserts the validity of the value after its borrow is
//! released), and the overflow guard on failed shared borrows. It is written
//! here instead of taken as a dependency (user decision on task #379).
//!
//! A borrow that conflicts panics, as `RefCell`'s does; the cell never blocks.

#![allow(unsafe_code)]

use std::cell::UnsafeCell;
use std::fmt::{self, Debug, Display, Formatter};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The flag value of an exclusive borrow.
const HIGH_BIT: usize = !(usize::MAX >> 1);

/// The count of failed shared borrows during one exclusive borrow past which
/// the process aborts, before the counter could wrap into a valid state.
const MAX_FAILED_BORROWS: usize = HIGH_BIT + (HIGH_BIT >> 1);

/// A mutable memory location shareable between threads, whose borrows are
/// checked at run time.
///
/// See the [module documentation](self).
pub struct AtomicRefCell<T: ?Sized> {
    /// `0`: free; `n` below [`HIGH_BIT`]: `n` shared borrows; [`HIGH_BIT`]
    /// set: one exclusive borrow (plus the failed shared attempts made since).
    borrow: AtomicUsize,
    value: UnsafeCell<T>,
}

/// The error [`AtomicRefCell::try_borrow`] returns while the value is
/// borrowed exclusively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowError {
    _private: (),
}

impl Display for BorrowError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("already mutably borrowed")
    }
}

impl std::error::Error for BorrowError {}

/// The error [`AtomicRefCell::try_borrow_mut`] returns while the value is
/// borrowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowMutError {
    _private: (),
}

impl Display for BorrowMutError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("already borrowed")
    }
}

impl std::error::Error for BorrowMutError {}

impl<T> AtomicRefCell<T> {
    /// Creates a cell holding `value`.
    pub const fn new(value: T) -> Self {
        Self {
            borrow: AtomicUsize::new(0),
            value: UnsafeCell::new(value),
        }
    }

    /// Consumes the cell and returns its value.
    pub fn into_inner(self) -> T {
        debug_assert!(self.borrow.load(Ordering::Acquire) == 0);
        self.value.into_inner()
    }

    /// Replaces the value with `value` and returns the old one.
    ///
    /// # Panics
    ///
    /// When the value is borrowed.
    pub fn replace(&self, value: T) -> T {
        std::mem::replace(&mut *self.borrow_mut(), value)
    }

    /// Takes the value, leaving `T::default()` in its place.
    ///
    /// # Panics
    ///
    /// When the value is borrowed.
    pub fn take(&self) -> T
    where
        T: Default,
    {
        self.replace(T::default())
    }
}

impl<T: ?Sized> AtomicRefCell<T> {
    /// Borrows the value immutably; any number of shared borrows may coexist.
    ///
    /// # Panics
    ///
    /// When the value is borrowed exclusively.
    #[track_caller]
    pub fn borrow(&self) -> AtomicRef<'_, T> {
        match self.try_borrow() {
            Ok(guard) => guard,
            Err(error) => panic!("{error}"),
        }
    }

    /// Borrows the value immutably, or fails while it is borrowed exclusively.
    ///
    /// # Errors
    ///
    /// [`BorrowError`] while an exclusive borrow is alive.
    pub fn try_borrow(&self) -> Result<AtomicRef<'_, T>, BorrowError> {
        let borrow = SharedBorrow::try_new(&self.borrow)?;
        Ok(AtomicRef {
            // SAFETY: `UnsafeCell::get` never returns null. Holding `borrow`
            // excludes every exclusive borrow until the guard drops, so shared
            // reads through this pointer cannot race a write.
            value: unsafe { NonNull::new_unchecked(self.value.get()) },
            borrow,
        })
    }

    /// Borrows the value mutably; the borrow excludes every other.
    ///
    /// # Panics
    ///
    /// When the value is borrowed.
    #[track_caller]
    pub fn borrow_mut(&self) -> AtomicRefMut<'_, T> {
        match self.try_borrow_mut() {
            Ok(guard) => guard,
            Err(error) => panic!("{error}"),
        }
    }

    /// Borrows the value mutably, or fails while it is borrowed.
    ///
    /// # Errors
    ///
    /// [`BorrowMutError`] while any other borrow is alive.
    pub fn try_borrow_mut(&self) -> Result<AtomicRefMut<'_, T>, BorrowMutError> {
        let borrow = ExclusiveBorrow::try_new(&self.borrow)?;
        Ok(AtomicRefMut {
            // SAFETY: `UnsafeCell::get` never returns null. Holding `borrow`
            // excludes every other borrow until the guard drops, so this
            // pointer is the only access to the value for that span.
            value: unsafe { NonNull::new_unchecked(self.value.get()) },
            borrow,
            marker: PhantomData,
        })
    }

    /// Returns a mutable reference to the value; the `&mut self` receiver
    /// proves statically that no borrow is alive.
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    /// Returns a raw pointer to the value.
    pub fn as_ptr(&self) -> *mut T {
        self.value.get()
    }
}

// SAFETY: the cell owns its value; sending the cell sends the value, which is
// sound exactly when `T: Send`.
unsafe impl<T: ?Sized + Send> Send for AtomicRefCell<T> {}

// SAFETY: through `&AtomicRefCell<T>` one thread may obtain `&mut T` (needs
// `T: Send`, since the value can be moved out with `replace`) while other
// threads obtain `&T` at other times (needs `T: Sync`). The atomic flag
// serialises the two kinds of access: an exclusive borrow is granted by a
// compare-exchange from `0` with `Acquire` and released by a `Release` store,
// a shared borrow by an `Acquire` increment and released by a `Release`
// decrement, so every access happens-after the release of the conflicting
// borrow before it.
unsafe impl<T: ?Sized + Send + Sync> Sync for AtomicRefCell<T> {}

impl<T: Default> Default for AtomicRefCell<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> From<T> for AtomicRefCell<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Clone> Clone for AtomicRefCell<T> {
    /// # Panics
    ///
    /// When the value is borrowed exclusively.
    fn clone(&self) -> Self {
        Self::new(self.borrow().clone())
    }
}

impl<T: ?Sized + Debug> Debug for AtomicRefCell<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.try_borrow() {
            Ok(value) => f
                .debug_struct("AtomicRefCell")
                .field("value", &&*value)
                .finish(),
            Err(_) => f
                .debug_struct("AtomicRefCell")
                .field("value", &format_args!("<borrowed>"))
                .finish(),
        }
    }
}

/// The record of one shared borrow; dropping it releases the borrow.
struct SharedBorrow<'b> {
    borrow: &'b AtomicUsize,
}

impl<'b> SharedBorrow<'b> {
    fn try_new(borrow: &'b AtomicUsize) -> Result<Self, BorrowError> {
        let new = borrow.fetch_add(1, Ordering::Acquire).wrapping_add(1);
        if new & HIGH_BIT != 0 {
            // An exclusive borrow is alive. The increment is left in place as
            // a benign side effect: releasing the exclusive borrow stores `0`
            // unconditionally. The overflow check keeps an unbounded number of
            // failed attempts from wrapping the counter into a state that
            // would grant a borrow.
            Self::check_overflow(borrow, new);
            Err(BorrowError { _private: () })
        } else {
            Ok(Self { borrow })
        }
    }

    #[cold]
    fn check_overflow(borrow: &AtomicUsize, new: usize) {
        if new == HIGH_BIT {
            // The shared count itself reached the high bit. Undo the increment
            // to leave a consistent shared state before panicking.
            borrow.fetch_sub(1, Ordering::Release);
            panic!("too many immutable borrows");
        } else if new >= MAX_FAILED_BORROWS {
            // An absurd number of failed attempts during one exclusive borrow.
            // Panicking while this guard unwinds aborts the process, which is
            // the only sound exit: the counter cannot be repaired here.
            struct ForceAbort;
            impl Drop for ForceAbort {
                fn drop(&mut self) {
                    panic!("aborting to avoid an unsound AtomicRefCell state");
                }
            }
            let _abort = ForceAbort;
            panic!("too many failed borrows");
        }
    }
}

impl Drop for SharedBorrow<'_> {
    fn drop(&mut self) {
        let old = self.borrow.fetch_sub(1, Ordering::Release);
        // A shared borrow is alive, so no exclusive borrow can be.
        debug_assert!(old & HIGH_BIT == 0);
    }
}

impl Clone for SharedBorrow<'_> {
    fn clone(&self) -> Self {
        Self::try_new(self.borrow).expect("INVARIANT: a shared borrow excludes exclusive borrows")
    }
}

/// The record of the exclusive borrow; dropping it releases the borrow.
struct ExclusiveBorrow<'b> {
    borrow: &'b AtomicUsize,
}

impl<'b> ExclusiveBorrow<'b> {
    fn try_new(borrow: &'b AtomicUsize) -> Result<Self, BorrowMutError> {
        // `Relaxed` on failure: a failed attempt reads nothing through the
        // cell.
        match borrow.compare_exchange(0, HIGH_BIT, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => Ok(Self { borrow }),
            Err(_) => Err(BorrowMutError { _private: () }),
        }
    }
}

impl Drop for ExclusiveBorrow<'_> {
    fn drop(&mut self) {
        // Discards the increments of the shared attempts that failed while
        // this borrow was alive.
        debug_assert!(self.borrow.load(Ordering::Relaxed) & HIGH_BIT != 0);
        self.borrow.store(0, Ordering::Release);
    }
}

/// A shared borrow of the value of an [`AtomicRefCell`]; the counterpart of
/// [`std::cell::Ref`].
pub struct AtomicRef<'b, T: ?Sized + 'b> {
    value: NonNull<T>,
    borrow: SharedBorrow<'b>,
}

impl<'b, T: ?Sized> AtomicRef<'b, T> {
    /// Makes a new guard for a component of the borrowed value.
    pub fn map<U: ?Sized, F>(orig: AtomicRef<'b, T>, f: F) -> AtomicRef<'b, U>
    where
        F: FnOnce(&T) -> &U,
    {
        AtomicRef {
            value: NonNull::from(f(&*orig)),
            borrow: orig.borrow,
        }
    }

    /// Makes a new guard for an optional component of the borrowed value, or
    /// gives the original guard back.
    ///
    /// # Errors
    ///
    /// The original guard, when `f` returns `None`.
    pub fn filter_map<U: ?Sized, F>(orig: AtomicRef<'b, T>, f: F) -> Result<AtomicRef<'b, U>, Self>
    where
        F: FnOnce(&T) -> Option<&U>,
    {
        match f(&*orig).map(NonNull::from) {
            Some(value) => Ok(AtomicRef {
                value,
                borrow: orig.borrow,
            }),
            None => Err(orig),
        }
    }

    /// Copies the guard: one more shared borrow of the same value.
    ///
    /// An associated function, as `Ref::clone`, so that it does not shadow
    /// `T::clone` through `Deref`.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(orig: &AtomicRef<'b, T>) -> AtomicRef<'b, T> {
        AtomicRef {
            value: orig.value,
            borrow: orig.borrow.clone(),
        }
    }
}

impl<T: ?Sized> Deref for AtomicRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: `value` points into the cell, which outlives `'b`, and the
        // shared borrow this guard holds excludes every exclusive borrow, so
        // no `&mut T` exists while this `&T` does.
        unsafe { self.value.as_ref() }
    }
}

// SAFETY: the guard is a `&T` with a borrow count attached; it may cross or be
// shared between threads exactly when `&T` may, that is when `T: Sync`. The
// count is atomic.
unsafe impl<T: ?Sized + Sync> Send for AtomicRef<'_, T> {}

// SAFETY: as for `Send`: sharing the guard shares only `&T`.
unsafe impl<T: ?Sized + Sync> Sync for AtomicRef<'_, T> {}

impl<T: ?Sized + Debug> Debug for AtomicRef<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized + Display> Display for AtomicRef<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// The exclusive borrow of the value of an [`AtomicRefCell`]; the
/// counterpart of [`std::cell::RefMut`].
pub struct AtomicRefMut<'b, T: ?Sized + 'b> {
    value: NonNull<T>,
    borrow: ExclusiveBorrow<'b>,
    /// Makes the guard invariant in `T`, as `&'b mut T` is.
    marker: PhantomData<&'b mut T>,
}

impl<'b, T: ?Sized> AtomicRefMut<'b, T> {
    /// Makes a new guard for a component of the borrowed value.
    pub fn map<U: ?Sized, F>(mut orig: AtomicRefMut<'b, T>, f: F) -> AtomicRefMut<'b, U>
    where
        F: FnOnce(&mut T) -> &mut U,
    {
        let value = NonNull::from(f(&mut *orig));
        AtomicRefMut {
            value,
            borrow: orig.borrow,
            marker: PhantomData,
        }
    }

    /// Makes a new guard for an optional component of the borrowed value, or
    /// gives the original guard back.
    ///
    /// # Errors
    ///
    /// The original guard, when `f` returns `None`.
    pub fn filter_map<U: ?Sized, F>(
        mut orig: AtomicRefMut<'b, T>,
        f: F,
    ) -> Result<AtomicRefMut<'b, U>, Self>
    where
        F: FnOnce(&mut T) -> Option<&mut U>,
    {
        match f(&mut *orig).map(NonNull::from) {
            Some(value) => Ok(AtomicRefMut {
                value,
                borrow: orig.borrow,
                marker: PhantomData,
            }),
            None => Err(orig),
        }
    }
}

impl<T: ?Sized> Deref for AtomicRefMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: `value` points into the cell, which outlives `'b`, and this
        // guard holds the cell's only borrow; the returned `&T` borrows the
        // guard, so no `&mut T` from `deref_mut` can coexist with it.
        unsafe { self.value.as_ref() }
    }
}

impl<T: ?Sized> DerefMut for AtomicRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as for `deref`; the returned `&mut T` borrows the guard
        // mutably, so it is the only reference to the value while it lives.
        unsafe { self.value.as_mut() }
    }
}

// SAFETY: the guard is a `&mut T` with the exclusive flag attached; it may
// cross threads exactly when `&mut T` may, that is when `T: Send`. Dropping it
// on another thread only stores to the atomic flag.
unsafe impl<T: ?Sized + Send> Send for AtomicRefMut<'_, T> {}

// SAFETY: sharing the guard lends only `&T` (`DerefMut` needs `&mut` access to
// the guard), which is sound when `T: Sync`.
unsafe impl<T: ?Sized + Sync> Sync for AtomicRefMut<'_, T> {}

impl<T: ?Sized + Debug> Debug for AtomicRefMut<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized + Display> Display for AtomicRefMut<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn assert_send_sync<T: ?Sized + Send + Sync>() {}
    fn assert_send<T: ?Sized + Send>() {}

    #[test]
    fn cell_and_guards_are_send_and_sync_for_send_sync_values() {
        assert_send_sync::<AtomicRefCell<String>>();
        assert_send_sync::<Arc<AtomicRefCell<Vec<u8>>>>();
        assert_send_sync::<AtomicRefCell<dyn Debug + Send + Sync>>();
        assert_send_sync::<AtomicRef<'static, String>>();
        assert_send_sync::<AtomicRefMut<'static, String>>();
        // A `Send`-only value makes a `Send` cell (as `Mutex` would not need
        // `Sync` either, but this cell lends `&T` to several threads).
        assert_send::<AtomicRefCell<std::cell::Cell<u8>>>();
    }

    #[test]
    fn shared_borrows_coexist() {
        let cell = AtomicRefCell::new(5);
        let a = cell.borrow();
        let b = cell.borrow();
        let c = AtomicRef::clone(&a);
        assert_eq!(*a + *b + *c, 15);
        assert!(cell.try_borrow_mut().is_err());
        drop((a, b, c));
        assert!(cell.try_borrow_mut().is_ok());
    }

    #[test]
    fn exclusive_borrow_excludes_all_others() {
        let cell = AtomicRefCell::new(String::from("a"));
        {
            let mut w = cell.borrow_mut();
            w.push('b');
            assert_eq!(
                cell.try_borrow().unwrap_err().to_string(),
                "already mutably borrowed"
            );
            assert_eq!(
                cell.try_borrow_mut().unwrap_err().to_string(),
                "already borrowed"
            );
            // Failed shared attempts are discarded on release.
            for _ in 0..10 {
                assert!(cell.try_borrow().is_err());
            }
        }
        assert_eq!(&*cell.borrow(), "ab");
        assert!(cell.try_borrow_mut().is_ok());
        assert_eq!(cell.borrow.load(Ordering::Relaxed), 0);
    }

    #[test]
    #[should_panic(expected = "already mutably borrowed")]
    fn borrow_while_mutably_borrowed_panics() {
        let cell = AtomicRefCell::new(1);
        let _w = cell.borrow_mut();
        let _r = cell.borrow();
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn borrow_mut_while_borrowed_panics() {
        let cell = AtomicRefCell::new(1);
        let _r = cell.borrow();
        let _w = cell.borrow_mut();
    }

    #[test]
    fn mapped_guards_keep_the_borrow() {
        let cell = AtomicRefCell::new((1u32, String::from("x")));
        let second = AtomicRef::map(cell.borrow(), |pair| pair.1.as_str());
        assert_eq!(&*second, "x");
        assert!(cell.try_borrow_mut().is_err());
        drop(second);
        {
            let mut first = AtomicRefMut::map(cell.borrow_mut(), |pair| &mut pair.0);
            *first += 1;
            assert!(cell.try_borrow().is_err());
        }
        assert_eq!(cell.borrow().0, 2);
        let missing = AtomicRef::filter_map(cell.borrow(), |_| None::<&u32>);
        assert!(missing.is_err());
        drop(missing);
        let mut present = AtomicRefMut::filter_map(cell.borrow_mut(), |pair| Some(&mut pair.1))
            .ok()
            .unwrap();
        present.push('y');
        drop(present);
        assert_eq!(cell.into_inner().1, "xy");
    }

    #[test]
    fn unsized_values_through_arc() {
        let cell: Arc<AtomicRefCell<dyn Debug + Send + Sync>> = Arc::new(AtomicRefCell::new(7u8));
        assert_eq!(format!("{:?}", &*cell.borrow()), "7");
        let mut values: Arc<AtomicRefCell<Vec<u8>>> = Arc::new(AtomicRefCell::default());
        Arc::get_mut(&mut values).unwrap().get_mut().push(1);
        assert_eq!(values.replace(vec![2]), vec![1]);
        assert_eq!(values.take(), vec![2]);
        assert_eq!(format!("{:?}", values), "AtomicRefCell { value: [] }");
        let _w = values.borrow_mut();
        assert_eq!(
            format!("{:?}", values),
            "AtomicRefCell { value: <borrowed> }"
        );
    }

    #[test]
    fn concurrent_shared_borrows_and_serialised_writes() {
        const THREADS: usize = 8;
        const ROUNDS: usize = 2_000;
        let cell = Arc::new(AtomicRefCell::new(0usize));
        let barrier = Barrier::new(THREADS);
        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    barrier.wait();
                    let mut written = 0;
                    while written < ROUNDS {
                        // Writers spin until they win the exclusive borrow;
                        // readers must never see a borrow conflict with
                        // another reader.
                        if let Ok(mut w) = cell.try_borrow_mut() {
                            *w += 1;
                            written += 1;
                        }
                        if let Ok(r) = cell.try_borrow() {
                            let r2 = cell.try_borrow().expect("shared borrows coexist");
                            assert_eq!(*r, *r2);
                        }
                    }
                });
            }
        });
        assert_eq!(*cell.borrow(), THREADS * ROUNDS);
        assert_eq!(cell.borrow.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn guards_cross_threads() {
        let cell = AtomicRefCell::new(vec![1, 2, 3]);
        let r = cell.borrow();
        std::thread::scope(|scope| {
            scope.spawn(move || assert_eq!(r.len(), 3));
        });
        let mut w = cell.borrow_mut();
        std::thread::scope(|scope| {
            scope.spawn(|| w.push(4));
        });
        drop(w);
        assert_eq!(cell.borrow().len(), 4);
    }
}
