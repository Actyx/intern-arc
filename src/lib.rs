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
//!  - it contains no static global state (interners must be created and can be dropped;
//!    you can use [`OnceCell`](https://docs.rs/once_cell) or [`lazy_static!`](https://docs.rs/lazy_static)
//!    to manage global instances)
//!  - it does not dispatch based on TypeId (each interner is for exactly one type)
//!  - it offers both [`Hash`](https://doc.rust-lang.org/std/hash/trait.Hash.html)-based
//!    and [`Ord`](https://doc.rust-lang.org/std/cmp/trait.Ord.html)-based storage
//!  - it handles unsized types without overhead, so you should use `Intern<str>` instead of `Intern<String>`
//!
//! Unfortunately, this combination of features makes it inevitable to use unsafe Rust.
//! The handling of reference counting and constructing of unsized values has been adapted
//! from the standard library’s [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html) type.
//! Additionally, the test suite passes also under [miri](https://github.com/rust-lang/miri) to
//! check against some classes of undefined behavior in the unsafe code (including memory leaks).
//!
//! ## API flavors
//!
//! The same API is provided in two flavors:
//!
//!  - [`InternHash`](struct.InternHash.html) uses hash-based storage, namely a [`DashMap`](https://docs.rs/dashmap)
//!  - [`InternOrd`](struct.InternOrd.html) uses ord-based storage, namely the standard
//!    [`BTreeSet`](https://doc.rust-lang.org/std/collections/struct.BTreeSet.html)
//!
//! Interning small values takes of the order of 100–200ns on a typical server CPU. The ord-based
//! storage has an advantage when interning large values (like slices greater than 1kB). The
//! hash-based storage has an advantage when keeping lots of values (many thousands and up)
//! interned at the same time.
//!
//! Nothing beats your own benchmarking, though.
//!
//! ## Caveat emptor!
//!
//! This crate’s [`Interned`](struct.Interned.html) type does not optimise equality using
//! pointer comparisons because there is a race condition between dropping a value and
//! interning that same value that will lead to “orphaned” instances (meaning that
//! interning that same value again later will yield a different storage location).
//! All similarly constructed interning implementations share this caveat (e.g.
//! [`internment`](https://crates.io/crates/internment) or the above mentioned
//! [`arc-interner`](https://crates.io/crates/arc-interner)).
#![doc(html_logo_url = "https://developer.actyx.com/img/logo.svg")]
#![doc(html_favicon_url = "https://developer.actyx.com/img/favicon.ico")]

mod hash;
mod ref_count;
mod tree;

pub use hash::InternHash;
pub use ref_count::Interned;
pub use tree::InternOrd;

#[cfg(loom)]
mod loom {
    pub use ::loom::alloc::{alloc, dealloc, Layout};
    pub use ::loom::sync::atomic::{spin_loop_hint, AtomicPtr, AtomicUsize, Ordering::*};
    pub use ::loom::sync::MutexGuard;
    pub use ::loom::thread::{current, yield_now};

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
    pub use parking_lot::{Mutex, MutexGuard};
    pub use std::alloc::{alloc, dealloc, Layout};
    pub use std::hint::spin_loop as spin_loop_hint;
    pub use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::*};
    pub use std::thread::{current, yield_now};
}
