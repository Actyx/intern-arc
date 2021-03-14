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
//! Interning library based on atomic reference counting
//!
//! This library differs from [`arc-interner`](https://crates.io/crates/arc-interner)
//! (which served as initial inspiration) in that
//!
//!  - interners must be created and can be dropped (for convenience functions see below)
//!  - it does not dispatch based on [`TypeId`](https://doc.rust-lang.org/std/any/struct.TypeId.html)
//!    (each interner is for exactly one type)
//!  - it offers both [`Hash`](https://doc.rust-lang.org/std/hash/trait.Hash.html)-based
//!    and [`Ord`](https://doc.rust-lang.org/std/cmp/trait.Ord.html)-based storage
//!  - it handles unsized types without overhead, so you should inline
//!    [`str`](https://doc.rust-lang.org/std/primitive.str.html) instead of
//!    [`String`](https://doc.rust-lang.org/std/string/struct.String.html)
//!
//! Unfortunately, this combination of features makes it inevitable to use unsafe Rust.
//! The construction of unsized values has been adapted from the standard library’s
//! [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html) type; reference counting
//! employs a [`parking_lot`](https://docs.rs/parking_lot) mutex since it must also ensure
//! consistent access to and usage of the cleanup function that will remove the value from its interner.
//! The test suite passes also under [miri](https://github.com/rust-lang/miri) to
//! check against some classes of undefined behavior in the unsafe code (including memory leaks).
//!
//! ## API flavors
//!
//! The same API is provided in two flavors:
//!
//!  - [`HashInterner`](struct.HashInterner.html) uses hash-based storage, namely the standard
//!    [`HashSet`](https://doc.rust-lang.org/std/collections/struct.HashSet.html)
//!  - [`OrdInterner`](struct.OrdInterner.html) uses ord-based storage, namely the standard
//!    [`BTreeSet`](https://doc.rust-lang.org/std/collections/struct.BTreeSet.html)
//!
//! Interning small values takes of the order of 100–200ns on a typical server CPU. The ord-based
//! storage has an advantage when interning large values (like slices greater than 1kB). The
//! hash-based storage has an advantage when keeping lots of values (many thousands and up)
//! interned at the same time.
//!
//! Nothing beats your own benchmarking, though.
//!
//! ## Convenience access to interners
//!
//! When employing interning for example within a dedicated deserialiser thread, it is best to create
//! and locally use an interner, avoiding further synchronisation overhead. You can also store interners
//! in thread-local variables if you only care about deduplication per thread.
//!
//! That said, this crate also provides convenience functions based on a global type-indexed pool:
//!
//! ```
//! use intern_arc::{global::hash_interner};
//!
//! let i1 = hash_interner().intern_ref("hello"); // -> InternedHash<str>
//! let i2 = hash_interner().intern_sized(vec![1, 2, 3]); // -> InternedHash<Vec<i32>>
//! # use std::any::{Any, TypeId}; // using TypeId to avoid influencing type interence
//! # use intern_arc::InternedHash;
//! # assert_eq!(i1.type_id(), TypeId::of::<InternedHash<str>>());
//! # assert_eq!(i2.type_id(), TypeId::of::<InternedHash<Vec<i32>>>());
//! ```
//!
//! You can also use the type-indexed pool yourself to control its lifetime:
//!
//! ```
//! use intern_arc::{global::OrdInternerPool, InternedOrd};
//!
//! let mut pool = OrdInternerPool::new();
//! let i: InternedOrd<[u8]> = pool.get_or_create().intern_box(vec![1, 2, 3].into());
//! ```
#![doc(html_logo_url = "https://developer.actyx.com/img/logo.svg")]
#![doc(html_favicon_url = "https://developer.actyx.com/img/favicon.ico")]

pub mod global;
mod hash;
mod ref_count;
mod tree;

pub use hash::{HashInterner, InternedHash};
pub use tree::{InternedOrd, OrdInterner};

#[cfg(loom)]
mod loom {
    pub use ::loom::{
        alloc::{alloc, dealloc, Layout},
        sync::{
            atomic::{spin_loop_hint, AtomicPtr, AtomicUsize, Ordering::*},
            MutexGuard, RwLock,
        },
        thread::{current, yield_now},
    };

    pub struct Mutex<T>(::loom::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Self(::loom::sync::Mutex::new(t))
        }
        pub fn lock(&self) -> MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
}

#[cfg(not(loom))]
mod loom {
    pub use parking_lot::{Mutex, MutexGuard, RwLock, RwLockUpgradableReadGuard};
    pub use std::{
        alloc::{alloc, dealloc, Layout},
        sync::atomic::{spin_loop_hint, AtomicPtr, AtomicUsize, Ordering::*},
        thread::{current, yield_now},
    };
}
