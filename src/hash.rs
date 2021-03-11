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
    collections::HashSet,
    fmt::{Debug, Display, Formatter, Pointer},
    hash::Hasher,
    ops::Deref,
    sync::Arc,
};

pub struct HashInterner<T: ?Sized + Eq + std::hash::Hash> {
    inner: Arc<Hash<T>>,
}

impl<T: ?Sized + Eq + std::hash::Hash> Clone for HashInterner<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[repr(C)]
pub struct Hash<T: ?Sized + Eq + std::hash::Hash> {
    set: RwLock<HashSet<InternedHash<T>>>,
}

#[cfg(loom)]
impl<T: ?Sized> Drop for Inner<T> {
    fn drop(&mut self) {
        // This is needed to “see” interned values added from other threads in loom
        // (since loom doesn’t provide Weak and therefore we cannot use its Arc).
        self.set.read();
    }
}

unsafe impl<T: ?Sized + Eq + std::hash::Hash + Sync + Send> Send for Hash<T> {}
unsafe impl<T: ?Sized + Eq + std::hash::Hash + Sync + Send> Sync for Hash<T> {}

impl<T: ?Sized + Eq + std::hash::Hash> Interner for Hash<T> {
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

/// Interner for hashable values
///
/// The interner is cheaply cloneable by virtue of keeping the underlying storage
/// in an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html). Once the last
/// reference to this interner is dropped, it will clear its backing storage and
/// release all references to the interned values it has created that are still live.
/// Those values remain fully operational until dropped. Memory for the values
/// themselves is freed for each value individually once its last reference is dropped.
impl<T: ?Sized + Eq + std::hash::Hash> HashInterner<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Hash {
                set: RwLock::new(HashSet::new()),
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

    fn intern<U, F>(&self, value: U, intern: F) -> InternedHash<T>
    where
        F: FnOnce(U) -> InternedHash<T>,
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
    /// use intern_arc::{HashInterner, InternedHash};
    ///
    /// let strings = HashInterner::<str>::new();
    /// let i: InternedHash<str> = strings.intern_ref("hello world!");
    /// ```
    pub fn intern_ref(&self, value: &T) -> InternedHash<T>
    where
        T: ToOwned,
        T::Owned: Into<Box<T>>,
    {
        self.intern(value, |v| {
            InternedHash(Interned::from_box(v.to_owned().into()))
        })
    }

    /// Intern a value from an owned reference without allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{HashInterner, InternedHash};
    ///
    /// let strings = HashInterner::<str>::new();
    /// let hello: Box<str> = "hello world!".into();
    /// let i: InternedHash<str> = strings.intern_box(hello);
    /// ```
    /// (This also works nicely with a `String` that can be turned `.into()` a `Box`.)
    pub fn intern_box(&self, value: Box<T>) -> InternedHash<T> {
        self.intern(value, |v| InternedHash(Interned::from_box(v)))
    }

    /// Intern a sized value, allocating heap memory for it.
    ///
    /// ```
    /// use intern_arc::{HashInterner, InternedHash};
    ///
    /// let arrays = HashInterner::<[u8; 1000]>::new();
    /// let i: InternedHash<[u8; 1000]> = arrays.intern_sized([0; 1000]);
    pub fn intern_sized(&self, value: T) -> InternedHash<T>
    where
        T: Sized,
    {
        self.intern(value, |v| InternedHash(Interned::from_sized(v)))
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Default for HashInterner<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(transparent)] // this is important to cast references
pub struct InternedHash<T: ?Sized + Eq + std::hash::Hash>(Interned<Hash<T>>);

impl<T: ?Sized + Eq + std::hash::Hash> InternedHash<T> {
    pub fn ref_count(&self) -> u32 {
        self.0.ref_count()
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Clone for InternedHash<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

fn cast<T: ?Sized + Eq + std::hash::Hash>(i: &Interned<Hash<T>>) -> &InternedHash<T> {
    // since the memory representation of InternedHash<T> is exactly the same as
    // Interned<Hash<T>>, we can turn a pointer to one into a pointer to the other
    unsafe { &*(i as *const _ as *const InternedHash<T>) }
}

impl<T: ?Sized + Eq + std::hash::Hash> PartialEq for InternedHash<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}
impl<T: ?Sized + Eq + std::hash::Hash> Eq for InternedHash<T> where T: Eq {}

impl<T: ?Sized + Eq + std::hash::Hash> PartialOrd for InternedHash<T>
where
    T: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.deref().partial_cmp(other.deref())
    }
}
impl<T: ?Sized + Eq + std::hash::Hash> Ord for InternedHash<T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> std::hash::Hash for InternedHash<T>
where
    T: std::hash::Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Borrow<T> for InternedHash<T> {
    fn borrow(&self) -> &T {
        self.deref()
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Deref for InternedHash<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> AsRef<T> for InternedHash<T> {
    fn as_ref(&self) -> &T {
        self.deref()
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Debug for InternedHash<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Interned({:?})", &*self)
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Display for InternedHash<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

impl<T: ?Sized + Eq + std::hash::Hash> Pointer for InternedHash<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Pointer::fmt(&(&**self as *const T), f)
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use crate::OrdInterner;

    #[test]
    fn pointer() {
        let interner = OrdInterner::new();
        let i = interner.intern_sized(42);
        let i2 = i.clone();
        assert_eq!(format!("{:p}", i), format!("{:p}", i2));
    }
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
            let i = HashInterner::new();
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
            let i = HashInterner::new();
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
            let i = HashInterner::new();
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
    fn drop_against_intern_and_interner() {
        model(|| {
            let i = HashInterner::new();
            let i2 = Arc::downgrade(&i.inner);

            let n = i.intern_box(42.into());
            let ii = i.clone();

            let h1 = spawn(move || drop(n));
            let h2 = spawn(move || i.intern_box(42.into()));
            let h3 = spawn(move || drop(ii));

            h1.join().unwrap();
            h2.join().unwrap();
            h3.join().unwrap();

            assert_eq!(counts(i2), (0, 1));
        })
    }
}
