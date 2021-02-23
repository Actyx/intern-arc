use dashmap::DashMap;
use std::{
    any::TypeId,
    collections::{btree_map::Entry, BTreeMap},
    fmt::Display,
    ops::Deref,
    sync::{Arc, Mutex},
};

use crate::CONTAINER_TREE;

struct TreeContainer<T: ?Sized> {
    tree: Mutex<BTreeMap<Arc<T>, ()>>,
}

impl<T: Ord + ?Sized> TreeContainer<T> {
    pub fn new() -> Self {
        TreeContainer {
            tree: Mutex::new(BTreeMap::new()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct InternedTree<T: ?Sized + Ord + 'static>(Arc<T>);

impl<T: ?Sized + Ord + 'static> InternedTree<T> {
    pub fn references(this: &Self) -> usize {
        Arc::strong_count(&this.0)
    }
}

impl<T: ?Sized + Ord + 'static + Display> Display for InternedTree<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: ?Sized + Ord + 'static> Deref for InternedTree<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: ?Sized + Ord + 'static> Drop for InternedTree<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 2 {
            if let Some(boxed) = CONTAINER_TREE
                .get_or_init(DashMap::new)
                .get(&TypeId::of::<T>())
            {
                let m = boxed.value().downcast_ref::<TreeContainer<T>>().unwrap();
                let mut set = m.tree.lock().unwrap();
                if Arc::strong_count(&self.0) == 2 {
                    set.remove(&self.0);
                }
            }
        }
    }
}

/// Intern a shared-ownership reference using tree map (will not clone)
///
/// Returns either the given Arc or an already-interned one pointing to an equivalent value.
pub fn intern_tree_arc<T>(val: Arc<T>) -> InternedTree<T>
where
    T: Ord + Send + Sync + ?Sized + 'static,
{
    let type_map = CONTAINER_TREE.get_or_init(DashMap::new);

    // Prefer taking the read lock to reduce contention, only use entry api if necessary.
    let boxed = if let Some(boxed) = type_map.get(&TypeId::of::<T>()) {
        boxed
    } else {
        type_map
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(TreeContainer::<T>::new()))
            .downgrade()
    };

    let m: &TreeContainer<T> = boxed.value().downcast_ref::<TreeContainer<T>>().unwrap();
    let mut set = m.tree.lock().unwrap();
    let ret = match set.entry(val) {
        Entry::Vacant(x) => {
            let ret = x.key().clone();
            x.insert(());
            ret
        }
        Entry::Occupied(x) => x.key().clone(),
    };

    InternedTree(ret)
}

/// Perform internal maintenance (removing otherwise unreferenced elements) and return count of elements
pub fn num_objects_interned_tree<T: Ord + ?Sized + 'static>() -> usize {
    if let Some(m) = CONTAINER_TREE
        .get()
        .and_then(|type_map| type_map.get(&TypeId::of::<T>()))
    {
        let m = m.downcast_ref::<TreeContainer<T>>().unwrap();
        let s = m.tree.lock().unwrap();
        s.len()
    } else {
        0
    }
}

/// Feed an iterator over all interned values for the given type to the given function
pub fn inspect_tree<T, F, U>(f: F) -> U
where
    T: Ord + ?Sized + 'static,
    F: for<'a> FnOnce(Box<dyn Iterator<Item = Arc<T>> + 'a>) -> U,
{
    let o = CONTAINER_TREE
        .get()
        .and_then(|type_map| type_map.get(&TypeId::of::<T>()));
    if let Some(r) = o {
        let m = r.downcast_ref::<TreeContainer<T>>().unwrap();
        let map = m.tree.lock().unwrap();
        f(Box::new(map.iter().map(|(k, _v)| k.clone())))
    } else {
        f(Box::new(std::iter::empty()))
    }
}
