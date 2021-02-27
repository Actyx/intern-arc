use std::{
    hash::Hash,
    sync::{Arc, Weak},
};

use dashmap::DashMap;

use crate::ref_count::{Interned, RemovalResult, RemovePtr};

#[derive(Clone)]
pub struct InternHash<T: ?Sized> {
    inner: Arc<Inner<T>>,
}

#[repr(C)]
struct Inner<T: ?Sized> {
    remove_if_last: RemovePtr<T>,
    map: DashMap<Interned<T>, ()>,
}

fn remove_if_last<T: ?Sized + Eq + Hash>(this: *const (), key: &Interned<T>) -> RemovalResult {
    // this is safe because we’re still holding a weak reference: the value may be dropped
    // but the ArcInner is still alive!
    let weak = unsafe { Weak::from_raw(this as *const Inner<T>) };
    let success = match weak.upgrade() {
        Some(strong) => strong
            .map
            .remove_if(key, |k, _| k.ref_count() == 2)
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

impl<T: ?Sized + Eq + Hash> InternHash<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                remove_if_last,
                map: DashMap::new(),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.map.is_empty()
    }

    fn intern(&self, interned: Interned<T>) -> Interned<T> {
        let entry = self.inner.map.entry(interned).or_insert(());
        let mut ret = entry.key().clone();
        let me = Weak::into_raw(Arc::downgrade(&self.inner));
        ret.make_hot(me as *mut RemovePtr<T>);
        ret
    }

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

    pub fn intern_box(&self, value: Box<T>) -> Interned<T> {
        if let Some(entry) = self.inner.map.get(value.as_ref()) {
            return entry.key().clone();
        }
        self.intern(Interned::from_box(value))
    }

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
