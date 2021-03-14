/*
 * Copyright 2021 Actyx AG
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use crate::{
    loom::*,
    ref_count::{Interned, Interner},
};
use std::{
    borrow::Borrow,
    cmp::Ordering,
    collections::BTreeSet,
    fmt::{Debug, Display, Formatter, Pointer},
    hash::Hasher,
    ops::Deref,
    sync::Arc,
};

/// Interner for values with a total order
///
/// The interner is cheaply cloneable by virtue of keeping the underlying storage
/// in an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html). Once the last
/// reference to this interner is dropped, it will clear its backing storage and
/// release all references to the interned values it has created that are still live.
/// Those values remain fully operational until dropped. Memory for the values
/// themselves is freed for each value individually once its last reference is dropped.
/// The interner’s ArcInner memory will only be deallocated once the last interned
/// value has been dropped (this is less than hundred bytes).
pub struct OrdInterner<T: ?Sized + std::cmp::Ord> {
    inner: Arc<Ord<T>>,
}

impl<T: ?Sized + std::cmp::Ord> Clone for OrdInterner<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[repr(C)]
pub struct Ord<T: ?Sized + std::cmp::Ord> {
    set: RwLock<BTreeSet<InternedOrd<T>>>,
}

#[cfg(loom)]
impl<T: ?Sized> Drop for Inner<T> {
    fn drop(&mut self) {
        // This is needed to “see” interned values added from other threads in loom
        // (since loom doesn’t provide Weak and therefore we cannot use its Arc).
        self.set.read();
    }
}

unsafe impl<T: ?Sized + std::cmp::Ord + Sync + Send> Send for Ord<T> {}
unsafe impl<T: ?Sized + std::cmp::Ord + Sync + Send> Sync for Ord<T> {}

impl<T: ?Sized + std::cmp::Ord> Interner for Ord<T> {
    type T = T;

    fn remove(&self, value: &Interned<Self>) -> (bool, Option<Interned<Self>>) {
        let value = cast(value);
        let mut set = self.set.write();
        #[cfg(loom)]
        let mut set = set.unwrap();
        if let Some(i) = set.take(value) {
            if i.ref_count() == 1 {
                (true, Some(i.0))
            } else {
                set.insert(i);
                (false, None)
            }
        } else {
            (true, None)
        }
    }
}

impl<T: ?Sized + std::cmp::Ord> OrdInterner<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Ord {
                set: RwLock::new(BTreeSet::new()),
            }),
        }
    }

    /// Returns the number of objects currently kept in this interner.
    pub fn len(&self) -> usize {
        let set = self.inner.set.read();
        #[cfg(loom)]
        let set = set.unwrap();
        set.len()
    }

    /// Returns `true` when this interner doesn’t hold any values.
    pub fn is_empty(&self) -> bool {
        let set = self.inner.set.read();
        #[cfg(loom)]
        let set = set.unwrap();
        set.is_empty()
    }

    fn intern<U, F>(&self, value: U, intern: F) -> InternedOrd<T>
    where
        F: FnOnce(U) -> InternedOrd<T>,
        U: Borrow<T>,
    {
        #[cfg(not(loom))]
        let set = self.inner.set.upgradable_read();
        #[cfg(loom)]
        let set = self.inner.set.read().unwrap();
        if let Some(entry) = set.get(value.borrow()) {
            return entry.clone();
        }
        #[cfg(not(loom))]
        let mut set = RwLockUpgradableReadGuard::upgrade(set);
        #[cfg(loom)]
        let mut set = {
            drop(set);
            self.inner.set.write().unwrap()
        };
        // check again with write lock to guard against concurrent insertion
        if let Some(entry) = set.get(value.borrow()) {
            return entry.clone();
        }
        let mut ret = intern(value);
        ret.0.make_hot(&self.inner);
        set.insert(ret.clone());
        ret
    }

    /// Intern a value from a shared reference by allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{OrdInterner, InternedOrd};
    ///
    /// let strings = OrdInterner::<str>::new();
    /// let i: InternedOrd<str> = strings.intern_ref("hello world!");
    /// ```
    pub fn intern_ref(&self, value: &T) -> InternedOrd<T>
    where
        T: ToOwned,
        T::Owned: Into<Box<T>>,
    {
        self.intern(value, |v| {
            InternedOrd(Interned::from_box(v.to_owned().into()))
        })
    }

    /// Intern a value from an owned reference without allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{OrdInterner, InternedOrd};
    ///
    /// let strings = OrdInterner::<str>::new();
    /// let hello: Box<str> = "hello world!".into();
    /// let i: InternedOrd<str> = strings.intern_box(hello);
    /// ```
    /// (This also works nicely with a `String` that can be turned `.into()` a `Box`.)
    pub fn intern_box(&self, value: Box<T>) -> InternedOrd<T> {
        self.intern(value, |v| InternedOrd(Interned::from_box(v)))
    }

    /// Intern a sized value, allocating heap memory for it.
    ///
    /// ```
    /// use intern_arc::{OrdInterner, InternedOrd};
    ///
    /// let arrays = OrdInterner::<[u8; 1000]>::new();
    /// let i: InternedOrd<[u8; 1000]> = arrays.intern_sized([0; 1000]);
    pub fn intern_sized(&self, value: T) -> InternedOrd<T>
    where
        T: Sized,
    {
        self.intern(value, |v| InternedOrd(Interned::from_sized(v)))
    }
}

impl<T: ?Sized + std::cmp::Ord> Default for OrdInterner<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// An interned value
///
/// This type works very similar to an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html)
/// with the difference that it has no concept of weak references. They are not needed because
/// **interned values must not be modified**, so reference cycles cannot be constructed. One
/// reference is held by the interner that created this value as long as that interner lives.
///
/// Keeping interned values around does not keep the interner alive: once the last reference to
/// the interner is dropped, it will release its existing references to interned values by
/// dropping its set implementation. Only the direct allocation size of the set implementation
/// remains allocated (but no longer functional) until the last weak reference to the interner
/// goes away — each interned value keeps one such weak reference to its interner.
#[repr(transparent)] // this is important to cast references
pub struct InternedOrd<T: ?Sized + std::cmp::Ord>(Interned<Ord<T>>);

impl<T: ?Sized + std::cmp::Ord> InternedOrd<T> {
    /// Obtain current number of references, including this one.
    ///
    /// The value will always be at least 1. If the value is 1, this means that the interner
    /// which produced this reference has been dropped; in this case you are still free to
    /// use this reference in any way you like.
    pub fn ref_count(&self) -> u32 {
        self.0.ref_count()
    }
}

impl<T: ?Sized + std::cmp::Ord> Clone for InternedOrd<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

fn cast<T: ?Sized + std::cmp::Ord>(i: &Interned<Ord<T>>) -> &InternedOrd<T> {
    // since the memory representation of InternedOrd<T> is exactly the same as
    // Interned<Ord<T>>, we can turn a pointer to one into a pointer to the other
    unsafe { &*(i as *const _ as *const InternedOrd<T>) }
}

impl<T: ?Sized + std::cmp::Ord> PartialEq for InternedOrd<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        // equal pointer means same interner and same contents
        self.0 == other.0 || self.deref().eq(other.deref())
    }
}
impl<T: ?Sized + std::cmp::Ord> Eq for InternedOrd<T> where T: Eq {}

impl<T: ?Sized + std::cmp::Ord> PartialOrd for InternedOrd<T>
where
    T: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.0 == other.0 {
            return Some(Ordering::Equal);
        }
        self.deref().partial_cmp(other.deref())
    }
}
impl<T: ?Sized + std::cmp::Ord> std::cmp::Ord for InternedOrd<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.0 == other.0 {
            return Ordering::Equal;
        }
        self.deref().cmp(other.deref())
    }
}

impl<T: ?Sized + std::cmp::Ord> std::hash::Hash for InternedOrd<T>
where
    T: std::hash::Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T: ?Sized + std::cmp::Ord> Borrow<T> for InternedOrd<T> {
    fn borrow(&self) -> &T {
        self.deref()
    }
}

impl<T: ?Sized + std::cmp::Ord> Deref for InternedOrd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: ?Sized + std::cmp::Ord> AsRef<T> for InternedOrd<T> {
    fn as_ref(&self) -> &T {
        self.deref()
    }
}

impl<T: ?Sized + std::cmp::Ord> Debug for InternedOrd<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Interned({:?})", &*self)
    }
}

impl<T: ?Sized + std::cmp::Ord> Display for InternedOrd<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

impl<T: ?Sized + std::cmp::Ord> Pointer for InternedOrd<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Pointer::fmt(&(&**self as *const T), f)
    }
}

#[test]
fn size() {
    let s = std::mem::size_of::<Ord<()>>();
    assert!(s < 100, "too big: {}", s);
}

#[cfg(all(test, loom))]
mod tests {
    use super::*;
    use ::loom::{model, thread::spawn};

    fn counts<T>(weak: Weak<T>) -> (usize, usize) {
        // this is unfortunate: Arc does not allow querying weak_count once strong_count is zero
        unsafe {
            let ptr = &weak as *const _ as *const *const (usize, usize);
            **ptr
        }
    }

    #[test]
    fn drop_interner() {
        model(|| {
            let i = OrdInterner::new();
            let i2 = Arc::downgrade(&i.inner);

            let n = i.intern_box(42.into());

            let h = spawn(move || drop(i));
            let h2 = spawn(move || drop(n));

            h.join().unwrap();
            h2.join().unwrap();

            assert_eq!(counts(i2), (0, 1));
        })
    }

    #[test]
    fn drop_two_external() {
        model(|| {
            let i = OrdInterner::new();
            let i2 = Arc::downgrade(&i.inner);

            let n = i.intern_box(42.into());
            let n2 = n.clone();
            drop(i);

            let h = spawn(move || drop(n));
            let h2 = spawn(move || drop(n2));

            h.join().unwrap();
            h2.join().unwrap();

            assert_eq!(counts(i2), (0, 1));
        })
    }

    #[test]
    fn drop_against_intern() {
        model(|| {
            let i = OrdInterner::new();
            let i2 = Arc::downgrade(&i.inner);

            let n = i.intern_box(42.into());

            let h1 = spawn(move || drop(n));
            let h2 = spawn(move || i.intern_box(42.into()));

            h1.join().unwrap();
            h2.join().unwrap();

            assert_eq!(counts(i2), (0, 1));
        })
    }

    #[test]
    fn tree_drop_against_intern_and_interner() {
        model(|| {
            let i = OrdInterner::new();
            let i2 = Arc::downgrade(&i.inner);

            let n = i.intern_box(42.into());
            let ii = i.clone();

            println!("{:?} setup", current().id());
            let h1 = spawn(move || drop(n));
            let h2 = spawn(move || i.intern_box(42.into()));
            let h3 = spawn(move || drop(ii));

            println!("{:?} joining", current().id());
            h1.join().unwrap();
            h2.join().unwrap();
            h3.join().unwrap();

            assert_eq!(counts(i2), (0, 1));
            println!("{:?} done", current().id());
        })
    }
}
