use std::{
    alloc::{alloc, dealloc, Layout},
    borrow::Borrow,
    fmt::{Debug, Display, Formatter, Pointer, Result},
    hash::{Hash, Hasher},
    intrinsics::drop_in_place,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering::*},
};

pub(crate) type RemovePtr<T> = fn(*const (), &Interned<T>) -> RemovalResult;
pub(crate) enum RemovalResult {
    /// weak reference has been dropped
    Removed,
    /// weak reference has NOT been dropped
    NotRemoved,
    /// weak reference has been dropped
    MapGone,
}

#[repr(C)]
struct RefCounted<T: ?Sized> {
    /// number of references held to this value, including the one from the interner map
    refs: AtomicUsize,
    /// Pointer to the location of a function pointer that can remove a given
    /// Interned<T> from the interner map. This same pointer is also provided
    /// as the first argument to the remove_if_last function in order to find
    /// the interner state in memory, so use #[repr(C)] and put the function
    /// pointer first!
    ///
    /// The function must remove the value from the interner if the reference
    /// count is TWO (one for the map, one for the last external reference).
    remove_if_last: AtomicPtr<RemovePtr<T>>,
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
            (*ptr).remove_if_last = AtomicPtr::new(std::ptr::null_mut());
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
                dealloc(b as *mut u8, Layout::for_value(&*b));
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
            remove_if_last: AtomicPtr::new(std::ptr::null_mut()),
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
            .remove_if_last
            .compare_exchange(std::ptr::null_mut(), map, Relaxed, Relaxed)
            .is_ok()
    }
}

impl<T: ?Sized> Clone for Interned<T> {
    fn clone(&self) -> Self {
        const MAX_REFCOUNT: usize = usize::MAX - 1;
        if self.inner().refs.fetch_add(1, Relaxed) == MAX_REFCOUNT {
            // the below misspelling is deliberate
            panic!("either you are running on an 8086 or you are leaking Interned values at a phantastic rate");
        }
        let ret = Self { inner: self.inner };
        #[cfg(feature = "println")]
        println!("clone {:p} {:p}", self, ret);
        ret
    }
}

impl<T: ?Sized> Drop for Interned<T> {
    fn drop(&mut self) {
        #[cfg(feature = "println")]
        println!("dropping {:p}", self);
        // precondition:
        //  - this Interned may or may not be referenced by an interner (since the interner can be dropped)
        //  - the `self` reference guarantees that the reference count is at least one
        //  - whatever happens, we must decrement the reference count by one
        //  - if the only remaining reference is the interner map, we need to try to remove it
        //    (this races against an `intern` call for the same value)
        //
        // IMPORTANT NOTE: each Interned starts out with two references!

        // after decrementing the reference count, we must consider our memory to be freed
        // by another thread (impossible when prior_refs == 1), so all work must happen before
        loop {
            if self.inner().refs.load(Relaxed) == 2 {
                // three scenarios:
                // 1. this external reference plus the map
                //    (only race condition is against interning of same value or drop of interner)
                // 2. this external reference plus another external
                //    (race conditions against drop & clone, latter allows further usage and dropping)
                // 3. this map reference plus external: successful remove_if_last or interner is being dropped
                //    (race conditions against drop & clone)
                const TAKEN: *mut u8 = std::mem::align_of::<RemovePtr<()>>() as *mut _;
                // must not set to null here: spurious `get` failure in DashMap may lead to another make_hot
                let remove_ptr = self.inner().remove_if_last.swap(TAKEN as *mut _, AcqRel);
                if remove_ptr as *mut u8 == TAKEN {
                    // happens in scenario 2, in which case this has turned into a normal Arc.
                    // (here it might be that the function pointer is still held by the other concurrent dropper,
                    // but that one will put it back and retry)
                    // happens in scenario 1 during successful remove_if_last.
                    // happens in scenario 1 while dropping the interner.
                    // does not happen during interning race because there ref count is 1
                    #[cfg(feature = "println")]
                    println!("null {:p}", self);
                    break;
                } else {
                    // we got a valid pointer because our weak reference is still in place
                    //
                    // THIS FUNCTION POINTER MUST BE USED SUCCESSFULLY EXACTLY ONCE!
                    //
                    let raw_arc = remove_ptr as *const ();
                    let remove_if_last = unsafe { *remove_ptr };
                    match remove_if_last(raw_arc, self) {
                        RemovalResult::Removed => {
                            // scenario 1 confirmed and races won:
                            // weak reference on interner has been dropped, so has our ref_count
                            // so the code below will now drop this value
                            #[cfg(feature = "println")]
                            println!("removed {:p}", self);
                            break;
                        }
                        RemovalResult::NotRemoved => {
                            // scenario 1 confirmed and race lost:
                            // another thread has won the race and obtained a fresh reference, so we
                            // keep our weak reference and put back the removal function pointer
                            self.inner().remove_if_last.store(remove_ptr, Release);
                            // at this point it is unclear what else has happened (other reference could already
                            // have been dropped or interner dropped), so we must check again: seeing 3 means that
                            // someone else will successfully use the pointer in the future, seeing 1 means that
                            // the situation has been cleared up permanently, seeing 2 we need to try again
                            #[cfg(feature = "println")]
                            println!("loop {:p}", self);
                        }
                        RemovalResult::MapGone => {
                            // scenario 2 or scenario 1 with concurrent drop of interner
                            // the interner has begun dropping at some point in the past, so its reference to
                            // us is either gone or will be gone soon; in any case our weak reference to it is toast
                            #[cfg(feature = "println")]
                            println!("gone{:p}", self);
                            break;
                        }
                    }
                }
            } else {
                break;
            }
        }

        // Release ordering is needed to be able to synchronise with the Acquire before actually dropping the value
        // to ensure that there cannot be any pending writes to the allocation.
        // Acquire ordering is needed to synchronise with a possible Release on remove_if_last.
        let prior_refs = self.inner().refs.fetch_sub(1, Release);
        if prior_refs != 1 {
            #[cfg(feature = "println")]
            println!("no drop {:p}", self);
            return;
        }
        #[cfg(feature = "println")]
        println!("drop {:p}", self);

        // Final reference, do delete! We need to ensure that the previous drop on a different
        // thread has stopped using the data (i.e. synchronise with the Release above).
        assert!(self.inner().refs.load(Acquire) == 0);

        let layout = Layout::for_value(self.inner());
        unsafe {
            // this is how you drop unsized values ...
            drop_in_place(self.inner.as_ptr());
            // and then we still have to free the memory
            dealloc(self.inner.as_ptr() as *mut u8, layout);
        }
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
