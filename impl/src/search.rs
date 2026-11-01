//! History searches while editing: incremental search (C-r / C-s),
//! non-incremental search (M-p / M-n and the -again variants) and the
//! prefix / substring history searches.
//! Owner: history workstream.
use crate::buffer;
use crate::display;
use crate::dispatch::same_fn;
use crate::histcmds;
use crate::history::{self, history_length, history_offset};
use crate::input::{self, set_state, unset_state};
use crate::keymap;
use crate::prelude::*;
use crate::terminal::rl_ding;
use crate::undo::rl_undo_list;

const ESC: c_int = 27;

/// The last incremental and non-incremental search strings.
static LAST_ISEARCH: CVar<Option<Vec<u8>>> = CVar::new(None);
static LAST_NONINC: CVar<Option<Vec<u8>>> = CVar::new(None);
/// history-search-*: the string and its length (the original point).
static HSEARCH: CVar<Vec<u8>> = CVar::new(Vec::new());

fn ignore_case() -> bool {
    crate::vars::search_ignore_case.get() != 0
}

fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if ignore_case() {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Does `needle` occur in `line` at `i`?
fn match_at(line: &[u8], i: isize, needle: &[u8]) -> bool {
    if i < 0 {
        return false;
    }
    let i = i as usize;
    i + needle.len() <= line.len() && bytes_eq(&line[i..i + needle.len()], needle)
}

fn find_in(line: &[u8], needle: &[u8], reverse: bool) -> Option<usize> {
    if needle.len() > line.len() {
        return None;
    }
    let n = line.len() - needle.len();
    if reverse {
        (0..=n).rev().find(|&i| bytes_eq(&line[i..i + needle.len()], needle))
    } else {
        (0..=n).find(|&i| bytes_eq(&line[i..i + needle.len()], needle))
    }
}

fn is_func(f: Option<rl_command_func_t>, g: rl_command_func_t) -> bool {
    same_fn(f, g)
}

/// Read the rest of a multibyte character starting with byte `c`.
unsafe fn read_mb_rest(c: u8) -> Vec<u8> {
    let mut v = vec![c];
    if !crate::mb::multibyte() || c < 0x80 {
        return v;
    }
    loop {
        let (wc, n) = crate::mb::decode_at(&v, 0);
        if wc.is_some() && n == v.len() {
            break;
        }
        // incomplete: keep reading continuation bytes
        if v.len() >= 6 || (wc.is_none() && !incomplete(&v)) {
            break;
        }
        let k = input::rl_read_key();
        if k < 0 {
            break;
        }
        v.push(k as u8);
    }
    v
}

fn incomplete(v: &[u8]) -> bool {
    // a UTF-8 style lead byte announcing more bytes than we have
    let need = match v[0] {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    };
    v.len() < need && v[1..].iter().all(|&b| (0x80..0xc0).contains(&b))
}

/// Read a bracketed paste (after its start sequence).
unsafe fn read_paste() -> Vec<u8> {
    const END: &[u8] = b"\x1b[201~";
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let c = input::rl_read_key();
        if c < 0 {
            break;
        }
        buf.push(c as u8);
        if buf.ends_with(END) {
            buf.truncate(buf.len() - END.len());
            break;
        }
    }
    buf
}

/// What a key (sequence) read inside a search means.
enum Key {
    Eof,
    /// a character to add to the search string
    Char(Vec<u8>),
    /// a bound function (with the keys that invoked it)
    Func(rl_command_func_t, Vec<u8>),
    /// anything else: ends the search, the keys are executed afterwards
    Other(Vec<u8>),
    /// an isearch terminator: ends the search, consumed
    Terminator,
}

fn terminators() -> Vec<u8> {
    match crate::vars::isearch_terminators.get_mut() {
        Some(v) => crate::bind::translate_keyseq_bytes(v).0,
        None => vec![ESC as u8, b'\n'],
    }
}

