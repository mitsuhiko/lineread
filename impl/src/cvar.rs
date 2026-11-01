//! `CVar<T>`: a global with C layout that both C and Rust read and write.
//!
//! `CVar<T>` is `#[repr(transparent)]` over `UnsafeCell<T>`, so a
//! `#[no_mangle] pub static NAME: CVar<T>` has exactly the layout of the C
//! object `T NAME;` and lives in writable memory.  C code (applications, the
//! test driver) reads and writes it directly; Rust uses `get`/`set`.
//!
//! Readline is single threaded by contract.  Rules (see ARCHITECTURE.md):
//! * `get`/`set` for `Copy` values are always fine; they never cache.
//! * `get_mut` hands out `&mut T`; keep such borrows short and never hold one
//!   across a call that can run application code (hooks, bound functions,
//!   `rl_getc_function`, completion generators) or re-enter the same global.
use core::cell::UnsafeCell;

#[repr(transparent)]
pub struct CVar<T>(UnsafeCell<T>);

// SAFETY: readline's API is single threaded; see the module docs.
unsafe impl<T> Sync for CVar<T> {}

impl<T> CVar<T> {
    pub const fn new(v: T) -> Self {
        CVar(UnsafeCell::new(v))
    }
    /// Raw pointer to the value (what C sees as `&NAME`).
    #[inline]
    pub const fn as_ptr(&self) -> *mut T {
        self.0.get()
    }
    /// Mutable access.  Keep the borrow short (module docs).
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub fn get_mut(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
    #[inline]
    pub fn set(&self, v: T) {
        unsafe { *self.0.get() = v }
    }
    #[inline]
    pub fn replace(&self, v: T) -> T {
        unsafe { core::ptr::replace(self.0.get(), v) }
    }
}

impl<T: Copy> CVar<T> {
    #[inline]
    pub fn get(&self) -> T {
        unsafe { core::ptr::read_volatile(self.0.get()) }
    }
}

impl CVar<core::ffi::c_int> {
    #[inline]
    pub fn add(&self, d: core::ffi::c_int) {
        self.set(self.get().wrapping_add(d))
    }
}
