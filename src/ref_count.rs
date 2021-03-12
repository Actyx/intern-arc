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
use parking_lot::lock_api::RawMutex;
use std::{
    cell::UnsafeCell,
    fmt::{Formatter, Pointer, Result},
    ops::Deref,
    ptr::NonNull,
    sync::{Arc, Weak},
};

/// This type must be sized so that we can be sure of the memory layout
/// and therefore the pointer width. This allows the Interned< I> to not
/// have the Interner as a type parameter: it just stores a pointer that
/// we then cast appropriately in `inner()`.
pub trait Interner: Sized {
    type T: ?Sized;
    fn remove(&self, value: &Interned<Self>) -> (bool, Option<Interned<Self>>);
}
/// This is a bogus shim impl used only for being able to compute the size of a RefCounted structure.
impl Interner for () {
    type T = ();
    fn remove(&self, _value: &Interned<Self>) -> (bool, Option<Interned<Self>>) {
        (false, None)
    }
}

struct State<I> {
    // inlining the raw mutex manually here to bring overhead down from 24 to 16 bytes
    // on 64bit platforms (which unfortunately implies writing our own `struct Guard`)
    mutex: parking_lot::RawMutex,
    /// 4 billion refs ought to be enough for anybody, plus this allows the RawMutex
    /// to live inside the same word on 64bit architectures.
    refs: UnsafeCell<u32>,
    cleanup: UnsafeCell<Option<Weak<I>>>,
}
impl<I: Interner> State<I> {
    pub fn new() -> Self {
        Self {
            mutex: parking_lot::RawMutex::INIT,
            refs: UnsafeCell::new(1),
            cleanup: UnsafeCell::new(None),
        }
    }
    pub fn lock(&self) -> Guard<I> {
        self.mutex.lock();
        Guard(self)
    }
}
struct Guard<'a, I>(&'a State<I>);
impl<'a, I> Guard<'a, I> {
    pub fn refs(&self) -> u32 {
        unsafe { *self.0.refs.get() }
    }
    pub fn refs_mut(&mut self) -> &mut u32 {
        unsafe { &mut *self.0.refs.get() }
    }
    pub fn cleanup(&mut self) -> &mut Option<Weak<I>> {
        unsafe { &mut *self.0.cleanup.get() }
    }
}
impl<'a, I> Drop for Guard<'a, I> {
    fn drop(&mut self) {
        unsafe { self.0.mutex.unlock() };
    }
}

#[repr(C)]
struct RefCounted<I: Interner> {
    state: State<I>,
    value: I::T,
}

impl<I: Interner> RefCounted<I> {
    fn from_box(value: Box<I::T>) -> NonNull<Self> {
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
            let b = Box::leak(value) as *mut I::T;
            // construct correct (fat) pointer from allocation result
            let ptr = {
                let mut temp = b as *mut Self;
                // unfortunately <*const>::set_ptr_value is still experimental, but this is what it does:
                std::ptr::write(&mut temp as *mut _ as *mut *mut u8, ptr);
                temp
            };
            // write the fields
            std::ptr::write(&mut (*ptr).state, State::new());
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

    fn from_sized(value: I::T) -> NonNull<Self>
    where
        I::T: Sized,
    {
        let b = Box::new(Self {
            state: State::new(),
            value,
        });
        NonNull::from(Box::leak(b))
    }
}

pub struct Interned<I: Interner> {
    inner: NonNull<RefCounted<I>>,
}

unsafe impl<I: Interner> Send for Interned<I> where I::T: Send + Sync + 'static {}
unsafe impl<I: Interner> Sync for Interned<I> where I::T: Send + Sync + 'static {}

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
impl<I: Interner> Interned<I> {
    /// Obtain current number of references, including this one.
    ///
    /// The value will always be at least 1. If the value is 1, this means that the interner
    /// which produced this reference has been dropped; in this case you are still free to
    /// use this reference in any way you like.
    pub fn ref_count(&self) -> u32 {
        self.lock().refs()
    }

    fn lock(&self) -> Guard<I> {
        // this is safe since the existence of &self proves that the pointer is still valid
        unsafe { self.inner.as_ref().state.lock() }
    }

    pub(crate) fn from_box(value: Box<I::T>) -> Self {
        Self {
            inner: RefCounted::from_box(value),
        }
    }

    pub(crate) fn from_sized(value: I::T) -> Self
    where
        I::T: Sized,
    {
        Self {
            inner: RefCounted::from_sized(value),
        }
    }

    pub(crate) fn make_hot(&mut self, set: &Arc<I>) {
        let mut state = self.lock();
        *state.cleanup() = Some(Arc::downgrade(set));
    }
}

// use the two upper bits as spin-wait conditions
const MAX_REFCOUNT: u32 = u32::MAX - 2;

impl<I: Interner> Clone for Interned<I> {
    fn clone(&self) -> Self {
        let mut state = self.lock();
        let previous = state.refs();
        *state.refs_mut() += 1;
        drop(state);
        if previous >= MAX_REFCOUNT {
            // the below misspelling is deliberate
            panic!("either you are running on an 8086 or you are leaking Interned values at a phantastic rate");
        }
        let ret = Self { inner: self.inner };
        #[cfg(feature = "println")]
        println!("{:?} clone {:p}", current().id(), *self);
        ret
    }
}

impl<I: Interner> Drop for Interned<I> {
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
        // IMPORTANT NOTE: each Interned starts out with two references! (by virtue of creation and first clone)
        //
        // Also, THE REMOVAL POINTER NEEDS TO BE USED EXACTLY ONCE!

