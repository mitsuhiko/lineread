//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

#[no_mangle]
pub static tilde_additional_prefixes: CVar<*mut *mut c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub static tilde_additional_suffixes: CVar<*mut *mut c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn tilde_expand(_a0: *const c_char) -> *mut c_char {
    todo_stub!("tilde_expand");
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn tilde_expand_word(_a0: *const c_char) -> *mut c_char {
    todo_stub!("tilde_expand_word");
    core::ptr::null_mut()
}

#[no_mangle]
pub static tilde_expansion_failure_hook: CVar<Option<tilde_hook_func_t>> = CVar::new(None);

#[no_mangle]
pub static tilde_expansion_preexpansion_hook: CVar<Option<tilde_hook_func_t>> = CVar::new(None);
