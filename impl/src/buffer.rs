//! The line buffer.  `rl_line_buffer` (malloc'd, NUL-terminated at
//! `rl_end`), `rl_point`, `rl_end`, `rl_mark` are the source of truth; the
//! application may change any of them between calls.  All text changes go
//! through `insert_text`/`delete_text` (which record undo information).
//! Owner: emacs workstream (core: coordinate changes).
use crate::prelude::*;
use crate::util::{xmalloc, xrealloc};

#[no_mangle]
pub static rl_line_buffer: CVar<*mut c_char> = CVar::new(null_mut());
#[no_mangle]
pub static rl_point: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_end: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_mark: CVar<c_int> = CVar::new(0);

/// Allocated size of rl_line_buffer (bytes, including room for the NUL).
pub static BUFFER_LEN: CVar<usize> = CVar::new(0);
const DEFAULT_BUFFER_SIZE: usize = 256;

pub fn point() -> usize {
    rl_point.get().clamp(0, rl_end.get().max(0)) as usize
}
pub fn end() -> usize {
    rl_end.get().max(0) as usize
}
pub fn set_point(p: usize) {
    rl_point.set(p as c_int);
}
pub fn set_mark(p: usize) {
    rl_mark.set(p as c_int);
}
pub fn mark() -> usize {
    rl_mark.get().clamp(0, rl_end.get().max(0)) as usize
}

/// Make sure the buffer exists and can hold `n` bytes plus a NUL.
pub fn ensure(n: usize) {
    unsafe {
        if rl_line_buffer.get().is_null() || BUFFER_LEN.get() == 0 {
            let size = (n + 1).max(DEFAULT_BUFFER_SIZE);
            let old = rl_line_buffer.get();
            let p = xmalloc(size) as *mut c_char;
            if !old.is_null() {
                // an application-provided buffer: copy what is there
                let l = libc::strlen(old).min(size - 1);
                ptr::copy_nonoverlapping(old, p, l);
                *p.add(l) = 0;
            } else {
                *p = 0;
            }
            rl_line_buffer.set(p);
            BUFFER_LEN.set(size);
        } else if n + 1 > BUFFER_LEN.get() {
            let mut size = BUFFER_LEN.get();
            while size < n + 1 {
                size *= 2;
            }
            let p = xrealloc(rl_line_buffer.get() as *mut c_void, size) as *mut c_char;
            rl_line_buffer.set(p);
            BUFFER_LEN.set(size);
        }
    }
}

/// The current line (`rl_line_buffer[0..rl_end]`).
pub fn line<'a>() -> &'a [u8] {
    ensure(end());
    unsafe { core::slice::from_raw_parts(rl_line_buffer.get() as *const u8, end()) }
}

/// The whole line as mutable bytes.
pub fn line_mut<'a>() -> &'a mut [u8] {
    ensure(end());
    unsafe { core::slice::from_raw_parts_mut(rl_line_buffer.get() as *mut u8, end()) }
}

fn terminate() {
    unsafe {
        *rl_line_buffer.get().add(end()) = 0;
    }
}

/// Replace the whole line (no undo), keeping point/mark within bounds.
pub fn set_line(text: &[u8]) {
    ensure(text.len());
    unsafe {
        ptr::copy_nonoverlapping(text.as_ptr(), rl_line_buffer.get() as *mut u8, text.len());
    }
    rl_end.set(text.len() as c_int);
    terminate();
    if rl_point.get() > rl_end.get() {
        rl_point.set(rl_end.get());
    }
    if rl_mark.get() > rl_end.get() {
        rl_mark.set(rl_end.get());
    }
}

/// Insert at point, move point after the text, record undo.
pub fn insert(text: &[u8]) -> c_int {
    let p = point();
    insert_at(p, text);
    rl_point.set((p + text.len()) as c_int);
    text.len() as c_int
}

/// Insert at `pos` without moving point; records undo.
pub fn insert_at(pos: usize, text: &[u8]) {
    if text.is_empty() {
        return;
    }
    let e = end();
    let pos = pos.min(e);
    ensure(e + text.len());
    unsafe {
        let b = rl_line_buffer.get() as *mut u8;
        ptr::copy(b.add(pos), b.add(pos + text.len()), e - pos);
        ptr::copy_nonoverlapping(text.as_ptr(), b.add(pos), text.len());
    }
    rl_end.set((e + text.len()) as c_int);
    terminate();
    crate::undo::record_insert(pos, pos + text.len());
}

/// Delete `[from, to)` (swapped/clamped), record undo; returns the number
/// of bytes deleted.  Point is not changed (callers adjust it).
pub fn delete(from: usize, to: usize) -> c_int {
    let (mut from, mut to) = if from > to { (to, from) } else { (from, to) };
    let e = end();
    to = to.min(e);
    from = from.min(to);
    if from == to {
        return 0;
    }
    let text = line()[from..to].to_vec();
    unsafe {
        let b = rl_line_buffer.get() as *mut u8;
        ptr::copy(b.add(to), b.add(from), e - to);
    }
    rl_end.set((e - (to - from)) as c_int);
    terminate();
    if rl_mark.get() > rl_end.get() {
        rl_mark.set(rl_end.get());
    }
    crate::undo::record_delete(from, to, &text);
    (to - from) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert_text(text: *const c_char) -> c_int {
    let t = cstr(text);
    if t.is_empty() {
        return 0;
    }
    insert(t)
}

#[no_mangle]
pub unsafe extern "C" fn rl_delete_text(from: c_int, to: c_int) -> c_int {
    let (from, to) = if from > to { (to, from) } else { (from, to) };
    let from = from.max(0) as usize;
    let to = to.max(0) as usize;
    let r = delete(from, to);
    if rl_point.get() > rl_end.get() {
        rl_point.set(rl_end.get());
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn rl_extend_line_buffer(len: c_int) {
    ensure(len.max(0) as usize);
}

#[no_mangle]
pub unsafe extern "C" fn rl_copy_text(from: c_int, to: c_int) -> *mut c_char {
    let (from, to) = if from > to { (to, from) } else { (from, to) };
    let e = end();
    let to = (to.max(0) as usize).min(e);
    let from = (from.max(0) as usize).min(to);
    c_strdup(&line()[from..to])
}

#[no_mangle]
pub unsafe extern "C" fn rl_replace_line(text: *const c_char, clear_undo: c_int) {
    let t = cstr(text).to_vec();
    set_line(&t);
    if clear_undo != 0 {
        crate::undo::rl_free_undo_list();
    }
}
