//! The kill ring and kill/yank commands.
//! Owner: emacs workstream (kill area).
use crate::buffer::{self, end, line, point, set_point};
use crate::commands::movement::{backward_word_pos, forward_word_pos};
use crate::prelude::*;
use crate::terminal::rl_ding;

const MAX_KILLS: usize = 10;

/// Newest last.
static RING: CVar<Vec<Vec<u8>>> = CVar::new(Vec::new());
/// Index (from the newest) of the entry yanked last (yank-pop rotates).
static YANK_INDEX: CVar<usize> = CVar::new(0);
/// The current command killed text / the previous one did.
static THIS_KILL: CVar<bool> = CVar::new(false);
static LAST_KILL: CVar<bool> = CVar::new(false);
/// The previous command was a yank (yank-pop is allowed).
static THIS_YANK: CVar<bool> = CVar::new(false);
static LAST_YANK: CVar<bool> = CVar::new(false);

/// Is the kill ring empty (yank would abort)?
pub fn kill_ring_empty() -> bool {
    RING.get_mut().is_empty()
}

/// Was the previous command a yank (so yank-pop can work)?
pub fn last_command_was_yank() -> bool {
    LAST_YANK.get() && !RING.get_mut().is_empty()
}

/// Called after every command by the key loop.
pub fn after_command() {
    // the kill sequence survives a command run while input is pushed back
    // (e.g. typeahead inserted by self-insert)
    if THIS_KILL.replace(false) {
        LAST_KILL.set(true);
    } else if crate::input::rl_pending_input.get() == 0 {
        LAST_KILL.set(false);
    }
    LAST_YANK.set(THIS_YANK.replace(false));
}

/// Called by vi commands before they kill.  Nothing to do: in vi mode a
/// kill directly after another kill replaces the kill-ring text (see
/// add_kill; observed: `dw dw` leaves one entry), other kills get a new
/// entry.
pub fn start_new_kill() {}

/// Add text to the kill ring: appended to (or, when `prepend`, put in
/// front of) the last kill if the previous command killed too.
pub fn add_kill(text: &[u8], prepend: bool) {
    let ring = RING.get_mut();
    if LAST_KILL.get() && !ring.is_empty() {
        let last = ring.last_mut().unwrap();
        if crate::readline::rl_editing_mode.get() == crate::readline::VI_MODE {
            // vi: consecutive kills replace the text instead of growing it
            *last = text.to_vec();
        } else if prepend {
            let mut t = text.to_vec();
            t.extend_from_slice(last);
            *last = t;
        } else {
            last.extend_from_slice(text);
        }
    } else {
        if ring.len() >= MAX_KILLS {
            ring.remove(0);
        }
        ring.push(text.to_vec());
    }
    THIS_KILL.set(true);
    // a kill continues an ongoing kill sequence within the same command
    LAST_KILL.set(true);
    YANK_INDEX.set(0);
}

/// Line and word kill commands leave the mark at point in emacs mode.
fn mark_at_point() {
    if crate::readline::rl_editing_mode.get() == crate::readline::EMACS_MODE {
        buffer::rl_mark.set(buffer::rl_point.get());
    }
}

/// Kill `[from, to)`; text before point (from > to in the call) is
/// prepended to an accumulating kill.
#[no_mangle]
pub unsafe extern "C" fn rl_kill_text(from: c_int, to: c_int) -> c_int {
    if from == to {
        THIS_KILL.set(true);
        return 0;
    }
    let prepend = from > to;
    let (a, b) = if from < to { (from, to) } else { (to, from) };
    let a = (a.max(0) as usize).min(end());
    let b = (b.max(0) as usize).min(end());
    let text = line()[a..b].to_vec();
    add_kill(&text, prepend);
    buffer::delete(a, b);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_kill_line(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_backward_kill_line(1, key);
    }
    let p = point();
    if p < end() {
        rl_kill_text(p as c_int, end() as c_int);
    } else {
        THIS_KILL.set(true);
    }
    set_point(p);
    mark_at_point();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_kill_line(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_kill_line(1, key);
    }
    let p = point();
    if p == 0 {
        rl_ding();
        return 0;
    }
    rl_kill_text(p as c_int, 0);
    set_point(0);
    mark_at_point();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_kill_full_line(_count: c_int, _key: c_int) -> c_int {
    crate::undo::rl_begin_undo_group();
    set_point(0);
    rl_kill_text(0, end() as c_int);
    set_point(0);
    crate::undo::rl_end_undo_group();
    mark_at_point();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_kill_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_backward_kill_word(-count, key);
    }
    let p = point();
    let to = forward_word_pos(p, count as usize);
    if to != p {
        rl_kill_text(p as c_int, to as c_int);
    }
    set_point(p);
    mark_at_point();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_kill_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_kill_word(-count, key);
    }
    let p = point();
    let from = backward_word_pos(p, count as usize);
    if from != p {
        rl_kill_text(p as c_int, from as c_int);
    }
    set_point(from);
    mark_at_point();
    0
}

