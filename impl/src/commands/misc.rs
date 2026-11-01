//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

/// Abort the current command: ding, drop numeric argument, macros and
/// pending state.
pub fn abort_internal() {
    unsafe {
        crate::terminal::rl_ding();
    }
    crate::numarg::reset();
    crate::kbdmacro::clear_macro_input();
    crate::input::unset_state(RL_STATE_MACRODEF | RL_STATE_MULTIKEY | RL_STATE_MOREINPUT);
    crate::display::clear_arg_message();
}

#[no_mangle]
pub unsafe extern "C" fn rl_abort(_count: c_int, _key: c_int) -> c_int {
    abort_internal();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_execute_named_command(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_execute_named_command");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_restart_output(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_restart_output");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_stop_output(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_stop_output");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_tty_status(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_tty_status");
    0
}
