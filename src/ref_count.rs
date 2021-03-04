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
use crate::loom::*;
use std::{
    borrow::Borrow,
    fmt::{Debug, Display, Formatter, Pointer, Result},
    hash::{Hash, Hasher},
    intrinsics::drop_in_place,
    ops::Deref,
    ptr::NonNull,
};

pub(crate) type RemovePtr<T> = fn(*const (), *const Interned<T>);

#[repr(C)]
struct RefCounted<T: ?Sized> {
    /// number of references held to this value, including the one from the interner map
    ///
    /// highest bit stores whether the remove_if_lsat pointer has been consumed
    refs: AtomicUsize,
    /// Pointer to the location of a function pointer that can remove a given
    /// Interned<T> from the interner map. This same pointer is also provided
    /// as the first argument to the remove_if_last function in order to find
    /// the interner state in memory, so use #[repr(C)] and put the function
    /// pointer first!
    ///
    /// The function must remove the value from the interner and drop the
    /// weak reference to the pointed-to location.
    remover: AtomicPtr<RemovePtr<T>>,
    value: T,
}

impl<T: ?Sized> RefCounted<T> {
    fn from_box(value: Box<T>) -> NonNull<Self> {
        // figure out the needed allocation size — this requires #[repr(C)]
        let layout = Layout::new::<RefCounted<()>>()
            .extend(Layout::for_value(value.as_ref()))
            .unwrap() // fails only on address range overflow
            .0
            .pad_to_align();
        unsafe {
            // allocate using global allocator
            let ptr = alloc(layout);
            // get value pointer with the right metadata (e.g. string length)
            // while making sure to NOT DROP THE BOX
            let b = Box::leak(value) as *mut T;
            // construct correct (fat) pointer from allocation result
            let ptr = {
                let mut temp = b as *mut Self;
                // unfortunately <*const>::set_ptr_value is still experimental, but this is what it does:
                std::ptr::write(&mut temp as *mut _ as *mut *mut u8, ptr);
                temp
            };
            // write the fields
            (*ptr).refs = AtomicUsize::new(1);
            (*ptr).remover = AtomicPtr::new(std::ptr::null_mut());
            let num_bytes = std::mem::size_of_val(&*b);
            if num_bytes > 0 {
                std::ptr::copy_nonoverlapping(
                    // copy payload value byte-wise, because what else can we do?
                    b as *const u8,
                    &mut (*ptr).value as *mut _ as *mut u8,
                    num_bytes,
                );
                // free the memory of the ex-Box; global allocator does not allow zero-sized allocations
                // so this must be guarded by num_bytes > 0
                #[cfg(not(loom))]
                dealloc(b as *mut u8, Layout::for_value(&*b));
                #[cfg(loom)]
                std::alloc::dealloc(b as *mut u8, Layout::for_value(&*b));
            }

            NonNull::new_unchecked(ptr)
        }
    }

    fn from_sized(value: T) -> NonNull<Self>
    where
        T: Sized,
    {
        let b = Box::new(Self {
            refs: AtomicUsize::new(1),
            remover: AtomicPtr::new(std::ptr::null_mut()),
            value,
        });
        NonNull::from(Box::leak(b))
    }
}

pub struct Interned<T: ?Sized> {
    inner: NonNull<RefCounted<T>>,
}

unsafe impl<T: ?Sized + Sync + Send> Send for Interned<T> {}
unsafe impl<T: ?Sized + Sync + Send> Sync for Interned<T> {}

