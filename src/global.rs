//! Convenience functions for managing type-indexed interner pools
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    hash::Hash,
};

use once_cell::sync::Lazy;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use crate::{HashInterner, OrdInterner};

static HASH_INTERNERS: Lazy<RwLock<HashInternerPool>> = Lazy::new(Default::default);

/// A lazy map from type to hash-based interner
///
/// This is the backing implementation for [`hash_interner()`](fn.hash_interner.html), you
/// can use it if you want the ability to drop the whole pool (note, though, that this will
/// not drop the interned values — they are kept alive by the references you keep to them
/// and are automatically freed when they are no longer references).
pub struct HashInternerPool {
    type_map: HashMap<TypeId, Box<dyn Any + Send + Sync + 'static>>,
}

impl Default for HashInternerPool {
    fn default() -> Self {
        Self::new()
    }
}

impl HashInternerPool {
    pub fn new() -> Self {
        Self {
            type_map: HashMap::new(),
        }
    }

    /// Obtain an interner for the given type if it has been created already.
    pub fn get<T>(&self) -> Option<HashInterner<T>>
    where
        T: Eq + Hash + Send + Sync + Any + ?Sized + 'static,
    {
        self.type_map
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref::<HashInterner<T>>().unwrap().clone())
    }

    /// Obtain an interner for the given type, create it if necessary.
    pub fn get_or_create<T>(&mut self) -> HashInterner<T>
    where
        T: Eq + Hash + Send + Sync + Any + ?Sized + 'static,
    {
        self.get::<T>().unwrap_or_else(|| {
            let ret = HashInterner::new();
            self.type_map
                .insert(TypeId::of::<T>(), Box::new(ret.clone()));
            ret
        })
    }
}

/// Obtain an interner for the given type, create it if necessary.
///
/// The interner will be stored for the rest of your program’s runtime within the global interner
/// pool and you will get the same instance when using this method for the same type later again.
pub fn hash_interner<T>() -> HashInterner<T>
where
    T: Eq + Hash + Send + Sync + Any + ?Sized + 'static,
{
    let map = HASH_INTERNERS.upgradable_read();
    if let Some(interner) = map.get::<T>() {
        return interner;
    }
    let mut map = RwLockUpgradableReadGuard::upgrade(map);
    map.get_or_create::<T>()
}

static ORD_INTERNERS: Lazy<RwLock<OrdInternerPool>> = Lazy::new(Default::default);

/// A lazy map from type to ord-based interner
///
/// This is the backing implementation for [`ord_interner()`](fn.ord_interner.html), you
/// can use it if you want the ability to drop the whole pool (note, though, that this will
/// not drop the interned values — they are kept alive by the references you keep to them
/// and are automatically freed when they are no longer references).
pub struct OrdInternerPool {
    type_map: HashMap<TypeId, Box<dyn Any + Send + Sync + 'static>>,
}

impl Default for OrdInternerPool {
    fn default() -> Self {
        Self::new()
    }
}

impl OrdInternerPool {
    pub fn new() -> Self {
        Self {
            type_map: HashMap::new(),
        }
    }

    /// Obtain an interner for the given type if it has been created already.
    pub fn get<T>(&self) -> Option<OrdInterner<T>>
    where
        T: Ord + Send + Sync + Any + ?Sized + 'static,
    {
        self.type_map
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref::<OrdInterner<T>>().unwrap().clone())
    }

    /// Obtain an interner for the given type, create it if necessary.
    pub fn get_or_create<T>(&mut self) -> OrdInterner<T>
    where
        T: Ord + Send + Sync + Any + ?Sized + 'static,
    {
        self.get::<T>().unwrap_or_else(|| {
            let ret = OrdInterner::new();
            self.type_map
                .insert(TypeId::of::<T>(), Box::new(ret.clone()));
            ret
        })
    }
}

/// Obtain an interner for the given type, create it if necessary.
///
/// The interner will be stored for the rest of your program’s runtime within the global interner
/// pool and you will get the same instance when using this method for the same type later again.
pub fn ord_interner<T>() -> OrdInterner<T>
where
    T: Ord + Send + Sync + Any + ?Sized + 'static,
{
    let map = ORD_INTERNERS.upgradable_read();
    if let Some(interner) = map.get::<T>() {
        return interner;
    }
    let mut map = RwLockUpgradableReadGuard::upgrade(map);
    map.get_or_create::<T>()
}
