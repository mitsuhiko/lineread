//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

/// ESC followed by an unbound key in vi command mode: nothing.
pub unsafe extern "C" fn vi_esc_noop(_count: c_int, _key: c_int) -> c_int {
    0
}

/// Is the editor in vi command (movement) mode?
pub fn in_command_mode() -> bool {
    crate::readline::rl_editing_mode.get() == crate::readline::VI_MODE
        && crate::keymap::current() == crate::keymap::vi_movement()
}

/// readline() starts every line in vi insert mode.
pub fn start_line_in_vi() {
    unsafe { crate::keymap::rl_set_keymap(crate::keymap::vi_insertion()) };
}

/// accept-line in vi mode.
pub fn newline_hook() {}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_append_eol(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_append_eol");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_append_mode(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_append_mode");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_arg_digit(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_arg_digit");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_bWord(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_bWord");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_back_to_indent(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_back_to_indent");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_bracktype(_a0: c_int) -> c_int {
    todo_stub!("rl_vi_bracktype");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_bword(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_bword");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_change_case(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_change_case");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_change_char(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_change_char");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_change_to(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_change_to");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_char_search(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_char_search");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_check() -> c_int {
    todo_stub!("rl_vi_check");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_column(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_column");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_complete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_complete");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_delete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_delete");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_delete_to(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_delete_to");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_domove(_a0: c_int, _a1: *mut c_int) -> c_int {
    todo_stub!("rl_vi_domove");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_eWord(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_eWord");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_editing_mode(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_editing_mode");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_end_word(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_end_word");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_eof_maybe(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_eof_maybe");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_eword(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_eword");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_fWord(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_fWord");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_fetch_history(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_fetch_history");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_first_print(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_first_print");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_fword(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_fword");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_goto_mark(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_goto_mark");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_insert_beg(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_insert_beg");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_insert_mode(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_insert_mode");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_insertion_mode(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_insertion_mode");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_match(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_match");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_movement_mode(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_movement_mode");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_next_word(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_next_word");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_overstrike(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_overstrike");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_overstrike_delete(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_overstrike_delete");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_prev_word(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_prev_word");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_put(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_put");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_redo(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_redo");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_replace(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_replace");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_rubout(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_rubout");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_search(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_search");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_search_again(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_search_again");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_set_mark(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_set_mark");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_start_inserting(_a0: c_int, _a1: c_int, _a2: c_int) {
    todo_stub!("rl_vi_start_inserting");
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_subst(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_subst");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_tilde_expand(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_tilde_expand");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_undo(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_undo");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_unix_word_rubout(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_unix_word_rubout");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_yank_arg(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_yank_arg");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_yank_pop(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_yank_pop");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_yank_to(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_vi_yank_to");
    0
}
