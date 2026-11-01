//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

#[no_mangle]
pub unsafe extern "C" fn get_history_event(_a0: *const c_char, _a1: *mut c_int, _a2: c_int) -> *mut c_char {
    todo_stub!("get_history_event");
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn history_arg_extract(_a0: c_int, _a1: c_int, _a2: *const c_char) -> *mut c_char {
    todo_stub!("history_arg_extract");
    core::ptr::null_mut()
}

#[no_mangle]
pub static history_comment_char: CVar<c_char> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn history_expand(_a0: *const c_char, _a1: *mut *mut c_char) -> c_int {
    todo_stub!("history_expand");
    0
}

#[no_mangle]
pub static history_expansion_char: CVar<c_char> = CVar::new(b'!' as c_char);

#[no_mangle]
pub static history_inhibit_expansion_function: CVar<Option<rl_linebuf_func_t>> = CVar::new(None);

#[no_mangle]
pub static history_no_expand_chars: CVar<*mut c_char> = CVar::new(cs_mut!(" \t\n\r="));

#[no_mangle]
pub static history_quotes_inhibit_expansion: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_quoting_state: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_search_delimiter_chars: CVar<*mut c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub static history_subst_char: CVar<c_char> = CVar::new(b'^' as c_char);

#[no_mangle]
pub unsafe extern "C" fn history_tokenize(_a0: *const c_char) -> *mut *mut c_char {
    todo_stub!("history_tokenize");
    core::ptr::null_mut()
}

#[no_mangle]
pub static history_word_delimiters: CVar<*mut c_char> = CVar::new(cs_mut!(" \t\n;&()|<>"));
