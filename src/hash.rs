use std::{
    hash::Hash,
    sync::{Arc, Weak},
};

use dashmap::DashMap;

use crate::ref_count::{Interned, RemovalResult, RemovePtr};

pub struct InternHash<T: ?Sized> {
    inner: Arc<Inner<T>>,
}

impl<T: ?Sized> Clone for InternHash<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[repr(C)]
struct Inner<T: ?Sized> {
    remove_if_last: RemovePtr<T>,
    map: DashMap<Interned<T>, ()>,
}

unsafe impl<T: ?Sized + Sync + Send> Send for Inner<T> {}
unsafe impl<T: ?Sized + Sync + Send> Sync for Inner<T> {}

fn remove_if_last<T: ?Sized + Eq + Hash>(this: *const (), key: &Interned<T>) -> RemovalResult {
    // this is safe because we’re still holding a weak reference: the value may be dropped
    // but the ArcInner is still alive!
    let weak = unsafe { Weak::from_raw(this as *const Inner<T>) };
    let success = match weak.upgrade() {
        Some(strong) => strong
            .map
            .remove_if(key, |k, _| {
                let r = k.ref_count();
                #[cfg(feature = "println")]
                println!("refs {} {:p}", r, key);
                r == 2
            })
            .is_some(),
        None => {
            // This means that the last strong reference has started being dropped, so the map will be cleared.
            // We drop the weak count because we won’t need to contact this interner anymore
            drop(weak);
            return RemovalResult::MapGone;
        }
    };
    if success {
        // need to decrement this Arc’s weak count — can’t be done by Interned::Drop since it cannot know this type
        drop(weak);
        RemovalResult::Removed
    } else {
        // otherwise we need to keep the weak count because the caller will not drop the `this` pointer
        // I prefer into_raw over forget because it clearly tells the Weak what is happening
        Weak::into_raw(weak);
        RemovalResult::NotRemoved
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
impl<T: ?Sized + Eq + Hash> InternHash<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                remove_if_last,
                map: DashMap::new(),
            }),
        }
    }

    /// Returns the number of objects currently kept in this interner.
    pub fn len(&self) -> usize {
        self.inner.map.len()
    }

    /// Returns `true` when this interner doesn’t hold any values.
    pub fn is_empty(&self) -> bool {
        self.inner.map.is_empty()
    }

    fn intern(&self, interned: Interned<T>) -> Interned<T> {
        let entry = self.inner.map.entry(interned).or_insert(());
        let mut ret = entry.key().clone();
        drop(entry);
        let me = Weak::into_raw(Arc::downgrade(&self.inner));
        if !ret.make_hot(me as *mut RemovePtr<T>) {
            // lost the race to install the weak reference, so we must properly drop it here
            drop(unsafe { Weak::from_raw(me) });
        }
        ret
    }

    /// Intern a value from a shared reference by allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{InternHash, Interned};
    ///
    /// let strings = InternHash::<str>::new();
    /// let i: Interned<str> = strings.intern_ref("hello world!");
    /// ```
    pub fn intern_ref(&self, value: &T) -> Interned<T>
    where
        T: ToOwned,
        T::Owned: Into<Box<T>>,
    {
        if let Some(entry) = self.inner.map.get(value) {
            return entry.key().clone();
        }
        self.intern(Interned::from_box(value.to_owned().into()))
    }

    /// Intern a value from an owned reference without allocating new memory for it.
    ///
    /// ```
    /// use intern_arc::{InternHash, Interned};
    ///
    /// let strings = InternHash::<str>::new();
    /// let hello: Box<str> = "hello world!".into();
    /// let i: Interned<str> = strings.intern_box(hello);
    /// ```
    /// (This also works nicely with a `String` that can be turned `.into()` a `Box`.)
    pub fn intern_box(&self, value: Box<T>) -> Interned<T> {
        if let Some(entry) = self.inner.map.get(value.as_ref()) {
            return entry.key().clone();
        }
        self.intern(Interned::from_box(value))
    }

    /// Intern a sized value, allocating heap memory for it.
    ///
    /// ```
    /// use intern_arc::{InternHash, Interned};
    ///
    /// let arrays = InternHash::<[u8; 1000]>::new();
    /// let i: Interned<[u8; 1000]> = arrays.intern_sized([0; 1000]);
    pub fn intern_sized(&self, value: T) -> Interned<T>
    where
        T: Sized,
    {
        if let Some(entry) = self.inner.map.get(&value) {
            return entry.key().clone();
        }
        self.intern(Interned::from_sized(value))
    }
}

impl<T: ?Sized + Eq + Hash> Default for InternHash<T> {
    fn default() -> Self {
        Self::new()
    }
}
