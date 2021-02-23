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
//! use intern_arc::{intern, intern_unsized, intern_boxed, intern_arc, Interned};
//!
//! // for sized types
//! let a1: Interned<String> = intern("hello".to_owned());
//!
//! // for unsized non-owned types
//! let a2: Interned<str> = intern_unsized("hello");
//!
//! // for unsized owned types
//! let a3: Interned<str> = intern_boxed(Box::<str>::from("hello"));
//!
//! // for types with shared ownership
//! let a4: Interned<str> = intern_arc(Arc::<str>::from("hello"));
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
//! All functions exist also with `tree` suffix instead of `hash`.
#![doc(html_logo_url = "https://developer.actyx.com/img/logo.svg")]
#![doc(html_favicon_url = "https://developer.actyx.com/img/favicon.ico")]

use dashmap::DashMap;
use once_cell::sync::OnceCell;
use std::hash::Hash;
use std::sync::Arc;
use std::{
    any::{Any, TypeId},
    fmt::Display,
    ops::Deref,
};

mod hash;
mod tree;

pub use hash::{inspect_hash, intern_hash_arc, num_objects_interned_hash, InternedHash};
pub use tree::{inspect_tree, intern_tree_arc, num_objects_interned_tree, InternedTree};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Interned<T: ?Sized + Eq + Hash + Ord + 'static> {
    Hash(InternedHash<T>),
    Tree(InternedTree<T>),
}

impl<T: ?Sized + Eq + Hash + Ord + 'static> Interned<T> {
    pub fn references(this: &Self) -> usize {
        match this {
            Interned::Hash(h) => InternedHash::references(h),
            Interned::Tree(t) => InternedTree::references(t),
        }
    }
}

impl<T: ?Sized + Eq + Hash + Ord + 'static + Display> Display for Interned<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Interned::Hash(h) => h.fmt(f),
            Interned::Tree(t) => t.fmt(f),
        }
    }
}

impl<T: ?Sized + Eq + Hash + Ord + 'static> Deref for Interned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            Interned::Hash(h) => h.deref(),
            Interned::Tree(t) => t.deref(),
        }
    }
}

static CONTAINER_HASH: OnceCell<DashMap<TypeId, Box<dyn Any + Send + Sync>>> = OnceCell::new();
static CONTAINER_TREE: OnceCell<DashMap<TypeId, Box<dyn Any + Send + Sync>>> = OnceCell::new();

/// Intern an owned value using hashing (will not clone)
pub fn intern_hash<T>(val: T) -> InternedHash<T>
where
    T: Eq + Hash + Send + Sync + 'static,
{
    intern_hash_arc(Arc::new(val))
}

/// Intern a non-owned reference using hashing (will clone)
pub fn intern_hash_unsized<T>(val: &T) -> InternedHash<T>
where
    T: Eq + Hash + Send + Sync + ?Sized + 'static,
    Arc<T>: for<'a> From<&'a T>,
{
    intern_hash_arc(Arc::from(val))
}

/// Intern an owned reference using hashing (will not clone)
pub fn intern_hash_boxed<T>(val: Box<T>) -> InternedHash<T>
where
    T: Eq + Hash + Send + Sync + ?Sized + 'static,
{
    intern_hash_arc(Arc::from(val))
}

/// Intern an owned value using tree map (will not clone)
pub fn intern_tree<T>(val: T) -> InternedTree<T>
where
    T: Ord + Send + Sync + 'static,
{
    intern_tree_arc(Arc::new(val))
}

/// Intern a non-owned reference using tree map (will clone)
pub fn intern_tree_unsized<T>(val: &T) -> InternedTree<T>
where
    T: Ord + Send + Sync + ?Sized + 'static,
    Arc<T>: for<'a> From<&'a T>,
{
    intern_tree_arc(Arc::from(val))
}

/// Intern an owned reference using tree map (will not clone)
pub fn intern_tree_boxed<T>(val: Box<T>) -> InternedTree<T>
where
    T: Ord + Send + Sync + ?Sized + 'static,
{
    intern_tree_arc(Arc::from(val))
}

/// Intern an owned value (will not clone)
pub fn intern<T>(val: T) -> Interned<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
{
    if std::mem::size_of::<T>() > 1000 {
        Interned::Tree(intern_tree(val))
    } else {
        Interned::Hash(intern_hash(val))
    }
}

/// Intern a non-owned reference (will clone)
pub fn intern_unsized<T: ?Sized>(val: &T) -> Interned<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
    Arc<T>: for<'a> From<&'a T>,
{
    if std::mem::size_of_val(val) > 1000 {
        Interned::Tree(intern_tree_unsized(val))
    } else {
        Interned::Hash(intern_hash_unsized(val))
    }
}

/// Intern an owned reference (will not clone)
pub fn intern_boxed<T: ?Sized>(val: Box<T>) -> Interned<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
{
    if std::mem::size_of_val(val.as_ref()) > 1000 {
        Interned::Tree(intern_tree_boxed(val))
    } else {
        Interned::Hash(intern_hash_boxed(val))
    }
}

/// Intern a shared-ownership reference (will not clone)
///
/// Returns either the given Arc or an already-interned one pointing to an equivalent value.
pub fn intern_arc<T: ?Sized>(val: Arc<T>) -> Interned<T>
where
    T: Eq + Hash + Ord + Send + Sync + 'static,
{
    if std::mem::size_of_val(val.as_ref()) > 1000 {
        Interned::Tree(intern_tree_arc(val))
    } else {
        Interned::Hash(intern_hash_arc(val))
    }
}

/// Number of different types (TypeIds) interned using hashing and tree-based, respectively
pub fn types_interned() -> (usize, usize) {
    (
        CONTAINER_HASH.get().map(|m| m.len()).unwrap_or_default(),
        CONTAINER_TREE.get().map(|m| m.len()).unwrap_or_default(),
    )
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
        // The above refs should be deallocated by now.
        assert_eq!(num_objects_interned_hash::<&str>(), 0);

        let _interned1 = intern_hash("foo".to_string());
        {
            let interned2 = intern_hash("foo".to_string());
            let interned3 = intern_hash("bar".to_string());

            assert_eq!(InternedHash::references(&interned2), 3);
            assert_eq!(InternedHash::references(&interned3), 2);
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
        // The above refs should be deallocated by now.
        assert_eq!(num_objects_interned_hash::<str>(), 0);

        let _interned1 = intern_hash_unsized("foo");
        {
            let interned2 = intern_hash_unsized("foo");
            let interned3 = intern_hash_unsized("bar");

            assert_eq!(InternedHash::references(&interned2), 3);
            assert_eq!(InternedHash::references(&interned3), 2);
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
        // The above refs should be deallocated by now.
        assert_eq!(num_objects_interned_tree::<&str>(), 0);

        let _interned1 = intern_tree("foo".to_string());
        {
            let interned2 = intern_tree("foo".to_string());
            let interned3 = intern_tree("bar".to_string());

            assert_eq!(InternedTree::references(&interned2), 3);
            assert_eq!(InternedTree::references(&interned3), 2);
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
        // The above refs should be deallocated by now.
        assert_eq!(num_objects_interned_tree::<str>(), 0);

        let _interned1 = intern_tree_unsized("foo");
        {
            let interned2 = intern_tree_unsized("foo");
            let interned3 = intern_tree_unsized("bar");

            assert_eq!(InternedTree::references(&interned2), 3);
            assert_eq!(InternedTree::references(&interned3), 2);
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
