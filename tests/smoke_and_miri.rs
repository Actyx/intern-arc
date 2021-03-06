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
use intern_arc::*;
use std::thread;

// Test basic functionality.
#[test]
fn basic_hash() {
    let interner = HashInterner::<&str>::new();

    assert_eq!(interner.intern_sized("foo"), interner.intern_sized("foo"));
    assert_ne!(interner.intern_sized("foo"), interner.intern_sized("bar"));
    // The above refs should be deallocated by now.
    assert_eq!(interner.len(), 0);

    let interner = HashInterner::<String>::new();

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
    let interner = HashInterner::<str>::new();

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
        &*HashInterner::new().intern_ref("foo") as *const _
    );

    assert_eq!(interned1.ref_count(), 1);
}

// Test basic functionality.
#[test]
fn basic_ord() {
    let interner = OrdInterner::<&str>::new();

    assert_eq!(interner.intern_sized("foo"), interner.intern_sized("foo"));
    assert_ne!(interner.intern_sized("foo"), interner.intern_sized("bar"));
    // The above refs should be deallocated by now.
    assert_eq!(interner.len(), 0);

    let interner = OrdInterner::<String>::new();

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
fn basic_ord_unsized() {
    let interner = OrdInterner::<str>::new();

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
        &*OrdInterner::new().intern_ref("foo") as *const _
    );

    assert_eq!(interned1.ref_count(), 1);
}

// Ordering should be based on values, not pointers.
// Also tests `Display` implementation.
#[test]
fn sorting() {
    let interner = HashInterner::new();
    let mut interned_vals = vec![
        interner.intern_sized(4),
        interner.intern_sized(2),
        interner.intern_sized(5),
        interner.intern_sized(0),
        interner.intern_sized(1),
        interner.intern_sized(3),
    ];
    interned_vals.sort();
    let sorted: Vec<String> = interned_vals.iter().map(|v| format!("{}", v)).collect();
    assert_eq!(&sorted.join(","), "0,1,2,3,4,5");
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TestStruct(String, u64);

#[test]
fn sequential_hash() {
    let interner = HashInterner::new();

    for _i in 0..10 {
        let mut interned = Vec::with_capacity(100);
        for j in 0..10 {
            interned.push(interner.intern_sized(TestStruct("foo".to_string(), j)));
        }
    }

    assert_eq!(interner.len(), 0);
}

#[test]
fn sequential_ord() {
    let interner = OrdInterner::new();

    for _i in 0..10 {
        let mut interned = Vec::with_capacity(100);
        for j in 0..10 {
            interned.push(interner.intern_sized(TestStruct("foo".to_string(), j)));
        }
    }

    assert_eq!(interner.len(), 0);
}

// Quickly create and destroy a small number of interned objects from
// multiple threads.
#[test]
fn multithreading_hash() {
    let interner = HashInterner::new();

    let mut thandles = vec![];
    for _i in 0..3 {
        let interner = interner.clone();
        thandles.push(thread::spawn(move || {
            #[cfg(feature = "println")]
            println!("begin");
            #[allow(unused_variables)]
            for i in 0..10 {
                let _interned1 = interner.intern_sized(TestStruct("foo".to_string(), 5));
                let _interned2 = interner.intern_sized(TestStruct("bar".to_string(), 10));
                #[cfg(feature = "println")]
                println!("loop {} done", i);
            }
        }));
    }
    for h in thandles.into_iter() {
        h.join().unwrap()
    }

    assert_eq!(interner.len(), 0);
}

// Quickly create and destroy a small number of interned objects from
// multiple threads.
#[test]
fn multithreading_ord() {
    let interner = OrdInterner::new();

    let mut thandles = vec![];
    for _i in 0..3 {
        let interner = interner.clone();
        thandles.push(thread::spawn(move || {
            #[allow(unused_variables)]
            for i in 0..10 {
                let _interned1 = interner.intern_sized(TestStruct("foo".to_string(), 5));
                let _interned2 = interner.intern_sized(TestStruct("bar".to_string(), 10));
                #[cfg(feature = "println")]
                println!("loop {} done", i);
            }
        }));
    }
    for h in thandles.into_iter() {
        h.join().unwrap()
    }

    assert_eq!(interner.len(), 0);
}
