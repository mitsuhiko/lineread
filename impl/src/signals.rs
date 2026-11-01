//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

#[no_mangle]
pub static rl_catch_signals: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static rl_catch_sigwinch: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static rl_change_environment: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub unsafe extern "C" fn rl_check_signals() {
    todo_stub!("rl_check_signals");
}

#[no_mangle]
pub unsafe extern "C" fn rl_cleanup_after_signal() {
    todo_stub!("rl_cleanup_after_signal");
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_signals() -> c_int {
    todo_stub!("rl_clear_signals");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_echo_signal_char(_a0: c_int) {
    todo_stub!("rl_echo_signal_char");
}

#[no_mangle]
pub unsafe extern "C" fn rl_free_line_state() {
    todo_stub!("rl_free_line_state");
}

#[no_mangle]
pub unsafe extern "C" fn rl_pending_signal() -> c_int {
    todo_stub!("rl_pending_signal");
    0
}

#[no_mangle]
pub static rl_persistent_signal_handlers: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn rl_reset_after_signal() {
    todo_stub!("rl_reset_after_signal");
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_signals() -> c_int {
    todo_stub!("rl_set_signals");
    0
}

#[no_mangle]
pub static rl_signal_event_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);
