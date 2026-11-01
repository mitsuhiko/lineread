//! Readline's history commands: moving through the history list while
//! editing, saving the line being edited, per-entry undo lists (kept in
//! `HIST_ENTRY.data`), restoring edited entries when a line is accepted.
//! Owner: history workstream (search area).
use crate::buffer;
use crate::history::{self, history_length, history_offset};
use crate::prelude::*;
use crate::terminal::rl_ding;
use crate::undo::rl_undo_list;

/// The line being edited before moving into the history.
struct Saved {
    text: Vec<u8>,
    point: c_int,
    undo: *mut UNDO_LIST,
}
static SAVED: CVar<Option<Saved>> = CVar::new(None);

/// Line setup: start at the end of the history list.
pub fn line_started() {
    unsafe { history::using_history() };
}

/// Clear `data` of entries that point to the undo list `l` (it is being
/// freed).
pub fn forget_undo_list(l: *mut UNDO_LIST) {
    if l.is_null() {
        return;
    }
    for i in 0..history_length.get() {
        let e = history::entry_at(i);
        unsafe {
            if !e.is_null() && (*e).data == l as histdata_t {
                (*e).data = null_mut();
            }
        }
    }
    if let Some(s) = SAVED.get_mut() {
        if s.undo == l {
            s.undo = null_mut();
        }
    }
}

/// The line was accepted: restore an edited history entry to its original
/// text (the result keeps the edits), drop the saved line.
pub fn line_finished() {
    unsafe {
        let o = history_offset.get();
        let e = history::entry_at(o);
        if !e.is_null() && !rl_undo_list.get().is_null() {
            let text = buffer::line().to_vec();
            let pt = buffer::rl_point.get();
            crate::undo::rl_revert_line(1, 0);
            let orig = buffer::line().to_vec();
            let c = c_strdup(&orig);
            let old = history::replace_history_entry(o, c, null_mut());
            cfree(c);
            history::free_history_entry(old);
            crate::undo::rl_free_undo_list();
            buffer::set_line(&text);
            buffer::rl_point.set(pt);
        }
        if let Some(s) = SAVED.get_mut().take() {
            crate::undo::free_list(s.undo);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_maybe_save_line() -> c_int {
    if SAVED.get_mut().is_none() {
        SAVED.set(Some(Saved {
            text: buffer::line().to_vec(),
            point: buffer::rl_point.get(),
            undo: rl_undo_list.replace(null_mut()),
        }));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_maybe_unsave_line() -> c_int {
    if let Some(s) = SAVED.get_mut().take() {
        buffer::set_line(&s.text);
        crate::undo::rl_free_undo_list();
        rl_undo_list.set(s.undo);
        buffer::set_point(buffer::end());
        let _ = s.point;
        return 0;
    }
    rl_ding();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_maybe_replace_line() -> c_int {
    let o = history_offset.get();
    let e = history::entry_at(o);
    if !e.is_null() && !rl_undo_list.get().is_null() && (*e).data != rl_undo_list.get() as histdata_t {
        let c = c_strdup(buffer::line());
        let old = history::replace_history_entry(o, c, rl_undo_list.get() as histdata_t);
        cfree(c);
        // the old entry's data was an undo list we no longer reference
        let d = history::free_history_entry(old);
        let _ = d;
    }
    rl_undo_list.set(null_mut());
    0
}

/// Show history entry `e` as the current line.
unsafe fn load_entry(e: *mut HIST_ENTRY) {
    buffer::set_line(cstr((*e).line));
    rl_undo_list.set((*e).data as *mut UNDO_LIST);
    let p = if crate::vars::history_preserve_point.get() != 0 {
        buffer::rl_point.get().min(buffer::rl_end.get())
    } else {
        buffer::rl_end.get()
    };
    buffer::rl_point.set(p);
    if crate::vi::in_command_mode() {
        buffer::rl_point.set(0);
    }
    buffer::rl_mark.set(0);
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_previous_history(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_get_next_history(-count, key);
    }
    if count == 0 || history_length.get() == 0 {
        return 0;
    }
    if history_offset.get() == history_length.get() {
        rl_maybe_save_line();
    }
    rl_maybe_replace_line();
    let mut found: *mut HIST_ENTRY = null_mut();
    for _ in 0..count {
        let e = history::previous_history();
        if e.is_null() {
            break;
        }
        found = e;
    }
    if found.is_null() {
        rl_ding();
        // stay where we were
        let e = history::current_history();
        if !e.is_null() {
            rl_undo_list.set((*e).data as *mut UNDO_LIST);
        }
        return 0;
    }
    load_entry(found);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_next_history(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_get_previous_history(-count, key);
    }
    if count == 0 {
        return 0;
    }
    rl_maybe_replace_line();
    let mut e: *mut HIST_ENTRY = null_mut();
    for _ in 0..count {
        e = history::next_history();
        if e.is_null() {
            break;
        }
    }
    if e.is_null() {
        rl_maybe_unsave_line();
    } else {
        load_entry(e);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_beginning_of_history(_count: c_int, key: c_int) -> c_int {
    rl_get_previous_history(1 + history::where_history(), key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_end_of_history(_count: c_int, _key: c_int) -> c_int {
    rl_maybe_replace_line();
    history::using_history();
    rl_maybe_unsave_line();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_operate_and_get_next(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_operate_and_get_next");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_fetch_history(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_fetch_history");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_history() {
    for i in 0..history_length.get() {
        let e = history::entry_at(i);
        if !e.is_null() && !(*e).data.is_null() {
            let l = (*e).data as *mut UNDO_LIST;
            if l != rl_undo_list.get() {
                crate::undo::free_list(l);
            }
            (*e).data = null_mut();
        }
    }
    history::clear_history();
}

// ---- history arguments (bound in emacs; tests in tests/kill)

#[no_mangle]
pub unsafe extern "C" fn rl_yank_nth_arg(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_yank_nth_arg");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_yank_last_arg(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_yank_last_arg");
    0
}
