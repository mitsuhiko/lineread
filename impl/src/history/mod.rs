//! The history list (history(3) "History List Management", "Information
//! About the History List", "Moving Around the History List").
//!
//! Storage is a malloc'd NULL-terminated `HIST_ENTRY **` (returned as is by
//! `history_list`).  The exported `history_length`, `history_offset`,
//! `history_base`, `history_max_entries` are the source of truth.
//! Entries, their lines and timestamps are malloc'd (the application frees
//! removed entries with `free_history_entry`).
//! Owner: history workstream.
use crate::prelude::*;
use crate::util::{xmalloc, xrealloc};

pub mod expand;
pub mod file;
pub mod search;

#[no_mangle]
pub static history_base: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static history_length: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static history_max_entries: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static history_offset: CVar<c_int> = CVar::new(0);
/// Old name of history_max_entries, kept in sync.
#[no_mangle]
pub static max_input_history: CVar<c_int> = CVar::new(0);

/// The array (`the_history`) and its allocated size in slots.
pub static THE_HISTORY: CVar<*mut *mut HIST_ENTRY> = CVar::new(null_mut());
pub static HISTORY_SIZE: CVar<c_int> = CVar::new(0);
pub static HISTORY_STIFLED: CVar<c_int> = CVar::new(0);

/// Entry pointer at index `i` (0-based), NULL when out of range.
pub fn entry_at(i: c_int) -> *mut HIST_ENTRY {
    let h = THE_HISTORY.get();
    if h.is_null() || i < 0 || i >= history_length.get() {
        return null_mut();
    }
    unsafe { *h.add(i as usize) }
}

/// The line of entry `i` as bytes.
pub fn line_at<'a>(i: c_int) -> Option<&'a [u8]> {
    let e = entry_at(i);
    if e.is_null() {
        None
    } else {
        unsafe { Some(cstr((*e).line)) }
    }
}

/// Copy of a timestamp string, padded with an extra NUL so that
/// `history_get_time` may look one byte past an empty timestamp.
pub unsafe fn dup_timestamp(ts: &[u8]) -> *mut c_char {
    let p = xmalloc(ts.len() + 2) as *mut u8;
    ptr::copy_nonoverlapping(ts.as_ptr(), p, ts.len());
    *p.add(ts.len()) = 0;
    *p.add(ts.len() + 1) = 0;
    p as *mut c_char
}

/// Timestamp for a new entry: the history comment character followed by
/// the decimal time.  With the default comment character (NUL) it reads as
/// "" but `history_get_time` sees it.
pub unsafe fn now_timestamp() -> *mut c_char {
    let t = libc::time(null_mut());
    let mut s = vec![expand::history_comment_char.get() as u8];
    s.extend_from_slice(t.to_string().as_bytes());
    let p = xmalloc(s.len() + 1) as *mut u8;
    ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
    *p.add(s.len()) = 0;
    p as *mut c_char
}

/// Make room for one more entry plus the NULL terminator: the first
/// allocation has 502 slots (max + 2 when stifled), then grows by 256.
unsafe fn ensure_capacity() {
    let len = history_length.get();
    let size = HISTORY_SIZE.get();
    let new = if size == 0 || THE_HISTORY.get().is_null() {
        if HISTORY_STIFLED.get() != 0 && history_max_entries.get() > 0 {
            history_max_entries.get() + 2
        } else {
            502
        }
        .max(len + 2)
    } else if len + 1 >= size {
        size + 256
    } else {
        return;
    };
    let p = xrealloc(THE_HISTORY.get() as *mut c_void, new as usize * core::mem::size_of::<*mut HIST_ENTRY>());
    THE_HISTORY.set(p as *mut *mut HIST_ENTRY);
    HISTORY_SIZE.set(new);
}

/// Remove the first `n` entries (freeing them).
unsafe fn drop_oldest(n: c_int) {
    let h = THE_HISTORY.get();
    let len = history_length.get();
    let n = n.min(len);
    if n <= 0 {
        return;
    }
    for i in 0..n {
        free_entry(*h.add(i as usize));
    }
    ptr::copy(h.add(n as usize), h, (len - n) as usize + 1);
    history_length.set(len - n);
}

unsafe fn free_entry(e: *mut HIST_ENTRY) {
    if e.is_null() {
        return;
    }
    cfree((*e).line);
    cfree((*e).timestamp);
    cfree(e);
}

/// Append an entry (stifling applies).  Used by add_history and the file
/// reader.
pub unsafe fn push_entry(e: *mut HIST_ENTRY) {
    if HISTORY_STIFLED.get() != 0 && history_length.get() >= history_max_entries.get() {
        if history_max_entries.get() <= 0 {
            free_entry(e);
            return;
        }
        drop_oldest(history_length.get() - history_max_entries.get() + 1);
        history_base.add(1);
        // observed: the reference's notion of the array size shrinks here
        HISTORY_SIZE.set(HISTORY_SIZE.get() - 1);
    }
    ensure_capacity();
    let len = history_length.get();
    let h = THE_HISTORY.get();
    *h.add(len as usize) = e;
    *h.add(len as usize + 1) = null_mut();
    history_length.set(len + 1);
}