/// Start of the `count`th whitespace (or `extra`) delimited word before p.
fn unix_word_start(mut p: usize, count: usize, extra: &[u8]) -> usize {
    let l = line();
    let is_sep = |c: u8| c == b' ' || c == b'\t' || extra.contains(&c);
    for _ in 0..count {
        while p > 0 && is_sep(l[p - 1]) {
            p -= 1;
        }
        while p > 0 && !is_sep(l[p - 1]) {
            p -= 1;
        }
    }
    p
}

#[no_mangle]
pub unsafe extern "C" fn rl_unix_word_rubout(count: c_int, _key: c_int) -> c_int {
    let p = point();
    if p == 0 {
        rl_ding();
        return 0;
    }
    let from = unix_word_start(p, count.max(1) as usize, b"");
    rl_kill_text(p as c_int, from as c_int);
    set_point(from);
    mark_at_point();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_unix_filename_rubout(count: c_int, _key: c_int) -> c_int {
    let p = point();
    if p == 0 {
        rl_ding();
        return 0;
    }
    let l = line();
    let mut q = p;
    for _ in 0..count.max(1) {
        while q > 0 && (l[q - 1] == b' ' || l[q - 1] == b'\t') {
            q -= 1;
        }
        // trailing slashes belong to the word
        while q > 0 && l[q - 1] == b'/' {
            q -= 1;
        }
        while q > 0 && !(l[q - 1] == b' ' || l[q - 1] == b'\t' || l[q - 1] == b'/') {
            q -= 1;
        }
    }
    rl_kill_text(p as c_int, q as c_int);
    set_point(q);
    mark_at_point();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_unix_line_discard(_count: c_int, _key: c_int) -> c_int {
    let p = point();
    if p == 0 {
        rl_ding();
        return 0;
    }
    rl_kill_text(p as c_int, 0);
    set_point(0);
    mark_at_point();
    0
}

/// Copy `[a, b)` to the kill ring (appending to an ongoing kill).
fn copy_to_kill(a: usize, b: usize) {
    let (a, b) = (a.min(b).min(end()), b.max(a).min(end()));
    if a == b {
        return;
    }
    let text = line()[a..b].to_vec();
    add_kill(&text, false);
}

#[no_mangle]
pub unsafe extern "C" fn rl_copy_region_to_kill(_count: c_int, _key: c_int) -> c_int {
    copy_to_kill(buffer::mark(), point());
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_kill_region(_count: c_int, _key: c_int) -> c_int {
    let (a, b) = (buffer::mark().min(point()), buffer::mark().max(point()));
    rl_kill_text(a as c_int, b as c_int);
    set_point(a);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_copy_forward_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_copy_backward_word(-count, key);
    }
    let n = count as usize;
    let to = forward_word_pos(point(), n);
    copy_to_kill(backward_word_pos(to, n), to);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_copy_backward_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_copy_forward_word(-count, key);
    }
    let n = count as usize;
    let from = backward_word_pos(point(), n);
    copy_to_kill(from, forward_word_pos(from, n));
    0
}

/// The kill-ring entry `back` steps from the newest.
fn ring_entry(back: usize) -> Option<Vec<u8>> {
    let r = RING.get_mut();
    if r.is_empty() {
        return None;
    }
    let i = r.len() - 1 - (back % r.len());
    Some(r[i].clone())
}

#[no_mangle]
pub unsafe extern "C" fn rl_yank(_count: c_int, _key: c_int) -> c_int {
    let Some(t) = ring_entry(0) else {
        // nothing to yank
        crate::commands::misc::abort_internal();
        return 1;
    };
    YANK_INDEX.set(0);
    buffer::set_mark(point());
    buffer::insert(&t);
    THIS_YANK.set(true);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_yank_pop(_count: c_int, _key: c_int) -> c_int {
    if !LAST_YANK.get() || RING.get_mut().is_empty() {
        crate::commands::misc::abort_internal();
        return 1;
    }
    let prev = ring_entry(YANK_INDEX.get()).unwrap_or_default();
    let p = point();
    let start = p.saturating_sub(prev.len());
    if line()[start..p] != prev[..] {
        crate::commands::misc::abort_internal();
        return 1;
    }
    buffer::delete(start, p);
    set_point(start);
    YANK_INDEX.set(YANK_INDEX.get() + 1);
    let t = ring_entry(YANK_INDEX.get()).unwrap_or_default();
    buffer::set_mark(start);
    buffer::insert(&t);
    THIS_YANK.set(true);
    0
}

// rl_yank_nth_arg / rl_yank_last_arg live in histcmds.rs (history workstream).
