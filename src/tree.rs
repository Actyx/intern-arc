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
use crate::ref_count::{Interned, RemovePtr};
use parking_lot::{Mutex, MutexGuard};
use std::{
    collections::BTreeSet,
    sync::{Arc, Weak},
};

pub struct InternOrd<T: ?Sized> {
    inner: Arc<Inner<T>>,
}

impl<T: ?Sized> Clone for InternOrd<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[repr(C)]
struct Inner<T: ?Sized> {
    remover: RemovePtr<T>,
    map: Mutex<BTreeSet<Interned<T>>>,
}

unsafe impl<T: ?Sized + Sync + Send> Send for Inner<T> {}
unsafe impl<T: ?Sized + Sync + Send> Sync for Inner<T> {}

fn remover<T: ?Sized + Ord>(this: *const (), key: *const Interned<T>) {
    // this is safe because we’re still holding a weak reference: the value may be dropped
    // but the ArcInner is still alive!
    let weak = unsafe { Weak::from_raw(this as *const Inner<T>) };
    if let Some(strong) = weak.upgrade() {
        let mut map = strong.map.lock();
        // Please see Interned::drop() for an explanation why `key` is safe in this case
        let _value = map.take(unsafe { &*key });
        // drop the lock before dropping the value, in case the value has Drop glue that needs
        // this lock
        drop(map);
    }
}

/// Interner for values with a total order
///
/// The interner is cheaply cloneable by virtue of keeping the underlying storage
/// in an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html). Once the last
/// reference to this interner is dropped, it will clear its backing storage and
/// release all references to the interned values it has created that are still live.
/// Those values remain fully operational until dropped. Memory for the values
/// themselves is freed for each value individually once its last reference is dropped.
impl<T: ?Sized + Ord> InternOrd<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                remover,
                map: Mutex::new(BTreeSet::new()),
            }),
        }
    }

    fn map(&self) -> MutexGuard<BTreeSet<Interned<T>>> {
        self.inner.map.lock()
    }

    /// Returns the number of objects currently kept in this interner.
    pub fn len(&self) -> usize {
        self.map().len()
    }

    /// Returns `true` when this interner doesn’t hold any values.
    pub fn is_empty(&self) -> bool {
        self.map().is_empty()
    }

    fn intern(
        &self,
        mut map: MutexGuard<BTreeSet<Interned<T>>>,
        interned: Interned<T>,
    ) -> Interned<T> {
        let mut ret = interned.clone();
        map.insert(interned);
        let me = Weak::into_raw(Arc::downgrade(&self.inner));
        ret.make_hot(me as *mut RemovePtr<T>);
        ret
    }

    /// Intern a value from a shared reference by allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{InternOrd, Interned};
    ///
    /// let strings = InternOrd::<str>::new();
    /// let i: Interned<str> = strings.intern_ref("hello world!");
    /// ```
    pub fn intern_ref(&self, value: &T) -> Interned<T>
    where
        T: ToOwned,
        T::Owned: Into<Box<T>>,
    {
        let map = self.map();
        if let Some(entry) = map.get(value) {
            return entry.clone();
        }
        self.intern(map, Interned::from_box(value.to_owned().into()))
    }

    /// Intern a value from an owned reference without allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{InternOrd, Interned};
    ///
    /// let strings = InternOrd::<str>::new();
    /// let hello: Box<str> = "hello world!".into();
    /// let i: Interned<str> = strings.intern_box(hello);
    /// ```
    /// (This also works nicely with a `String` that can be turned `.into()` a `Box`.)
    pub fn intern_box(&self, value: Box<T>) -> Interned<T> {
        let map = self.map();
        if let Some(entry) = map.get(value.as_ref()) {
            return entry.clone();
        }
        self.intern(map, Interned::from_box(value))
    }

    /// Intern a sized value, allocating heap memory for it.
    ///
    /// ```
    /// use intern_arc::{InternOrd, Interned};
    ///
    /// let arrays = InternOrd::<[u8; 1000]>::new();
    /// let i: Interned<[u8; 1000]> = arrays.intern_sized([0; 1000]);
    pub fn intern_sized(&self, value: T) -> Interned<T>
    where
        T: Sized,
    {
        let map = self.map();
        if let Some(entry) = map.get(&value) {
            return entry.clone();
        }
        self.intern(map, Interned::from_sized(value))
    }
}

impl<T: ?Sized + Ord> Default for InternOrd<T> {
    fn default() -> Self {
        Self::new()
    }
}
