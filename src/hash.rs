use std::{any::TypeId, fmt::Display, hash::Hash, ops::Deref, sync::Arc};

use dashmap::DashMap;

use crate::CONTAINER_HASH;

struct HashContainer<T: ?Sized> {
    hashed: DashMap<Arc<T>, ()>,
}

impl<T: Eq + Hash + ?Sized> HashContainer<T> {
    pub fn new() -> Self {
        HashContainer {
            hashed: DashMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct InternedHash<T: ?Sized + Eq + Hash + 'static>(Arc<T>);

impl<T: ?Sized + Eq + Hash + 'static> InternedHash<T> {
    pub fn references(this: &Self) -> usize {
        Arc::strong_count(&this.0)
    }
}

impl<T: ?Sized + Eq + Hash + 'static + Display> Display for InternedHash<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: ?Sized + Eq + Hash + 'static> Deref for InternedHash<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: ?Sized + Eq + Hash + 'static> Drop for InternedHash<T> {
    fn drop(&mut self) {
        // one reference from this Interned, one from the map
        if Arc::strong_count(&self.0) == 2 {
            if let Some(boxed) = CONTAINER_HASH
                .get_or_init(DashMap::new)
                .get(&TypeId::of::<T>())
            {
                let m = boxed.value().downcast_ref::<HashContainer<T>>().unwrap();
                m.hashed
                    .remove_if(&self.0, |k, _v| Arc::strong_count(k) == 2);
            }
        }
    }
}

/// Intern a shared-ownership reference using hashing (will not clone)
///
/// Returns either the given Arc or an already-interned one pointing to an equivalent value.
pub fn intern_hash_arc<T>(val: Arc<T>) -> InternedHash<T>
where
    T: Eq + Hash + Send + Sync + ?Sized + 'static,
{
    let type_map = CONTAINER_HASH.get_or_init(DashMap::new);

    // Prefer taking the read lock to reduce contention, only use entry api if necessary.
    let boxed = if let Some(boxed) = type_map.get(&TypeId::of::<T>()) {
        boxed
    } else {
        type_map
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(HashContainer::<T>::new()))
            .downgrade()
    };

    let m: &HashContainer<T> = boxed.value().downcast_ref::<HashContainer<T>>().unwrap();
    let b = m.hashed.entry(val).or_insert(());
    let ret = b.key().clone();
    drop(b);

    InternedHash(ret)
}

/// Perform internal maintenance (removing otherwise unreferenced elements) and return count of elements
pub fn num_objects_interned_hash<T: Eq + Hash + ?Sized + 'static>() -> usize {
    if let Some(m) = CONTAINER_HASH
        .get()
        .and_then(|type_map| type_map.get(&TypeId::of::<T>()))
    {
        let m = m.downcast_ref::<HashContainer<T>>().unwrap();
        m.hashed.len()
    } else {
        0
    }
}

/// Feed an iterator over all interned values for the given type to the given function
pub fn inspect_hash<T, F, U>(f: F) -> U
where
    T: Eq + Hash + ?Sized + 'static,
    F: for<'a> FnOnce(Box<dyn Iterator<Item = Arc<T>> + 'a>) -> U,
{
    let o = CONTAINER_HASH
        .get()
        .and_then(|type_map| type_map.get(&TypeId::of::<T>()));
    if let Some(r) = o {
        let m = r.downcast_ref::<HashContainer<T>>().unwrap();
        f(Box::new(m.hashed.iter().map(|r| r.key().clone())))
    } else {
        f(Box::new(std::iter::empty()))
    }
}
