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

/// history-preserve-point: the point remembered when moving into the
/// history (-1: none).
static SAVED_POINT: CVar<c_int> = CVar::new(-1);
/// operate-and-get-next: the logical history number to show next.
static NEXT_LINE: CVar<Option<c_int>> = CVar::new(None);

/// Line setup: start at the end of the history list (or at the entry
/// requested by operate-and-get-next).
pub fn line_started() {
    unsafe {
        history::using_history();
        // (the history-preserve-point position carries over to the next
        // line, observed)
        if let Some(l) = NEXT_LINE.get_mut().take() {
            let i = l - history::history_base.get();
            if i >= 0 && i < history_length.get() {
                goto_history(i);
            }
        }
    }
}

fn remember_point() {
    let (p, e) = (buffer::rl_point.get(), buffer::rl_end.get());
    if SAVED_POINT.get() == -1 && (p != 0 || e != 0) {
        SAVED_POINT.set(if p == e { -1 } else { p });
    }
}

/// Clear `data` of entries that point to the undo list `l` (it is being
/// freed).
pub fn forget_undo_list(l: *mut UNDO_LIST) {
    replace_undo_list(l, null_mut());
}

/// The current history entry (if the line being edited is one) when it
/// shares the undo list `l`.
pub fn current_entry_sharing(l: *mut UNDO_LIST) -> Option<c_int> {
    let o = history_offset.get();
    let e = history::entry_at(o);
    if l.is_null() || e.is_null() || unsafe { (*e).data } != l as histdata_t {
        return None;
    }
    Some(o)
}

/// After an undo on history entry `o`: the entry shows the line as it is
/// now (with the remaining undo list), observed.
pub unsafe fn entry_undone(o: c_int) {
    let c = c_strdup(buffer::line());
    let old = history::replace_history_entry(o, c, rl_undo_list.get() as histdata_t);
    cfree(c);
    let _ = history::free_history_entry(old);
}

/// References (history entries, the saved line) to the undo list `old`
/// now point to `new` (rl_do_undo releases the head of a shared list).
pub fn replace_undo_list(old: *mut UNDO_LIST, new: *mut UNDO_LIST) {
    if old.is_null() {
        return;
    }
    for i in 0..history_length.get() {
        let e = history::entry_at(i);
        unsafe {
            if !e.is_null() && (*e).data == old as histdata_t {
                (*e).data = new as histdata_t;
            }
        }
    }
    if let Some(s) = SAVED.get_mut() {
        if s.undo == old {
            s.undo = new;
        }
    }
}

