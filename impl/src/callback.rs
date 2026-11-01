//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

/// Called before every rl_getc_function call (callback suspension point).
pub fn before_getc() {}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_handler_install(_a0: *const c_char, _a1: Option<rl_vcpfunc_t>) {
    todo_stub!("rl_callback_handler_install");
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_handler_remove() {
    todo_stub!("rl_callback_handler_remove");
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_read_char() {
    todo_stub!("rl_callback_read_char");
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_sigcleanup() {
    todo_stub!("rl_callback_sigcleanup");
}