/// An interned value
///
/// This type works very similar to an [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html)
/// with the difference that it has no concept of weak references. They are not needed because
/// **interned values must not be modified**, so reference cycles cannot be constructed. One
/// reference is held by the interner that created this value as long as that interner lives.
///
/// Keeping interned values around does not keep the interner alive: once the last reference to
/// the interner is dropped, it will release its existing interned values, so the backing memory
/// will be released once each of the interned values is no longer referenced through any `Interned`
/// instances. (`Interned` keeps a [`Weak`](https://doc.rust-lang.org/std/sync/struct.Weak.html)
/// reference to the interner that created it, so it will prevent the `ArcInner` from being
/// deallocated while it lives.)
impl<T: ?Sized> Interned<T> {
    /// Obtain current number of references, including this one, using
    /// [`Ordering::Relaxed`](https://doc.rust-lang.org/std/sync/atomic/enum.Ordering.html#variant.Relaxed).
    /// This means that reads and writes of your code may be freely reordered around this
    /// read, there is no synchronisation with other threads.
    ///
    /// The value will always be at least 1. If the value is 1, this means that the interner
    /// which produced this reference has been dropped; in this case you are still free to
    /// use this reference in any way you like.
    pub fn ref_count(&self) -> usize {
        self.inner().refs.load(Relaxed)
    }

    fn inner(&self) -> &RefCounted<T> {
        // this is safe since the existence of &self proves that the pointer is still valid
        unsafe { self.inner.as_ref() }
    }

    pub(crate) fn from_box(value: Box<T>) -> Self {
        Self {
            inner: RefCounted::from_box(value),
        }
    }

    pub(crate) fn from_sized(value: T) -> Self
    where
        T: Sized,
    {
        Self {
            inner: RefCounted::from_sized(value),
        }
    }

    pub(crate) fn make_hot(&mut self, map: *mut RemovePtr<T>) -> bool {
        self.inner()
            .remover
            .compare_exchange(std::ptr::null_mut(), map, Release, Relaxed)
            .is_ok()
    }
}

// use the two upper bits as spin-wait conditions
const MAX_REFCOUNT: usize = usize::MAX - 2;

// cannot use null: spurious `get` failure in DashMap may lead to another make_hot
// which we prevent by using a non-null marker when taking the removal function
const TAKEN: *mut u8 = std::mem::align_of::<RemovePtr<()>>() as *mut _;

impl<T: ?Sized> Clone for Interned<T> {
    fn clone(&self) -> Self {
        if self.inner().refs.fetch_add(1, Relaxed) >= MAX_REFCOUNT {
            // the below misspelling is deliberate
            panic!("either you are running on an 8086 or you are leaking Interned values at a phantastic rate");
        }
        let ret = Self { inner: self.inner };
        #[cfg(feature = "println")]
        println!("{:?} clone {:p}", current().id(), *self);
        ret
    }
}