/// Read one key (sequence) inside the incremental search.
unsafe fn isearch_key() -> Key {
    let c = input::rl_read_key();
    if c < 0 {
        return Key::Eof;
    }
    let term = terminators();
    if term.contains(&(c as u8)) {
        if c == ESC {
            let t = crate::vars::keyseq_timeout.get();
            if !input::input_available(t.max(0) as i64 * 1000) {
                return Key::Terminator;
            }
        } else {
            return Key::Terminator;
        }
    }
    if c >= 0x80 {
        if crate::mb::multibyte() {
            return Key::Char(read_mb_rest(c as u8));
        }
        return Key::Other(vec![c as u8]);
    }
    resolve(c, (0x20..0x7f).contains(&c))
}

/// Resolve a key sequence starting with `c` in the current keymap.
/// `printable`: a single key that resolves to itself is a character.
unsafe fn resolve(c: c_int, printable: bool) -> Key {
    let mut keys = vec![c as u8];
    let mut map = keymap::current();
    let mut k = c;
    loop {
        let e = *keymap::entry(map, k as usize);
        match e.type_ {
            ISKMAP => {
                let sub = e.function.map;
                let other = *keymap::entry(sub, ANYOTHERKEY);
                let has_other = !other.is_null();
                if has_other {
                    let t = crate::vars::keyseq_timeout.get();
                    if t > 0 && !input::input_available(t as i64 * 1000) {
                        return fallback(other, keys, printable);
                    }
                }
                set_state(RL_STATE_MULTIKEY);
                let n = input::rl_read_key();
                unset_state(RL_STATE_MULTIKEY);
                if n < 0 {
                    return Key::Eof;
                }
                let ne = *keymap::entry(sub, n as usize);
                if ne.is_null() && ne.type_ == ISFUNC {
                    if has_other {
                        input::unread_keys(&[n as u8]);
                        return fallback(other, keys, printable);
                    }
                    keys.push(n as u8);
                    return Key::Other(keys);
                }
                keys.push(n as u8);
                map = sub;
                k = n;
            }
            ISFUNC => {
                if printable && keys.len() == 1 {
                    return Key::Char(keys);
                }
                return match e.function.func {
                    Some(f) => Key::Func(f, keys),
                    None => Key::Other(keys),
                };
            }
            _ => {
                if printable && keys.len() == 1 {
                    return Key::Char(keys);
                }
                return Key::Other(keys);
            }
        }
    }
}

unsafe fn fallback(other: KEYMAP_ENTRY, keys: Vec<u8>, printable: bool) -> Key {
    if printable && keys.len() == 1 {
        return Key::Char(keys);
    }
    if other.type_ == ISFUNC {
        if let Some(f) = other.function.func {
            return Key::Func(f, keys);
        }
    }
    Key::Other(keys)
}

// ---------------------------------------------------------------- isearch

struct Isearch {
    lines: Vec<Vec<u8>>,
    start: usize,
    hp: isize,
    sline: Vec<u8>,
    si: isize,
    lfl: usize,
    dir: isize,
    search: Vec<u8>,
    failed: bool,
    orig_line: Vec<u8>,
    orig_point: c_int,
    orig_mark: c_int,
    orig_undo: *mut UNDO_LIST,
}

impl Isearch {
    unsafe fn new(dir: isize) -> Isearch {
        let len = history_length.get().max(0) as usize;
        let mut lines: Vec<Vec<u8>> = (0..len).map(|i| history::line_at(i as c_int).unwrap_or(&[]).to_vec()).collect();
        let cur = buffer::line().to_vec();
        let last = histcmds::saved_line_text().unwrap_or_else(|| cur.clone());
        lines.push(last);
        let start = (history_offset.get().max(0) as usize).min(len);
        lines[start] = cur.clone();
        Isearch {
            lines,
            start,
            hp: start as isize,
            sline: cur.clone(),
            si: buffer::rl_point.get() as isize,
            lfl: start,
            dir,
            search: Vec::new(),
            failed: false,
            orig_line: cur,
            orig_point: buffer::rl_point.get(),
            orig_mark: buffer::rl_mark.get(),
            orig_undo: rl_undo_list.get(),
        }
    }

