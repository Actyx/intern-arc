/*
 * Copyright 2020 Actyx AG
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
//! Library for interning based on stdlib Arc
//!
//! The design of the hash-based half is taken from the [`arc-interner`](https://docs.rs/arc-interner)
//! crate, with the addition of another half based on [`BTreeMap`](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html).
//! Local benchmarks have shown that hashing is faster for objects below 1kB while tree traversal
//! and comparisons are faster above 1kB (very roughly and broadly speaking).
//!
//! The main functions exist in three variants:
//!
//!  - `intern_hash` and friends for when you want to use hashing (or have no `Ord` instance at hand)
//!  - `intern_tree` and friends for when you want to use tree map (or have no `Hash` instance at hand)
//!  - `intern` and friends to automatically choose based on object size
//!
//! Within each of these classes, four function exist to ingest data in various forms:
//!
//! ```
//! use std::sync::Arc;
//! use intern_arc::{intern, intern_unsized, intern_boxed, intern_arc};
//!
//! // for sized types
//! let a1: Arc<String> = intern("hello".to_owned());
//!
//! // for unsized non-owned types
//! let a2: Arc<str> = intern_unsized("hello");
//!
//! // for unsized owned types
//! let a3: Arc<str> = intern_boxed(Box::<str>::from("hello"));
//!
//! // for types with shared ownership
//! let a4: Arc<str> = intern_arc(Arc::<str>::from("hello"));
//! ```
//!
//! # Introspection
//!
//! This library offers some utilities for checking how well it works for a given use-case:
//!
//! ```
//! use std::sync::Arc;
//! use intern_arc::{inspect_hash, num_objects_interned_hash, types_interned};
//!
//! println!("str: {} objects", num_objects_interned_hash::<str>());
//!
//! let (hash, tree) = types_interned();
//! println!("types interned: {} with hashing, {} with trees", hash, tree);
//!
//! inspect_hash::<[u8], _, _>(|iter| {
//!     for arc in iter {
//!         println!("{} x {:?}", Arc::strong_count(&arc), arc);
//!     }
//! });
//! ```
//!
//! All function exist also with `tree` suffix instead of `hash`.
#![doc(html_logo_url = "https://developer.actyx.com/img/logo.svg")]
#![doc(html_favicon_url = "https://developer.actyx.com/img/favicon.ico")]

use dashmap::DashMap;
use once_cell::sync::OnceCell;
use std::any::{Any, TypeId};
use std::hash::Hash;
use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::{
        atomic::{AtomicIsize, Ordering},
        Arc, Mutex,
    },
};

struct HashContainer<T: ?Sized> {
    hashed: DashMap<Arc<T>, ()>,
    h_count: AtomicIsize,
}

impl<T: Eq + Hash + ?Sized> HashContainer<T> {
    pub fn new() -> Self {
        HashContainer {
            hashed: DashMap::new(),
            h_count: AtomicIsize::new(1),
        }
    }
}

struct TreeContainer<T: ?Sized> {
    tree: Mutex<BTreeMap<Arc<T>, ()>>,
    t_count: AtomicIsize,
}

impl<T: Ord + ?Sized> TreeContainer<T> {
    pub fn new() -> Self {
        TreeContainer {
            tree: Mutex::new(BTreeMap::new()),
            t_count: AtomicIsize::new(1),
        }
    }
}

static CONTAINER_HASH: OnceCell<DashMap<TypeId, Box<dyn Any + Send + Sync>>> = OnceCell::new();
static CONTAINER_TREE: OnceCell<DashMap<TypeId, Box<dyn Any + Send + Sync>>> = OnceCell::new();

/// Intern a shared-ownership reference using hashing (will not clone)
///
/// Returns either the given Arc or an already-interned one pointing to an equivalent value.
pub fn intern_hash_arc<T>(val: Arc<T>) -> Arc<T>
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

    // maintenance
    if m.h_count.fetch_sub(1, Ordering::Relaxed) == 0 {
        janitor_h(m);
    }

    ret
}

/// Intern an owned value using hashing (will not clone)
pub fn intern_hash<T>(val: T) -> Arc<T>
where
    T: Eq + Hash + Send + Sync + 'static,
{
    intern_hash_arc(Arc::new(val))
}

/// Intern a non-owned reference using hashing (will clone)
pub fn intern_hash_unsized<T>(val: &T) -> Arc<T>
where
    T: Eq + Hash + Send + Sync + ?Sized + 'static,
    Arc<T>: for<'a> From<&'a T>,
{
    intern_hash_arc(Arc::from(val))
}

/// Intern an owned reference using hashing (will not clone)
pub fn intern_hash_boxed<T>(val: Box<T>) -> Arc<T>
where
    T: Eq + Hash + Send + Sync + ?Sized + 'static,
{
    intern_hash_arc(Arc::from(val))
}

/// Intern a shared-ownership reference using tree map (will not clone)
///
/// Returns either the given Arc or an already-interned one pointing to an equivalent value.
pub fn intern_tree_arc<T>(val: Arc<T>) -> Arc<T>
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

    // maintenance
    if m.t_count.fetch_sub(1, Ordering::Relaxed) == 0 {
        janitor_t(&mut *set, &m.t_count);
    }

    ret
}

/// Intern an owned value using tree map (will not clone)
pub fn intern_tree<T>(val: T) -> Arc<T>
where
    T: Ord + Send + Sync + 'static,
{
    intern_tree_arc(Arc::new(val))
}

/// Intern a non-owned reference using tree map (will clone)
pub fn intern_tree_unsized<T>(val: &T) -> Arc<T>
where
    T: Ord + Send + Sync + ?Sized + 'static,
    Arc<T>: for<'a> From<&'a T>,
{
    intern_tree_arc(Arc::from(val))
}

/// Intern an owned reference using tree map (will not clone)
pub fn intern_tree_boxed<T>(val: Box<T>) -> Arc<T>
where
    T: Ord + Send + Sync + ?Sized + 'static,
{
    intern_tree_arc(Arc::from(val))
}

/// Intern an owned value (will not clone)
pub fn intern<T>(val: T) -> Arc<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
{
    if std::mem::size_of::<T>() > 1000 {
        intern_tree(val)
    } else {
        intern_hash(val)
    }
}

/// Intern a non-owned reference (will clone)
pub fn intern_unsized<T: ?Sized>(val: &T) -> Arc<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
    Arc<T>: for<'a> From<&'a T>,
{
    if std::mem::size_of_val(val) > 1000 {
        intern_tree_unsized(val)
    } else {
        intern_hash_unsized(val)
    }
}

/// Intern an owned reference (will not clone)
pub fn intern_boxed<T: ?Sized>(val: Box<T>) -> Arc<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
{
    if std::mem::size_of_val(val.as_ref()) > 1000 {
        intern_tree_boxed(val)
    } else {
        intern_hash_boxed(val)
    }
}

/// Intern a shared-ownership reference (will not clone)
///
/// Returns either the given Arc or an already-interned one pointing to an equivalent value.
pub fn intern_arc<T: ?Sized>(val: Arc<T>) -> Arc<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
{
    if std::mem::size_of_val(val.as_ref()) > 1000 {
        intern_tree_arc(val)
    } else {
        intern_hash_arc(val)
    }
}

/// Perform internal maintenance (removing otherwise unreferenced elements) and return count of elements
pub fn num_objects_interned_hash<T: Eq + Hash + ?Sized + 'static>() -> usize {
    if let Some(m) = CONTAINER_HASH
        .get()
        .and_then(|type_map| type_map.get(&TypeId::of::<T>()))
    {
        let m = m.downcast_ref::<HashContainer<T>>().unwrap();
        janitor_h(m);
        m.hashed.len()
    } else {
        0
    }
}

/// Perform internal maintenance (removing otherwise unreferenced elements) and return count of elements
pub fn num_objects_interned_tree<T: Ord + ?Sized + 'static>() -> usize {
    if let Some(m) = CONTAINER_TREE
        .get()
        .and_then(|type_map| type_map.get(&TypeId::of::<T>()))
    {
        let m = m.downcast_ref::<TreeContainer<T>>().unwrap();
        let mut s = m.tree.lock().unwrap();
        janitor_t(&mut *s, &m.t_count);
        s.len()
    } else {
        0
    }
}

/// Number of different types (TypeIds) interned using hashing and tree-based, respectively
pub fn types_interned() -> (usize, usize) {
    (
        CONTAINER_HASH.get().map(|m| m.len()).unwrap_or_default(),
        CONTAINER_TREE.get().map(|m| m.len()).unwrap_or_default(),
    )
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

fn janitor_h<T: Eq + Hash + ?Sized + 'static>(m: &HashContainer<T>) {
    let before = m.hashed.len();
    m.hashed.retain(|k, _v| Arc::strong_count(k) > 1);
    let after = m.hashed.len();
    let removed = (before as isize - after as isize).max(1) as usize;
    // assume removals are always possible
    // the interval is tuned such that it is very short for high churn and very long for low churn
    // this is done such that the amortized cost is one retain check per insert
    m.h_count
        .store((before / removed) as isize, Ordering::Relaxed);
}

fn janitor_t<T: Ord + ?Sized + 'static>(set: &mut BTreeMap<Arc<T>, ()>, count: &AtomicIsize) {
    let before = set.len();
    let to_remove = set
        .iter()
        .filter_map(|(k, _v)| {
            if Arc::strong_count(k) == 1 {
                Some(k.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for a in to_remove {
        set.remove(&a);
    }
    let after = set.len();
    let removed = (before - after).max(1);
    // assume removals are always possible
    // the interval is tuned such that it is very short for high churn and very long for low churn
    // this is done such that the amortized cost is one retain check per insert
    count.store((before / removed) as isize, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::thread;

    // Test basic functionality.
    #[test]
    fn basic_hash() {
        assert_eq!(intern_hash("foo"), intern_hash("foo"));
        assert_ne!(intern_hash("foo"), intern_hash("bar"));
        // The above refs should be deallocate by now.
        assert_eq!(num_objects_interned_hash::<&str>(), 0);

        let _interned1 = intern_hash("foo".to_string());
        {
            let interned2 = intern_hash("foo".to_string());
            let interned3 = intern_hash("bar".to_string());

            assert_eq!(Arc::strong_count(&interned2), 3);
            assert_eq!(Arc::strong_count(&interned3), 2);
            // We now have two unique interned strings: "foo" and "bar".
            assert_eq!(num_objects_interned_hash::<String>(), 2);
        }

        // "bar" is now gone.
        assert_eq!(num_objects_interned_hash::<String>(), 1);
    }

    // Test basic functionality.
    #[test]
    fn basic_hash_unsized() {
        assert_eq!(intern_hash_unsized("foo"), intern_hash_unsized("foo"));
        assert_ne!(intern_hash_unsized("foo"), intern_hash_unsized("bar"));
        // The above refs should be deallocate by now.
        assert_eq!(num_objects_interned_hash::<str>(), 0);

        let _interned1 = intern_hash_unsized("foo");
        {
            let interned2 = intern_hash_unsized("foo");
            let interned3 = intern_hash_unsized("bar");

            assert_eq!(Arc::strong_count(&interned2), 3);
            assert_eq!(Arc::strong_count(&interned3), 2);
            // We now have two unique interned strings: "foo" and "bar".
            assert_eq!(num_objects_interned_hash::<str>(), 2);
        }

        // "bar" is now gone.
        assert_eq!(num_objects_interned_hash::<str>(), 1);
    }

    // Test basic functionality.
    #[test]
    fn basic_tree() {
        assert_eq!(intern_tree("foo"), intern_tree("foo"));
        assert_ne!(intern_tree("foo"), intern_tree("bar"));
        // The above refs should be deallocate by now.
        assert_eq!(num_objects_interned_tree::<&str>(), 0);

        let _interned1 = intern_tree("foo".to_string());
        {
            let interned2 = intern_tree("foo".to_string());
            let interned3 = intern_tree("bar".to_string());

            assert_eq!(Arc::strong_count(&interned2), 3);
            assert_eq!(Arc::strong_count(&interned3), 2);
            // We now have two unique interned strings: "foo" and "bar".
            assert_eq!(num_objects_interned_tree::<String>(), 2);
        }

        // "bar" is now gone.
        assert_eq!(num_objects_interned_tree::<String>(), 1);
    }

    // Test basic functionality.
    #[test]
    fn basic_tree_unsized() {
        assert_eq!(intern_tree_unsized("foo"), intern_tree_unsized("foo"));
        assert_ne!(intern_tree_unsized("foo"), intern_tree_unsized("bar"));
        // The above refs should be deallocate by now.
        assert_eq!(num_objects_interned_tree::<str>(), 0);

        let _interned1 = intern_tree_unsized("foo");
        {
            let interned2 = intern_tree_unsized("foo");
            let interned3 = intern_tree_unsized("bar");

            assert_eq!(Arc::strong_count(&interned2), 3);
            assert_eq!(Arc::strong_count(&interned3), 2);
            // We now have two unique interned strings: "foo" and "bar".
            assert_eq!(num_objects_interned_tree::<str>(), 2);
        }

        // "bar" is now gone.
        assert_eq!(num_objects_interned_tree::<str>(), 1);
    }

    // Ordering should be based on values, not pointers.
    // Also tests `Display` implementation.
    #[test]
    fn sorting() {
        let mut interned_vals = vec![
            intern_hash(4),
            intern_hash(2),
            intern_hash(5),
            intern_hash(0),
            intern_hash(1),
            intern_hash(3),
        ];
        interned_vals.sort();
        let sorted: Vec<String> = interned_vals.iter().map(|v| format!("{}", v)).collect();
        assert_eq!(&sorted.join(","), "0,1,2,3,4,5");
    }

    #[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct TestStruct2(String, u64);

    #[test]
    fn sequential() {
        for _i in 0..10_000 {
            let mut interned = Vec::with_capacity(100);
            for j in 0..100 {
                interned.push(intern_hash(TestStruct2("foo".to_string(), j)));
            }
        }

        assert_eq!(num_objects_interned_hash::<TestStruct2>(), 0);
    }

    #[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
    pub struct TestStruct(String, u64);

    // Quickly create and destroy a small number of interned objects from
    // multiple threads.
    #[test]
    fn multithreading1() {
        let mut thandles = vec![];
        for _i in 0..10 {
            thandles.push(thread::spawn(|| {
                for _i in 0..100_000 {
                    let _interned1 = intern_hash(TestStruct("foo".to_string(), 5));
                    let _interned2 = intern_hash(TestStruct("bar".to_string(), 10));
                }
            }));
        }
        for h in thandles.into_iter() {
            h.join().unwrap()
        }

        assert_eq!(num_objects_interned_hash::<TestStruct>(), 0);
    }
}
