# Library for interning based on stdlib Arc

The design of the hash-based half is taken from the [`arc-interner`](https://docs.rs/arc-interner)
crate, with the addition of another half based on [`BTreeMap`](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html).
Local benchmarks have shown that hashing is faster for objects below 1kB while tree traversal
and comparisons are faster above 1kB (very roughly and broadly speaking).

The main functions exist in three variants:

  - `intern_hash` and friends for when you want to use hashing (or have no `Ord` instance at hand)
  - `intern_tree` and friends for when you want to use tree map (or have no `Hash` instance at hand)
  - `intern` and friends to automatically choose based on object size

Within each of these classes, four function exist to ingest data in various forms:

```rust
use std::sync::Arc;
use intern_arc::{intern, intern_unsized, intern_boxed, intern_arc};

// for sized types
let a1: Arc<String> = intern("hello".to_owned());

// for unsized non-owned types
let a2: Arc<str> = intern_unsized("hello");

// for unsized owned types
let a3: Arc<str> = intern_boxed(Box::<str>::from("hello"));

// for types with shared ownership
let a4: Arc<str> = intern_arc(Arc::<str>::from("hello"));
```

# Introspection

This library offers some utilities for checking how well it works for a given use-case:

```rust
use std::sync::Arc;
use intern_arc::{inspect_hash, num_objects_interned_hash, types_interned};

println!("str: {} objects", num_objects_interned_hash::<str>());

let (hash, tree) = types_interned();
println!("types interned: {} with hashing, {} with trees", hash, tree);

inspect_hash::<[u8], _, _>(|iter| {
    for arc in iter {
        println!("{} x {:?}", Arc::strong_count(&arc), arc);
    }
});
```

All function exist also with `tree` suffix instead of `hash`.