    fn prompt(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.push(b'(');
        if self.failed {
            p.extend_from_slice(b"failed ");
        }
        if self.dir < 0 {
            p.extend_from_slice(b"reverse-");
        }
        p.extend_from_slice(b"i-search)`");
        p.extend_from_slice(&self.search);
        p.extend_from_slice(b"': ");
        p
    }

    unsafe fn show(&self) {
        display::set_message(&self.prompt());
        display::redisplay();
    }

    /// Search from the current position (inclusive).
    unsafe fn search(&mut self) {
        let slen = self.search.len() as isize;
        if slen == 0 {
            self.failed = true;
            rl_ding();
            return;
        }
        let hlen = self.lines.len() as isize;
        loop {
            let limit = self.sline.len() as isize - slen + 1;
            while if self.dir < 0 { self.si >= 0 } else { self.si < limit } {
                if match_at(&self.sline, self.si, &self.search) {
                    self.found();
                    return;
                }
                self.si += self.dir;
            }
            self.hp += self.dir;
            if self.hp < 0 || self.hp >= hlen {
                self.failed = true;
                self.hp = self.lfl as isize;
                if self.si < 0 {
                    self.si = 0;
                }
                rl_ding();
                return;
            }
            self.sline = self.lines[self.hp as usize].clone();
            self.si = if self.dir < 0 { self.sline.len() as isize - slen } else { 0 };
        }
    }

    unsafe fn found(&mut self) {
        self.failed = false;
        self.lfl = self.hp as usize;
        buffer::set_line(&self.lines[self.lfl]);
        let p = (self.si.max(0) as usize).min(buffer::end());
        buffer::rl_point.set(p as c_int);
        buffer::rl_mark.set(((p + self.search.len()).min(buffer::end())) as c_int);
        crate::commands::text::rl_activate_mark();
    }

    /// Restore the line as it was when the search started.
    unsafe fn restore(&self) {
        buffer::set_line(&self.orig_line);
        rl_undo_list.set(self.orig_undo);
    }

    unsafe fn finish(&mut self) {
        display::clear_arg_message();
        crate::commands::text::rl_deactivate_mark();
        if !self.search.is_empty() {
            LAST_ISEARCH.set(Some(self.search.clone()));
        }
        let hp = if self.failed { self.lfl } else { self.hp as usize };
        let point = self.si.max(0) as usize;
        if hp == self.start {
            self.restore();
            buffer::rl_point.set(point.min(buffer::end()) as c_int);
        } else {
            let mark = buffer::rl_mark.get();
            self.restore();
            buffer::rl_point.set(self.orig_point);
            histcmds::goto_history(hp as c_int);
            buffer::rl_point.set(point.min(buffer::end()) as c_int);
            if crate::readline::rl_editing_mode.get() != crate::readline::EMACS_MODE {
                buffer::rl_mark.set(mark.min(buffer::rl_end.get()));
            }
        }
    }

    unsafe fn abort(&mut self) {
        display::clear_arg_message();
        crate::commands::text::rl_deactivate_mark();
        self.restore();
        buffer::rl_point.set(self.orig_point.min(buffer::rl_end.get()));
        buffer::rl_mark.set(self.orig_mark.min(buffer::rl_end.get()));
    }
}