        // take the lock — we MUST NOT hold this lock while calling the cleanup function!
        let mut state = self.lock();

        #[cfg(feature = "println")]
        println!(
            "{:?} read {} {:p} {:p}",
            current().id(),
            state.refs(),
            self,
            *self
        );

        // decrement the count and read the value
        *state.refs_mut() -= 1;

        // two cases require action:
        //  - count was two: perform the removal (unless already done)
        //  - count was one: deallocate

        if state.refs() > 1 {
            return;
        }

        if state.refs() == 1 {
            // the other reference could be the map or external (if the map was dropped and we didn’t get here yet)
            // so this races against:
            //  1. map being dropped
            //  2. same value being interned again
            //  3. other external reference being dropped
            // In the dropping cases, the other thread saw read == 1.
            if let Some(cleanup) = state.cleanup().take() {
                #[cfg(feature = "println")]
                println!("{:?} removing {:p} {:p}", current().id(), self, *self);
                // At this point, the other remaining reference can either be the interner or an
                // external one (if the interner was already dropped).
                if let Some(strong) = cleanup.upgrade() {
                    // Interner is still there, so the other reference is in there; we may race
                    // against interning of the same value, which may already have taken the interner
                    // lock, so we cannot just call the cleanup function.
                    drop(state);
                    loop {
                        // in here another thread may have found the interned reference and started cloning it,
                        // it might even have dropped it already (but without running cleanup, since we have that)
                        let (removed, _value) = strong.remove(self);
                        if removed {
                            // nobody interfered and the value is now removed from the interner, we can safely drop it
                            break;
                        } else {
                            // someone interfered, so we need to take the lock again to put things in order
                            let mut state = self.lock();
                            if state.refs() > 1 {
                                // someone else will drop the refs to one later
                                *state.cleanup() = Some(cleanup);
                                break;
                            } else {
                                // whoever interfered already dropped their reference again, so we need to retry
                            }
                        }
                    }
                } else {
                    // Interner is gone or on its way out, which means that no concurrent interning
                    // is possible anymore; it also means that the other reference may well be from
                    // the interner, still (it may be dropping right now). Our job here is done.
                }
                #[cfg(feature = "println")]
                println!("{:?} removed {:p}", current().id(), self);
            } else {
                #[cfg(feature = "println")]
                println!("{:?} cleanup gone {:p}", current().id(), self);
            }
        } else if state.refs() == 0 {
            #[cfg(feature = "println")]
            println!("{:?} drop {:p} {:p}", current().id(), self, *self);

            // drop the lock before freeing the memory
            drop(state);

            // since we created the pointer with Box::leak(), we can recreate the box to drop it
            unsafe { Box::from_raw(self.inner.as_ptr()) };
        }

        #[cfg(feature = "println")]
        println!("{:?} dropend {:p}", current().id(), self);
    }
}

impl<I: Interner> Deref for Interned<I> {
    type Target = I::T;

    fn deref(&self) -> &Self::Target {
        &unsafe { self.inner.as_ref() }.value
    }
}

impl<I: Interner> Pointer for Interned<I> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        Pointer::fmt(&(&**self as *const I::T), f)
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use crate::OrdInterner;

    #[test]
    fn pointer() {
        let interner = OrdInterner::new();
        let i = interner.intern_sized(42);
        let i2 = i.clone();
        assert_eq!(format!("{:p}", i), format!("{:p}", i2));
    }

    #[test]
    fn size() {
        use std::mem::size_of;
        const SIZE: usize = if size_of::<usize>() == 4 { 12 } else { 16 };
        assert_eq!(size_of::<RefCounted<()>>(), SIZE);

        let fake = RefCounted::<crate::hash::Hash<i32>> {
            state: State::new(),
            value: 42,
        };

        println!("base:  {:p}", &fake);
        let base = &fake as *const _ as *const u8;
        println!("state: {:p} (base + {})", &fake.state, unsafe {
            (&fake.state as *const _ as *const u8).offset_from(base)
        });
        println!("mutex: {:p} (base + {})", &fake.state.mutex, unsafe {
            (&fake.state.mutex as *const _ as *const u8).offset_from(base)
        });
        println!("refs:  {:p} (base + {})", &fake.state.refs, unsafe {
            (&fake.state.refs as *const _ as *const u8).offset_from(base)
        });
        println!("clean: {:p} (base + {})", &fake.state.cleanup, unsafe {
            (&fake.state.cleanup as *const _ as *const u8).offset_from(base)
        });
        println!("value: {:p} (base + {})", &fake.value, unsafe {
            (&fake.value as *const _ as *const u8).offset_from(base)
        });
    }
}
