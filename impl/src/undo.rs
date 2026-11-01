//! The undo list (`rl_undo_list`, malloc'd UNDO_LIST nodes, newest first),
//! undo groups, rl_do_undo, undo/revert-line commands.
//! Owner: emacs workstream (undo area).
use crate::buffer;
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;
use crate::util::xmalloc;

#[no_mangle]
pub static rl_undo_list: CVar<*mut UNDO_LIST> = CVar::new(null_mut());

/// Nesting depth of rl_begin_undo_group (while > 0, inserts do not merge
/// across the group boundary).
static GROUP_LEVEL: CVar<c_int> = CVar::new(0);

unsafe fn push(what: undo_code, start: c_int, end: c_int, text: *mut c_char) {
    let n = xmalloc(core::mem::size_of::<UNDO_LIST>()) as *mut UNDO_LIST;
    (*n).next = rl_undo_list.get();
    (*n).start = start;
    (*n).end = end;
    (*n).text = text;
    (*n).what = what;
    rl_undo_list.set(n);
}

/// Called by buffer::insert_at.
pub fn record_insert(start: usize, end: usize) {
    if is_state(RL_STATE_UNDOING) {
        return;
    }
    unsafe {
        let top = rl_undo_list.get();
        // single-byte insertions extend the previous insertion (up to 20 bytes)
        if end - start == 1
            && !top.is_null()
            && (*top).what == UNDO_INSERT
            && (*top).end == start as c_int
            && (*top).end - (*top).start < 20
        {
            (*top).end = end as c_int;
            return;
        }
        push(UNDO_INSERT, start as c_int, end as c_int, null_mut());
    }
}

/// Called by buffer::delete with the deleted text.
pub fn record_delete(start: usize, end: usize, text: &[u8]) {
    if is_state(RL_STATE_UNDOING) {
        return;
    }
    unsafe { push(UNDO_DELETE, start as c_int, end as c_int, c_strdup(text)) }
}

#[no_mangle]
pub unsafe extern "C" fn rl_add_undo(what: undo_code, start: c_int, end: c_int, text: *mut c_char) {
    push(what, start, end, text);
}

/// Free a whole undo list.
pub unsafe fn free_list(mut l: *mut UNDO_LIST) {
    while !l.is_null() {
        let n = (*l).next;
        cfree((*l).text);
        cfree(l);
        l = n;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_free_undo_list() {
    let l = rl_undo_list.replace(null_mut());
    crate::histcmds::forget_undo_list(l);
    free_list(l);
}

#[no_mangle]
pub unsafe extern "C" fn rl_begin_undo_group() -> c_int {
    push(UNDO_BEGIN, 0, 0, null_mut());
    GROUP_LEVEL.add(1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_end_undo_group() -> c_int {
    push(UNDO_END, 0, 0, null_mut());
    GROUP_LEVEL.add(-1);
    0
}

/// Undo one entry (or one group).  Returns 0 when the list is empty.
#[no_mangle]
pub unsafe extern "C" fn rl_do_undo() -> c_int {
    if rl_undo_list.get().is_null() {
        return 0;
    }
    let mut waiting_for_begin = 0;
    let entry = crate::histcmds::current_entry_sharing(rl_undo_list.get());
    set_state(RL_STATE_UNDOING);
    loop {
        let u = rl_undo_list.get();
        if u.is_null() {
            break;
        }
        rl_undo_list.set((*u).next);
        let start = (*u).start.max(0) as usize;
        let end = (*u).end.max(0) as usize;
        match (*u).what {
            UNDO_DELETE => {
                let t = cstr((*u).text).to_vec();
                buffer::insert_at(start.min(buffer::end()), &t);
                buffer::set_point(start.min(buffer::end()) + t.len());
            }
            UNDO_INSERT => {
                buffer::delete(start, end);
                buffer::set_point(start.min(buffer::end()));
            }
            UNDO_END => waiting_for_begin += 1,
            UNDO_BEGIN => {
                if waiting_for_begin > 0 {
                    waiting_for_begin -= 1;
                }
            }
            _ => {}
        }
        // whoever shares the list (history entries, the saved line) now
        // starts at the next entry
        crate::histcmds::replace_undo_list(u, (*u).next);
        cfree((*u).text);
        cfree(u);
        if waiting_for_begin == 0 {
            break;
        }
    }
    unset_state(RL_STATE_UNDOING);
    if let Some(o) = entry {
        crate::histcmds::entry_undone(o);
    }
    1
}

#[no_mangle]
pub unsafe extern "C" fn rl_modifying(start: c_int, end: c_int) -> c_int {
    let (start, end) = if start > end { (end, start) } else { (start, end) };
    if start == end {
        return 0;
    }
    let s = (start.max(0) as usize).min(buffer::end());
    let e = (end.max(0) as usize).min(buffer::end());
    let text = buffer::line()[s..e].to_vec();
    rl_begin_undo_group();
    push(UNDO_DELETE, s as c_int, e as c_int, c_strdup(&text));
    push(UNDO_INSERT, s as c_int, e as c_int, null_mut());
    rl_end_undo_group();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_undo_command(count: c_int, _key: c_int) -> c_int {
    if count < 0 {
        return 0;
    }
    for _ in 0..count {
        if rl_do_undo() == 0 {
            crate::terminal::rl_ding();
            break;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_revert_line(_count: c_int, _key: c_int) -> c_int {
    if rl_undo_list.get().is_null() {
        crate::terminal::rl_ding();
        return 0;
    }
    while !rl_undo_list.get().is_null() {
        rl_do_undo();
    }
    if crate::readline::rl_editing_mode.get() == crate::readline::VI_MODE {
        // vi `U`: point and mark at the start of the restored line
        buffer::rl_point.set(0);
        buffer::rl_mark.set(0);
    }
    0
}