unsafe fn isearch(count: c_int, dir: isize) -> c_int {
    let dir = if count < 0 { -dir } else { dir };
    let mut cx = Isearch::new(dir);
    crate::dispatch::keyseq_reset();
    set_state(RL_STATE_ISEARCH);
    cx.show();
    loop {
        set_state(RL_STATE_MOREINPUT);
        let k = isearch_key();
        unset_state(RL_STATE_MOREINPUT);
        match k {
            Key::Eof => {
                unset_state(RL_STATE_ISEARCH);
                crate::commands::text::rl_deactivate_mark();
                crate::readline::signal_eof();
                display::clear_arg_message();
                return -1;
            }
            Key::Terminator => {
                cx.finish();
                break;
            }
            Key::Other(keys) => {
                cx.finish();
                input::unread_keys(&keys);
                break;
            }
            Key::Char(ch) => {
                cx.search.extend_from_slice(&ch);
                cx.search();
            }
            Key::Func(f, keys) => {
                if is_func(Some(f), rl_reverse_search_history) || is_func(Some(f), rl_forward_search_history) {
                    let nd = if is_func(Some(f), rl_reverse_search_history) { -1 } else { 1 };
                    if cx.search.is_empty() {
                        match LAST_ISEARCH.get_mut().clone() {
                            Some(s) => {
                                cx.dir = nd;
                                cx.search = s;
                                cx.search();
                            }
                            None => {
                                rl_ding();
                            }
                        }
                    } else if nd != cx.dir {
                        cx.dir = nd;
                    } else {
                        cx.si += cx.dir;
                        cx.search();
                    }
                } else if is_func(Some(f), crate::commands::misc::rl_abort) {
                    cx.abort();
                    break;
                } else if is_func(Some(f), crate::commands::text::rl_rubout) {
                    if cx.search.is_empty() {
                        cx.failed = true;
                        rl_ding();
                    } else {
                        let n = cx.search.len();
                        let p = if crate::mb::multibyte() { crate::mb::prev_char(&cx.search, n) } else { n - 1 };
                        cx.search.truncate(p);
                        cx.search();
                    }
                } else if is_func(Some(f), crate::kill::rl_unix_word_rubout) {
                    // add the rest of the word after the match
                    let line = buffer::line().to_vec();
                    let mut e = (buffer::rl_point.get().max(0) as usize + cx.search.len()).min(line.len());
                    let s0 = e;
                    while e < line.len() && crate::mb::is_word_char(&line, e) {
                        e = crate::mb::next_char(&line, e);
                    }
                    if e > s0 {
                        cx.search.extend_from_slice(&line[s0..e]);
                        cx.search();
                    } else {
                        rl_ding();
                    }
                } else if is_func(Some(f), crate::kill::rl_yank) {
                    let line = buffer::line().to_vec();
                    let s0 = (buffer::rl_point.get().max(0) as usize + cx.search.len()).min(line.len());
                    if s0 < line.len() {
                        cx.search.extend_from_slice(&line[s0..]);
                        cx.search();
                    } else {
                        rl_ding();
                    }
                } else if is_func(Some(f), crate::commands::text::rl_quoted_insert) {
                    set_state(RL_STATE_MOREINPUT);
                    let c = input::rl_read_key();
                    unset_state(RL_STATE_MOREINPUT);
                    if c < 0 {
                        unset_state(RL_STATE_ISEARCH);
                        crate::commands::text::rl_deactivate_mark();
                        crate::readline::signal_eof();
                        display::clear_arg_message();
                        return -1;
                    }
                    let ch = read_mb_rest(c as u8);
                    cx.search.extend_from_slice(&ch);
                    cx.search();
                } else if is_func(Some(f), crate::commands::text::rl_bracketed_paste_begin) {
                    let p = read_paste();
                    if !p.is_empty() {
                        cx.search.extend_from_slice(&p);
                        cx.search();
                    }
                } else {
                    cx.finish();
                    input::unread_keys(&keys);
                    break;
                }
            }
        }
        cx.show();
    }
    unset_state(RL_STATE_ISEARCH);
    display::redisplay();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_reverse_search_history(count: c_int, _key: c_int) -> c_int {
    isearch(count, -1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_forward_search_history(count: c_int, _key: c_int) -> c_int {
    isearch(count, 1)
}

// ---------------------------------------------------------------- non-incremental

/// Search the history for `s` from the current position, `count` times.
/// Returns (entry, offset in line).
unsafe fn noninc_find(s: &[u8], dir: isize, count: c_int) -> Option<(c_int, usize)> {
    let (anchored, pat) = if s.first() == Some(&b'^') { (true, &s[1..]) } else { (false, s) };
    let len = history_length.get();
    let mut pos = history_offset.get().min(len);
    let mut found = None;
    for _ in 0..count.max(1) {
        let mut i = pos + dir as c_int;
        let mut hit = None;
        while i >= 0 && i < len {
            let line = history::line_at(i).unwrap_or(&[]);
            let m = if anchored {
                if line.len() >= pat.len() && bytes_eq(&line[..pat.len()], pat) {
                    Some(0)
                } else {
                    None
                }
            } else {
                find_in(line, pat, false)
            };
            if let Some(off) = m {
                hit = Some((i, off));
                break;
            }
            i += dir as c_int;
        }
        match hit {
            Some(h) => {
                pos = h.0;
                found = Some(h);
            }
            None => return None,
        }
    }
    found
}

/// Run a non-incremental search for `s`; on success move there.
unsafe fn noninc_do(s: &[u8], dir: isize, count: c_int) -> c_int {
    match noninc_find(s, dir, count) {
        Some((i, off)) => {
            histcmds::goto_history(i);
            buffer::rl_point.set(off.min(buffer::end()) as c_int);
            buffer::rl_mark.set((off + s.len()).min(buffer::end()) as c_int);
            crate::commands::text::rl_activate_mark();
            0
        }
        None => {
            rl_ding();
            -1
        }
    }
}

/// Read the search string in the minibuffer; None when aborted.
unsafe fn noninc_read() -> Result<Vec<u8>, bool> {
    let full = cstr(display::rl_prompt.get()).to_vec();
    let mut prompt = display::split_prompt(&full).1.to_vec();
    prompt.push(b':');
    let orig_line = buffer::line().to_vec();
    let orig_point = buffer::rl_point.get();
    let orig_mark = buffer::rl_mark.get();
    let orig_undo = rl_undo_list.replace(null_mut());
    let restore = |redisplay: bool| {
        buffer::set_line(&orig_line);
        buffer::rl_point.set(orig_point.min(buffer::rl_end.get()));
        buffer::rl_mark.set(orig_mark.min(buffer::rl_end.get()));
        rl_undo_list.set(orig_undo);
        display::clear_arg_message();
        if redisplay {
            display::redisplay();
        }
    };
    let mut s: Vec<u8> = Vec::new();
    display::set_message(&prompt);
    loop {
        buffer::set_line(&s);
        buffer::rl_point.set(buffer::rl_end.get());
        display::redisplay();
        set_state(RL_STATE_NSEARCH);
        let c = input::rl_read_key();
        unset_state(RL_STATE_NSEARCH);
        if c < 0 {
            restore(true);
            crate::readline::signal_eof();
            return Err(true);
        }
        match c as u8 {
            0x07 => {
                rl_ding();
                restore(true);
                return Err(false);
            }
            b'\r' | b'\n' => break,
            0x08 | 0x7f => {
                if s.is_empty() {
                    restore(true);
                    return Err(false);
                }
                let n = s.len();
                let p = if crate::mb::multibyte() { crate::mb::prev_char(&s, n) } else { n - 1 };
                s.truncate(p);
            }
            0x15 => s.clear(),
            0x17 => {
                let mut e = s.len();
                while e > 0 && (s[e - 1] == b' ' || s[e - 1] == b'\t') {
                    e -= 1;
                }
                while e > 0 && s[e - 1] != b' ' && s[e - 1] != b'\t' {
                    e -= 1;
                }
                s.truncate(e);
            }
            0x16 | 0x11 => {
                let k = input::rl_read_key();
                if k < 0 {
                    restore(true);
                    crate::readline::signal_eof();
                    return Err(true);
                }
                s.extend_from_slice(&read_mb_rest(k as u8));
            }
            0x1b => {
                // a bracketed paste start, else a literal ESC
                let mut got = vec![0x1bu8];
                if crate::vars::enable_bracketed_paste.get() != 0 {
                    const START: &[u8] = b"\x1b[200~";
                    let t = crate::vars::keyseq_timeout.get().max(0) as i64 * 1000;
                    while got.len() < START.len() && START.starts_with(&got) && input::input_available(t) {
                        let k = input::rl_read_key();
                        if k < 0 {
                            break;
                        }
                        got.push(k as u8);
                    }
                    if got == START {
                        let p = read_paste();
                        s.extend_from_slice(&p);
                        continue;
                    }
                }
                s.extend_from_slice(&got);
            }
            b => s.extend_from_slice(&read_mb_rest(b)),
        }
    }
    restore(false);
    Ok(s)
}

unsafe fn noninc(count: c_int, dir: isize) -> c_int {
    let dir = if count < 0 { -dir } else { dir };
    let s = match noninc_read() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let s = if s.is_empty() {
        match LAST_NONINC.get_mut().clone() {
            Some(l) => l,
            None => {
                rl_ding();
                display::redisplay();
                return -1;
            }
        }
    } else {
        LAST_NONINC.set(Some(s.clone()));
        s
    };
    let r = noninc_do(&s, dir, 1);
    display::redisplay();
    r
}

unsafe fn noninc_again(count: c_int, dir: isize) -> c_int {
    let dir = if count < 0 { -dir } else { dir };
    match LAST_NONINC.get_mut().clone() {
        Some(s) => noninc_do(&s, dir, 1),
        None => {
            rl_ding();
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_noninc_reverse_search(count: c_int, _key: c_int) -> c_int {
    noninc(count, -1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_noninc_forward_search(count: c_int, _key: c_int) -> c_int {
    noninc(count, 1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_noninc_reverse_search_again(count: c_int, _key: c_int) -> c_int {
    noninc_again(count, -1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_noninc_forward_search_again(count: c_int, _key: c_int) -> c_int {
    noninc_again(count, 1)
}

// ---------------------------------------------------------------- history-search-*

fn is_hsearch_func(f: Option<rl_command_func_t>, substring: bool) -> bool {
    if substring {
        is_func(f, rl_history_substr_search_backward) || is_func(f, rl_history_substr_search_forward)
    } else {
        is_func(f, rl_history_search_backward) || is_func(f, rl_history_search_forward)
    }
}

unsafe fn hsearch(count: c_int, dir: isize, substring: bool) -> c_int {
    if count == 0 {
        return 0;
    }
    let (count, dir) = if count < 0 { (-count, -dir) } else { (count, dir) };
    if !is_hsearch_func(crate::dispatch::rl_last_func.get(), substring) {
        let p = buffer::point().min(buffer::end());
        HSEARCH.set(buffer::line()[..p].to_vec());
    }
    let s = HSEARCH.get_mut().clone();
    if s.is_empty() {
        return if dir < 0 {
            histcmds::rl_get_previous_history(count, 0)
        } else {
            histcmds::rl_get_next_history(count, 0)
        };
    }
    let len = history_length.get();
    let mut pos = history_offset.get().min(len);
    let mut cur = buffer::line().to_vec();
    let mut found: Option<(c_int, usize)> = None;
    let mut left = count;
    let mut i = pos + dir as c_int;
    while left > 0 && i >= 0 && i < len {
        let line = history::line_at(i).unwrap_or(&[]);
        let m = if substring {
            find_in(line, &s, dir < 0)
        } else if line.len() >= s.len() && bytes_eq(&line[..s.len()], &s) {
            Some(0)
        } else {
            None
        };
        if let Some(off) = m {
            if line != &cur[..] {
                found = Some((i, off));
                cur = line.to_vec();
                pos = i;
                left -= 1;
            }
        }
        i += dir as c_int;
    }
    let _ = pos;
    match found {
        Some((i, off)) => {
            histcmds::goto_history(i);
            let p = if substring { off } else { s.len() };
            buffer::rl_point.set(p.min(buffer::end()) as c_int);
            buffer::rl_mark.set(buffer::rl_end.get());
            0
        }
        None => {
            rl_ding();
            buffer::rl_point.set(s.len().min(buffer::end()) as c_int);
            buffer::rl_mark.set(buffer::rl_end.get());
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_history_search_backward(count: c_int, _key: c_int) -> c_int {
    hsearch(count, -1, false)
}

#[no_mangle]
pub unsafe extern "C" fn rl_history_search_forward(count: c_int, _key: c_int) -> c_int {
    hsearch(count, 1, false)
}

#[no_mangle]
pub unsafe extern "C" fn rl_history_substr_search_backward(count: c_int, _key: c_int) -> c_int {
    hsearch(count, -1, true)
}

#[no_mangle]
pub unsafe extern "C" fn rl_history_substr_search_forward(count: c_int, _key: c_int) -> c_int {
    hsearch(count, 1, true)
}