#[no_mangle]
pub unsafe extern "C" fn using_history() {
    history_offset.set(history_length.get());
}

#[no_mangle]
pub unsafe extern "C" fn history_get_history_state() -> *mut HISTORY_STATE {
    let s = xmalloc(core::mem::size_of::<HISTORY_STATE>()) as *mut HISTORY_STATE;
    (*s).entries = THE_HISTORY.get();
    (*s).offset = history_offset.get();
    (*s).length = history_length.get();
    (*s).size = HISTORY_SIZE.get();
    (*s).flags = if HISTORY_STIFLED.get() != 0 { HS_STIFLED } else { 0 };
    s
}

#[no_mangle]
pub unsafe extern "C" fn history_set_history_state(s: *mut HISTORY_STATE) {
    if s.is_null() {
        return;
    }
    THE_HISTORY.set((*s).entries);
    history_offset.set((*s).offset);
    history_length.set((*s).length);
    HISTORY_SIZE.set((*s).size);
    if (*s).flags & HS_STIFLED != 0 {
        HISTORY_STIFLED.set(1);
    }
}

pub unsafe fn new_entry(line: &[u8], ts: *mut c_char, data: histdata_t) -> *mut HIST_ENTRY {
    let e = xmalloc(core::mem::size_of::<HIST_ENTRY>()) as *mut HIST_ENTRY;
    (*e).line = c_strdup(line);
    (*e).timestamp = ts;
    (*e).data = data;
    e
}

#[no_mangle]
pub unsafe extern "C" fn add_history(line: *const c_char) {
    let e = new_entry(cstr(line), now_timestamp(), null_mut());
    push_entry(e);
}

#[no_mangle]
pub unsafe extern "C" fn add_history_time(ts: *const c_char) {
    if ts.is_null() || history_length.get() == 0 {
        return;
    }
    let e = entry_at(history_length.get() - 1);
    cfree((*e).timestamp);
    (*e).timestamp = dup_timestamp(cstr(ts));
}

#[no_mangle]
pub unsafe extern "C" fn remove_history(which: c_int) -> *mut HIST_ENTRY {
    let len = history_length.get();
    if which < 0 || which >= len || THE_HISTORY.get().is_null() {
        return null_mut();
    }
    let h = THE_HISTORY.get();
    let e = *h.add(which as usize);
    ptr::copy(h.add(which as usize + 1), h.add(which as usize), (len - which) as usize);
    history_length.set(len - 1);
    e
}

#[no_mangle]
pub unsafe extern "C" fn remove_history_range(first: c_int, last: c_int) -> *mut *mut HIST_ENTRY {
    let len = history_length.get();
    if THE_HISTORY.get().is_null() || first < 0 || last < first || first >= len || last >= len {
        return null_mut();
    }
    let n = (last - first + 1) as usize;
    let h = THE_HISTORY.get();
    let out = xmalloc((n + 1) * core::mem::size_of::<*mut HIST_ENTRY>()) as *mut *mut HIST_ENTRY;
    for i in 0..n {
        *out.add(i) = *h.add(first as usize + i);
    }
    *out.add(n) = null_mut();
    ptr::copy(h.add(last as usize + 1), h.add(first as usize), (len - last) as usize);
    history_length.set(len - n as c_int);
    out
}

#[no_mangle]
pub unsafe extern "C" fn alloc_history_entry(line: *mut c_char, ts: *mut c_char) -> *mut HIST_ENTRY {
    let ts = if ts.is_null() { null_mut() } else { dup_timestamp(cstr(ts)) };
    let e = xmalloc(core::mem::size_of::<HIST_ENTRY>()) as *mut HIST_ENTRY;
    (*e).line = if line.is_null() { null_mut() } else { c_strdup(cstr(line)) };
    (*e).timestamp = ts;
    (*e).data = null_mut();
    e
}

#[no_mangle]
pub unsafe extern "C" fn copy_history_entry(e: *mut HIST_ENTRY) -> *mut HIST_ENTRY {
    if e.is_null() {
        return null_mut();
    }
    let n = xmalloc(core::mem::size_of::<HIST_ENTRY>()) as *mut HIST_ENTRY;
    (*n).line = crate::util::c_strdup_ptr((*e).line);
    (*n).timestamp = if (*e).timestamp.is_null() { null_mut() } else { dup_raw_timestamp((*e).timestamp) };
    (*n).data = (*e).data;
    n
}