/// The line was accepted: restore an edited history entry to its original
/// text (the result keeps the edits), drop the saved line.
pub fn line_finished() {
    unsafe {
        // history-preserve-point remembers where the accepted line had point
        let (p, e) = (buffer::rl_point.get(), buffer::rl_end.get());
        SAVED_POINT.set(if p == e { -1 } else { p });
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
        if crate::vars::revert_all_at_newline.get() != 0 {
            // every other edited entry goes back to its original text too
            let (text, pt, cur) = (buffer::line().to_vec(), buffer::rl_point.get(), rl_undo_list.get());
            for i in 0..history_length.get() {
                let e = history::entry_at(i);
                if e.is_null() || (*e).data.is_null() || (*e).data == cur as histdata_t {
                    continue;
                }
                buffer::set_line(cstr((*e).line));
                rl_undo_list.set((*e).data as *mut UNDO_LIST);
                crate::undo::rl_revert_line(1, 0);
                let c = c_strdup(buffer::line());
                history::free_history_entry(history::replace_history_entry(i, c, null_mut()));
                cfree(c);
                crate::undo::rl_free_undo_list();
            }
            rl_undo_list.set(cur);
            buffer::set_line(&text);
            buffer::rl_point.set(pt);
        }
        if let Some(s) = SAVED.get_mut().take() {
            if s.undo != rl_undo_list.get() {
                crate::undo::free_list(s.undo);
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_maybe_save_line() -> c_int {
    if SAVED.get_mut().is_none() {
        SAVED.set(Some(Saved {
            text: buffer::line().to_vec(),
            point: buffer::rl_point.get(),
            // the saved line shares the undo list (it stays active)
            undo: rl_undo_list.get(),
        }));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_maybe_unsave_line() -> c_int {
    if let Some(s) = SAVED.get_mut().take() {
        buffer::set_line(&s.text);
        // (the current list belongs to the history entry it was saved in)
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
    if !e.is_null() && (*e).data != rl_undo_list.get() as histdata_t {
        let c = c_strdup(buffer::line());
        let old = history::replace_history_entry(o, c, rl_undo_list.get() as histdata_t);
        cfree(c);
        // (the old entry's undo list may still be referenced elsewhere, e.g.
        // by the line being edited: it is not freed here)
        let _ = history::free_history_entry(old);
    }
    0
}

/// The text of the line saved when moving into the history, if any.
pub fn saved_line_text() -> Option<Vec<u8>> {
    SAVED.get_mut().as_ref().map(|s| s.text.clone())
}

/// Move to history position `i` (0..=history_length) the way the
/// navigation commands do: save the edited line when leaving the end of
/// the list, keep edits of the entry being left, load the new entry (or
/// the saved line at the end).  Point goes to the end of the line.
pub unsafe fn goto_history(i: c_int) {
    let len = history_length.get();
    let i = i.clamp(0, len);
    if history_offset.get() >= len {
        rl_maybe_save_line();
    }
    rl_maybe_replace_line();
    history_offset.set(i);
    if i >= len {
        rl_maybe_unsave_line();
    } else {
        let e = history::entry_at(i);
        buffer::set_line(cstr((*e).line));
        rl_undo_list.set((*e).data as *mut UNDO_LIST);
        buffer::rl_point.set(buffer::rl_end.get());
        if crate::readline::rl_editing_mode.get() == crate::readline::EMACS_MODE {
            buffer::rl_mark.set(0);
        }
    }
}

/// Show history entry `e` as the current line.
unsafe fn load_entry(e: *mut HIST_ENTRY) {
    buffer::set_line(cstr((*e).line));
    rl_undo_list.set((*e).data as *mut UNDO_LIST);
    let end = buffer::rl_end.get();
    let sp = SAVED_POINT.get();
    let p = if crate::vars::history_preserve_point.get() != 0 && sp != -1 { sp.min(end) } else { end };
    buffer::rl_point.set(p);
    if crate::vi::in_command_mode() {
        // vi command mode: point at the start, mark at the end
        buffer::rl_point.set(0);
        buffer::rl_mark.set(end);
    } else if crate::readline::rl_editing_mode.get() == crate::readline::EMACS_MODE {
        buffer::rl_mark.set(if p == end { 0 } else { end });
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_previous_history(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_get_next_history(-count, key);
    }
    if count == 0 || history_length.get() == 0 {
        return 0;
    }
    remember_point();
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
        // stay where we were; the edits went into the entry and the line
        // has no undo list any more (observed)
        if !history::current_history().is_null() {
            rl_undo_list.set(null_mut());
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
    remember_point();
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
pub unsafe extern "C" fn rl_operate_and_get_next(count: c_int, key: c_int) -> c_int {
    let l = if crate::numarg::rl_explicit_arg.get() != 0 {
        count
    } else {
        history::where_history() + history::history_base.get() + 1
    };
    NEXT_LINE.set(Some(l));
    crate::readline::rl_newline(1, key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_fetch_history(count: c_int, key: c_int) -> c_int {
    if crate::numarg::rl_explicit_arg.get() == 0 {
        return rl_beginning_of_history(count, key);
    }
    let i = count - history::history_base.get();
    if i < 0 || i >= history_length.get() {
        return rl_beginning_of_history(count, key);
    }
    goto_history(i);
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
    history::clear_entries();
}

// ---- history arguments (bound in emacs; tests in tests/kill)

/// Insert word `n` (history_arg_extract numbering, `'$'` = last) of the
/// entry `skip + 1` lines before the current history position.
unsafe fn yank_arg(n: c_int, skip: c_int) -> c_int {
    let pos = history::where_history();
    let idx = pos - 1 - skip;
    let line = if idx >= 0 { history::line_at(idx) } else { None };
    let Some(line) = line else {
        rl_ding();
        return -1;
    };
    let arg = match history::expand::arg_extract(n, n, line) {
        Some(a) if !a.is_empty() => a,
        _ => {
            rl_ding();
            return -1;
        }
    };
    crate::undo::rl_begin_undo_group();
    buffer::rl_mark.set(buffer::rl_point.get());
    buffer::insert(&arg);
    crate::undo::rl_end_undo_group();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_yank_nth_arg(count: c_int, _key: c_int) -> c_int {
    yank_arg(count, 0)
}

struct LastArg {
    skip: c_int,
    explicit: bool,
    count: c_int,
    direction: c_int,
    undo_needed: bool,
}
static LAST_ARG: CVar<LastArg> = CVar::new(LastArg { skip: 0, explicit: false, count: 0, direction: 1, undo_needed: false });

#[no_mangle]
pub unsafe extern "C" fn rl_yank_last_arg(count: c_int, _key: c_int) -> c_int {
    let st = LAST_ARG.get_mut();
    if !crate::dispatch::same_fn(crate::dispatch::rl_last_func.get(), rl_yank_last_arg) {
        st.skip = 0;
        st.explicit = crate::numarg::rl_explicit_arg.get() != 0;
        st.count = count;
        st.direction = 1;
    } else {
        if st.undo_needed {
            crate::undo::rl_do_undo();
        }
        if count < 0 {
            st.direction = -st.direction;
        }
        st.skip = (st.skip + st.direction).max(0);
    }
    let n = if st.explicit { st.count } else { b'$' as c_int };
    let skip = st.skip;
    let r = yank_arg(n, skip);
    LAST_ARG.get_mut().undo_needed = r == 0;
    r
}
