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

mod hash;
mod ref_count;
// mod tree;

pub use hash::InternHash;
pub use ref_count::Interned;
// pub use tree::InternedTree;

#[cfg(test)]
mod tests {
    use crate::*;
    //     use std::thread;

    // Test basic functionality.
    #[test]
    fn basic_hash() {
        let interner = InternHash::<&str>::new();

        assert_eq!(interner.intern_sized("foo"), interner.intern_sized("foo"));
        assert_ne!(interner.intern_sized("foo"), interner.intern_sized("bar"));
        // The above refs should be deallocated by now.
        assert_eq!(interner.len(), 0);

        let interner = InternHash::<String>::new();

        let interned1 = interner.intern_sized("foo".to_string());
        {
            let interned2 = interner.intern_sized("foo".to_string());
            let interned3 = interner.intern_sized("bar".to_string());

            assert_eq!(interned2.ref_count(), 3);
            assert_eq!(interned3.ref_count(), 2);
            // We now have two unique interned strings: "foo" and "bar".
            assert_eq!(interner.len(), 2);
        }

        // "bar" is now gone.
        assert_eq!(interner.len(), 1);

        drop(interner);
        assert_eq!(interned1.ref_count(), 1);
    }

    // Test basic functionality.
    #[test]
    fn basic_hash_unsized() {
        let interner = InternHash::<str>::new();

        assert_eq!(interner.intern_ref("foo"), interner.intern_ref("foo"));
        assert_ne!(interner.intern_ref("foo"), interner.intern_ref("bar"));
        // The above refs should be deallocated by now.
        assert_eq!(interner.len(), 0);

        let interned1 = interner.intern_ref("foo");
        {
            let interned2 = interner.intern_ref("foo");
            let interned3 = interner.intern_ref("bar");

            assert_eq!(interned2.ref_count(), 3);
            assert_eq!(interned3.ref_count(), 2);
            // We now have two unique interned strings: "foo" and "bar".
            assert_eq!(interner.len(), 2);
        }

        // "bar" is now gone.
        assert_eq!(interner.len(), 1);

        assert_eq!(
            &*interned1 as *const _,
            &*interner.intern_ref("foo") as *const _
        );

        drop(interner);

        assert_ne!(
            &*interned1 as *const _,
            &*InternHash::new().intern_ref("foo") as *const _
        );

        assert_eq!(interned1.ref_count(), 1);
    }

    //     // Test basic functionality.
    //     #[test]
    //     fn basic_tree() {
    //         assert_eq!(intern_tree("foo"), intern_tree("foo"));
    //         assert_ne!(intern_tree("foo"), intern_tree("bar"));
    //         // The above refs should be deallocated by now.
    //         assert_eq!(num_objects_interned_tree::<&str>(), 0);

    //         let _interned1 = intern_tree("foo".to_string());
    //         {
    //             let interned2 = intern_tree("foo".to_string());
    //             let interned3 = intern_tree("bar".to_string());

    //             assert_eq!(InternedTree::references(&interned2), 3);
    //             assert_eq!(InternedTree::references(&interned3), 2);
    //             // We now have two unique interned strings: "foo" and "bar".
    //             assert_eq!(num_objects_interned_tree::<String>(), 2);
    //         }

    //         // "bar" is now gone.
    //         assert_eq!(num_objects_interned_tree::<String>(), 1);
    //     }

    //     // Test basic functionality.
    //     #[test]
    //     fn basic_tree_unsized() {
    //         assert_eq!(intern_tree_unsized("foo"), intern_tree_unsized("foo"));
    //         assert_ne!(intern_tree_unsized("foo"), intern_tree_unsized("bar"));
    //         // The above refs should be deallocated by now.
    //         assert_eq!(num_objects_interned_tree::<str>(), 0);

    //         let _interned1 = intern_tree_unsized("foo");
    //         {
    //             let interned2 = intern_tree_unsized("foo");
    //             let interned3 = intern_tree_unsized("bar");

    //             assert_eq!(InternedTree::references(&interned2), 3);
    //             assert_eq!(InternedTree::references(&interned3), 2);
    //             // We now have two unique interned strings: "foo" and "bar".
    //             assert_eq!(num_objects_interned_tree::<str>(), 2);
    //         }

    //         // "bar" is now gone.
    //         assert_eq!(num_objects_interned_tree::<str>(), 1);
    //     }

    //     // Ordering should be based on values, not pointers.
    //     // Also tests `Display` implementation.
    //     #[test]
    //     fn sorting() {
    //         let mut interned_vals = vec![
    //             intern_hash(4),
    //             intern_hash(2),
    //             intern_hash(5),
    //             intern_hash(0),
    //             intern_hash(1),
    //             intern_hash(3),
    //         ];
    //         interned_vals.sort();
    //         let sorted: Vec<String> = interned_vals.iter().map(|v| format!("{}", v)).collect();
    //         assert_eq!(&sorted.join(","), "0,1,2,3,4,5");
    //     }

    //     #[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
    //     pub struct TestStruct2(String, u64);

    //     #[test]
    //     fn sequential() {
    //         for _i in 0..10_000 {
    //             let mut interned = Vec::with_capacity(100);
    //             for j in 0..100 {
    //                 interned.push(intern_hash(TestStruct2("foo".to_string(), j)));
    //             }
    //         }

    //         assert_eq!(num_objects_interned_hash::<TestStruct2>(), 0);
    //     }

    //     #[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
    //     pub struct TestStruct(String, u64);

    //     // Quickly create and destroy a small number of interned objects from
    //     // multiple threads.
    //     #[test]
    //     fn multithreading1() {
    //         let mut thandles = vec![];
    //         for _i in 0..10 {
    //             thandles.push(thread::spawn(|| {
    //                 for _i in 0..100_000 {
    //                     let _interned1 = intern_hash(TestStruct("foo".to_string(), 5));
    //                     let _interned2 = intern_hash(TestStruct("bar".to_string(), 10));
    //                 }
    //             }));
    //         }
    //         for h in thandles.into_iter() {
    //             h.join().unwrap()
    //         }

    //         assert_eq!(num_objects_interned_hash::<TestStruct>(), 0);
    //     }
}
