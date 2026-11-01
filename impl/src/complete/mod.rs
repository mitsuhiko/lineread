//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

#[no_mangle]
pub static rl_attempted_completion_function: CVar<Option<rl_completion_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_attempted_completion_over: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn rl_backward_menu_complete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_backward_menu_complete");
    0
}

#[no_mangle]
pub static rl_basic_quote_characters: CVar<*const c_char> = CVar::new(cs!("\"'"));

#[no_mangle]
pub static rl_basic_word_break_characters: CVar<*const c_char> = CVar::new(cs!(" \t\n\"\\'`@$><=;|&{("));

#[no_mangle]
pub static rl_char_is_quoted_p: CVar<Option<rl_linebuf_func_t>> = CVar::new(None);

#[no_mangle]
pub unsafe extern "C" fn rl_complete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_complete");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_complete_internal(_a0: c_int) -> c_int {
    todo_stub!("rl_complete_internal");
    0
}

#[no_mangle]
pub static rl_completer_quote_characters: CVar<*const c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub static rl_completer_word_break_characters: CVar<*const c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub static rl_completion_append_character: CVar<c_int> = CVar::new(b' ' as c_int);

#[no_mangle]
pub static rl_completion_display_matches_hook: CVar<Option<rl_compdisp_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_completion_entry_function: CVar<Option<rl_compentry_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_completion_found_quote: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_completion_invoking_key: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_completion_mark_symlink_dirs: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn rl_completion_matches(_a0: *const c_char, _a1: Option<rl_compentry_func_t>) -> *mut *mut c_char {
    todo_stub!("rl_completion_matches");
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn rl_completion_mode(_a0: Option<rl_command_func_t>) -> c_int {
    todo_stub!("rl_completion_mode");
    0
}

#[no_mangle]
pub static rl_completion_query_items: CVar<c_int> = CVar::new(100);

#[no_mangle]
pub static rl_completion_quote_character: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_completion_rewrite_hook: CVar<Option<rl_dequote_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_completion_suppress_append: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_completion_suppress_quote: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_completion_type: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_completion_word_break_hook: CVar<Option<rl_cpvfunc_t>> = CVar::new(None);

#[no_mangle]
pub unsafe extern "C" fn rl_delete_or_show_completions(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_delete_or_show_completions");
    0
}

#[no_mangle]
pub static rl_directory_completion_hook: CVar<Option<rl_icppfunc_t>> = CVar::new(None);

#[no_mangle]
pub static rl_directory_rewrite_hook: CVar<Option<rl_icppfunc_t>> = CVar::new(None);

#[no_mangle]
pub unsafe extern "C" fn rl_display_match_list(_a0: *mut *mut c_char, _a1: c_int, _a2: c_int) {
    todo_stub!("rl_display_match_list");
}

#[no_mangle]
pub unsafe extern "C" fn rl_export_completions(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_export_completions");
    0
}

#[no_mangle]
pub static rl_filename_completion_desired: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn rl_filename_completion_function(_a0: *const c_char, _a1: c_int) -> *mut c_char {
    todo_stub!("rl_filename_completion_function");
    core::ptr::null_mut()
}

#[no_mangle]
pub static rl_filename_dequoting_function: CVar<Option<rl_dequote_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_filename_quote_characters: CVar<*const c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub static rl_filename_quoting_desired: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static rl_filename_quoting_function: CVar<Option<rl_quote_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_filename_rewrite_hook: CVar<Option<rl_dequote_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_filename_stat_hook: CVar<Option<rl_icppfunc_t>> = CVar::new(None);

#[no_mangle]
pub static rl_full_quoting_desired: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static rl_ignore_completion_duplicates: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static rl_ignore_some_completions_function: CVar<Option<rl_compignore_func_t>> = CVar::new(None);

#[no_mangle]
pub static rl_inhibit_completion: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn rl_insert_completions(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_insert_completions");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_menu_complete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_menu_complete");
    0
}

#[no_mangle]
pub static rl_menu_completion_entry_function: CVar<Option<rl_compentry_func_t>> = CVar::new(None);

#[no_mangle]
pub unsafe extern "C" fn rl_old_menu_complete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_old_menu_complete");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_possible_completions(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_possible_completions");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_reparse_colors() {
    todo_stub!("rl_reparse_colors");
}

#[no_mangle]
pub static rl_sort_completion_matches: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static rl_special_prefixes: CVar<*const c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub unsafe extern "C" fn rl_username_completion_function(_a0: *const c_char, _a1: c_int) -> *mut c_char {
    todo_stub!("rl_username_completion_function");
    core::ptr::null_mut()
}