/// Copy a timestamp including our hidden "NUL + digits" form.
unsafe fn dup_raw_timestamp(ts: *const c_char) -> *mut c_char {
    let b = ts as *const u8;
    if *b == 0 && *b.add(1) != 0 {
        // NUL followed by the time digits
        let digits = cstr(ts.add(1));
        let p = xmalloc(digits.len() + 2) as *mut u8;
        *p = 0;
        ptr::copy_nonoverlapping(digits.as_ptr(), p.add(1), digits.len());
        *p.add(digits.len() + 1) = 0;
        return p as *mut c_char;
    }
    dup_timestamp(cstr(ts))
}

#[no_mangle]
pub unsafe extern "C" fn free_history_entry(e: *mut HIST_ENTRY) -> histdata_t {
    if e.is_null() {
        return null_mut();
    }
    let d = (*e).data;
    cfree((*e).line);
    cfree((*e).timestamp);
    cfree(e);
    d
}

#[no_mangle]
pub unsafe extern "C" fn replace_history_entry(which: c_int, line: *const c_char, data: histdata_t) -> *mut HIST_ENTRY {
    if which < 0 || which >= history_length.get() {
        return null_mut();
    }
    let h = THE_HISTORY.get();
    let old = *h.add(which as usize);
    let ts = if (*old).timestamp.is_null() { null_mut() } else { dup_raw_timestamp((*old).timestamp) };
    *h.add(which as usize) = new_entry(cstr(line), ts, data);
    old
}

#[no_mangle]
pub unsafe extern "C" fn clear_history() {
    let h = THE_HISTORY.get();
    for i in 0..history_length.get() {
        free_entry(*h.add(i as usize));
        *h.add(i as usize) = null_mut();
    }
    history_length.set(0);
    history_offset.set(0);
    history_base.set(1);
}

#[no_mangle]
pub unsafe extern "C" fn stifle_history(max: c_int) {
    let max = max.max(0);
    let len = history_length.get();
    if len > max {
        let n = len - max;
        drop_oldest(n);
        history_base.set(n);
    }
    HISTORY_STIFLED.set(1);
    history_max_entries.set(max);
    max_input_history.set(max);
}

#[no_mangle]
pub unsafe extern "C" fn unstifle_history() -> c_int {
    if HISTORY_STIFLED.get() != 0 {
        HISTORY_STIFLED.set(0);
        history_max_entries.get()
    } else {
        -history_max_entries.get()
    }
}

#[no_mangle]
pub unsafe extern "C" fn history_is_stifled() -> c_int {
    HISTORY_STIFLED.get()
}

#[no_mangle]
pub unsafe extern "C" fn history_list() -> *mut *mut HIST_ENTRY {
    THE_HISTORY.get()
}

#[no_mangle]
pub unsafe extern "C" fn where_history() -> c_int {
    history_offset.get()
}

#[no_mangle]
pub unsafe extern "C" fn current_history() -> *mut HIST_ENTRY {
    entry_at(history_offset.get())
}

#[no_mangle]
pub unsafe extern "C" fn history_get(offset: c_int) -> *mut HIST_ENTRY {
    entry_at(offset.wrapping_sub(history_base.get()))
}

#[no_mangle]
pub unsafe extern "C" fn history_get_time(e: *mut HIST_ENTRY) -> time_t {
    if e.is_null() || (*e).timestamp.is_null() {
        return 0;
    }
    let ts = (*e).timestamp as *const u8;
    if *ts as c_char != expand::history_comment_char.get() {
        return 0;
    }
    if *ts == 0 && *ts.add(1) == 0 {
        return 0;
    }
    *errno_ptr() = 0;
    let t = libc::strtol(ts.add(1) as *const c_char, null_mut(), 10);
    if *errno_ptr() == libc::ERANGE {
        return 0;
    }
    t as time_t
}

#[cfg(target_os = "macos")]
unsafe fn errno_ptr() -> *mut c_int {
    libc::__error()
}
#[cfg(not(target_os = "macos"))]
unsafe fn errno_ptr() -> *mut c_int {
    libc::__errno_location()
}

#[no_mangle]
pub unsafe extern "C" fn history_total_bytes() -> c_int {
    let mut n = 0usize;
    for i in 0..history_length.get() {
        let e = entry_at(i);
        n += cstr((*e).line).len() + cstr((*e).timestamp).len();
    }
    n as c_int
}

#[no_mangle]
pub unsafe extern "C" fn history_set_pos(pos: c_int) -> c_int {
    if pos < 0 || pos > history_length.get() || THE_HISTORY.get().is_null() {
        return 0;
    }
    history_offset.set(pos);
    1
}

#[no_mangle]
pub unsafe extern "C" fn previous_history() -> *mut HIST_ENTRY {
    let o = history_offset.get();
    if o <= 0 || history_length.get() == 0 {
        return null_mut();
    }
    history_offset.set(o - 1);
    entry_at(o - 1)
}

#[no_mangle]
pub unsafe extern "C" fn next_history() -> *mut HIST_ENTRY {
    let o = history_offset.get();
    if o >= history_length.get() {
        return null_mut();
    }
    history_offset.set(o + 1);
    entry_at(o + 1)
}