impl<T: ?Sized> Drop for Interned<T> {
    fn drop(&mut self) {
        // printing `self` to mark this particular execution (points to the stack)
        // printing `*self` to mark the interned value (as printed by `clone`)
        #[cfg(feature = "println")]
        println!("{:?} dropping {:p} {:p}", current().id(), self, *self);

        // preconditions:
        //  - this Interned may or may not be referenced by an interner (since the interner can be dropped)
        //  - the `self` reference guarantees that the reference count is at least one
        //  - whatever happens, we must decrement the reference count by one
        //  - if the only remaining reference is the interner map, we need to try to remove it
        //    (this races against an `intern` call for the same value and against dropping the interner)
        //
        // IMPORTANT NOTE: each Interned starts out with two references!
        //
        // Also, THE REMOVAL POINTER NEEDS TO BE USED EXACTLY ONCE!

        // decrement the count and read the value; the Release synchronises with an Acquire in case we deallocate
        let read = self.inner().refs.fetch_sub(1, Release);

        // two cases require action:
        //  - count was two: perform the removal (unless already done)
        //  - count was one: deallocate

        #[cfg(feature = "println")]
        println!("{:?} read {} {:p} {:p}", current().id(), read, self, *self);

        if read > 2 {
            return;
        }

        if read == 2 {
            // the other reference could be the map or external (if the map was dropped and we didn’t get here yet)
            // so this races against:
            //  1. map being dropped
            //  2. same value being interned again
            //  3. other external reference being dropped
            // In the dropping cases, the other thread saw read == 1.
            let remove_ptr = self.inner().remover.swap(TAKEN as *mut _, Acquire);
            if remove_ptr as *mut u8 != TAKEN {
                #[cfg(feature = "println")]
                println!("{:?} remover {:p} {:p}", current().id(), self, *self);
                // We’re the first to see ref_count 2, so drop the connection to the interner.
                // There is a race here against concurrent interning of the same value or clone & drop
                // of this value if the interner was dropped earlier. In both cases we leave the
                // interner without this value, which in the first case means that a freshly interned
                // and live value is now unknown to the interner — interning it again will yield a
                // different RefCounted instance (which is why we can’t support pointer Eq).
                let raw_arc = remove_ptr as *const ();
                let remover = unsafe { *remove_ptr };
                // If this thread saw read == 2 and another saw read == 1, didn’t get the remove_ptr,
                // and proceeded with the deallocation before we get here, then passing the self
                // reference is undefined behavior. OTOH, this can only happen if the interner has
                // been dropped, because otherwise that other reference is in the interner and cannot
                // concurrently be dropped. So we will only ever USE the self pointer when it is safe.
                // (but this is why it cannot be &Interned<T>, it must be *const Interned<T>)
                remover(raw_arc, self);
                #[cfg(feature = "println")]
                println!("{:?} removed {:p} {:p}", current().id(), self, *self);
            } else {
                #[cfg(feature = "println")]
                println!("{:?} second {:p} {:p}", current().id(), self, *self);
            }
        } else if read == 1 {
            // we must wait until the thread that saw read == 2 has taken the remover
            let mut spin_count = 0;
            loop {
                let p = self.inner().remover.load(Relaxed) as *mut u8;
                if p == TAKEN || p.is_null() {
                    break;
                }
                spin_count += 1;
                if spin_count < 100 {
                    spin_loop_hint(); // this is just a CPU hint that we’re waiting for a cache line to change
                } else {
                    yield_now(); // this is a syscall to ask the kernel to let another thread run
                }
            }

            #[cfg(feature = "println")]
            println!("{:?} drop {:p} {:p}", current().id(), self, *self);

            // Final reference, do delete! We need to ensure that a previous drop on a different
            // thread has stopped using the data (i.e. synchronise with the Release in fetch_sub).
            assert!(self.inner().refs.load(Acquire) == 0);

            let layout = Layout::for_value(self.inner());
            unsafe {
                // this is how you drop unsized values ...
                drop_in_place(self.inner.as_ptr());
                // and then we still have to free the memory
                dealloc(self.inner.as_ptr() as *mut u8, layout);
            }
        }

        #[cfg(feature = "println")]
        println!("{:?} dropend {:p} {:p}", current().id(), self, *self);
    }
}

impl<T: ?Sized + PartialEq> PartialEq for Interned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner().value.eq(&other.inner().value)
    }
}
impl<T: ?Sized + Eq> Eq for Interned<T> {}

impl<T: ?Sized + PartialOrd> PartialOrd for Interned<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.inner().value.partial_cmp(&other.inner().value)
    }
}
impl<T: ?Sized + Ord> Ord for Interned<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.inner().value.cmp(&other.inner().value)
    }
}

impl<T: ?Sized + Hash> Hash for Interned<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner().value.hash(state)
    }
}

impl<T: ?Sized> Borrow<T> for Interned<T> {
    fn borrow(&self) -> &T {
        &self.inner().value
    }
}

// The following would be nice, but it clashes with the Borrow<T> for T blanket impl
// impl<T: ?Sized + Borrow<X>, X: ?Sized> Borrow<X> for Interned<T> {
//     fn borrow(&self) -> &T {
//         &self.inner().value.borrow()
//     }
// }

impl<T: ?Sized> Deref for Interned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl<T: ?Sized> AsRef<T> for Interned<T> {
    fn as_ref(&self) -> &T {
        self.deref()
    }
}

impl<T: ?Sized + Debug> Debug for Interned<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "Interned({:?})", &*self)
    }
}

impl<T: ?Sized + Display> Display for Interned<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        self.deref().fmt(f)
    }
}

impl<T: ?Sized> Pointer for Interned<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        Pointer::fmt(&(&**self as *const T), f)
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use crate::InternOrd;

    #[test]
    fn pointer() {
        let interner = InternOrd::new();
        let i = interner.intern_sized(42);
        let i2 = i.clone();
        assert_eq!(format!("{:p}", i), format!("{:p}", i2));
    }
}
