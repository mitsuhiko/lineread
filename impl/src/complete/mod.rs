//! Completion: finding the word to complete, generating matches (the
//! application's attempted-completion function or an entry function),
//! post-processing (duplicates, ignore function), inserting the result
//! (quoting, closing quotes, append character, directory slashes),
//! menu completion and the bindable completion commands.
//!
//! Submodules: `filename` (filename and username generators), `listing`
//! (displaying matches, paging, `rl_display_match_list`), `colors`
//! (LS_COLORS parsing and colored listings).
//! Owner: display & completion workstream.
use crate::buffer;
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;
use crate::util::{strvec_iter, xmalloc};

pub mod colors;
pub mod filename;
pub mod listing;

#[no_mangle]
pub static rl_attempted_completion_function: CVar<Option<rl_completion_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_attempted_completion_over: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_basic_quote_characters: CVar<*const c_char> = CVar::new(cs!("\"'"));
#[no_mangle]
pub static rl_basic_word_break_characters: CVar<*const c_char> = CVar::new(cs!(" \t\n\"\\'`@$><=;|&{("));
#[no_mangle]
pub static rl_char_is_quoted_p: CVar<Option<rl_linebuf_func_t>> = CVar::new(None);
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
pub static rl_directory_completion_hook: CVar<Option<rl_icppfunc_t>> = CVar::new(None);
#[no_mangle]
pub static rl_directory_rewrite_hook: CVar<Option<rl_icppfunc_t>> = CVar::new(None);
#[no_mangle]
pub static rl_filename_completion_desired: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_filename_dequoting_function: CVar<Option<rl_dequote_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_filename_quote_characters: CVar<*const c_char> = CVar::new(core::ptr::null_mut());
#[no_mangle]
pub static rl_filename_quoting_desired: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_filename_quoting_function: CVar<Option<rl_quote_func_t>> = CVar::new(Some(default_quote_filename));
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
pub static rl_menu_completion_entry_function: CVar<Option<rl_compentry_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_sort_completion_matches: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_special_prefixes: CVar<*const c_char> = CVar::new(core::ptr::null_mut());

// quote flags of rl_completion_found_quote
pub const QF_SINGLE: c_int = 0x01;
pub const QF_DOUBLE: c_int = 0x02;
pub const QF_BACKSLASH: c_int = 0x04;
pub const QF_OTHER: c_int = 0x08;

const ABORT_CHAR: c_int = 7;
const RUBOUT: c_int = 0x7f;

/// The line changed during the last completion (a second TAB lists only
/// when the first one did not change it).
static CHANGED_BUFFER: CVar<bool> = CVar::new(false);
/// The last completion command was aborted (^G at a question): it does
/// not count as the previous command.
static ABORTED: CVar<bool> = CVar::new(false);

/// Result of a completion attempt: `[lcd, match...]`.
pub type Matches = Vec<Vec<u8>>;

// ------------------------------------------------------------ small helpers

fn bytes_of(p: *const c_char) -> Vec<u8> {
    unsafe { cstr(p).to_vec() }
}

fn contains(set: *const c_char, c: u8) -> bool {
    c != 0 && unsafe { cstr(set).contains(&c) }
}

pub fn case_fold() -> bool {
    crate::vars::completion_ignore_case.get() != 0
}
pub fn map_case() -> bool {
    case_fold() && crate::vars::completion_map_case.get() != 0
}

/// Fold one character for case-insensitive comparison (and map `-` to `_`
/// when completion-map-case is on).
fn fold_char(c: u32, map: bool) -> u32 {
    if map && c == b'-' as u32 {
        return b'_' as u32;
    }
    unsafe { crate::util::towlower(c) }
}

/// Compare two strings character-wise and return the length (in bytes of
/// `a`) of their common prefix, ignoring case when `fold`.
pub fn common_prefix_len(a: &[u8], b: &[u8], fold: bool, map: bool) -> usize {
    if !fold {
        let mut i = 0;
        while i < a.len() && i < b.len() && a[i] == b[i] {
            i += 1;
        }
        if crate::mb::multibyte() && i < a.len() {
            // back up to the start of a partially shared character
            return crate::mb::align(a, i);
        }
        return i;
    }
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (ca, na) = crate::mb::decode_at(a, i);
        let (cb, nb) = crate::mb::decode_at(b, j);
        let eq = match (ca, cb) {
            (Some(x), Some(y)) if na > 1 || nb > 1 || crate::mb::multibyte() || (x < 0x80 && y < 0x80) => {
                fold_char(x, map) == fold_char(y, map)
            }
            (Some(x), Some(y)) => {
                // byte locale: tolower on bytes
                let fx = if map && x == b'-' as u32 { b'_' as i32 } else { unsafe { libc::tolower(x as i32) } };
                let fy = if map && y == b'-' as u32 { b'_' as i32 } else { unsafe { libc::tolower(y as i32) } };
                fx == fy
            }
            _ => a[i] == b[j] && na == nb,
        };
        if !eq {
            break;
        }
        i += na.max(1);
        j += nb.max(1);
    }
    i
}

/// Does `name` start with `prefix` (honouring completion-ignore-case /
/// completion-map-case)?
pub fn prefix_matches(prefix: &[u8], name: &[u8]) -> bool {
    if !case_fold() {
        return name.starts_with(prefix);
    }
    // all of prefix must be consumed
    let (mut i, mut j) = (0, 0);
    while i < prefix.len() {
        if j >= name.len() {
            return false;
        }
        let n = common_prefix_len(&name[j..], &prefix[i..], true, map_case());
        if n == 0 {
            return false;
        }
        // advance one character in each
        let (_, na) = crate::mb::decode_at(name, j);
        let (_, np) = crate::mb::decode_at(prefix, i);
        j += na.max(1);
        i += np.max(1);
    }
    true
}

/// Call a hook taking `char **` (directory/stat hooks) on a malloc'd copy
/// of `s`; returns the hook's result and the (possibly replaced) string.
pub unsafe fn call_icpp(hook: rl_icppfunc_t, s: &[u8]) -> (c_int, Vec<u8>) {
    let mut p = c_strdup(s);
    let r = hook(&mut p);
    let v = bytes_of(p);
    cfree(p);
    (r, v)
}

/// Call a dequote-style hook `(char *, int) -> char *` (freeing the result).
pub unsafe fn call_dequote(hook: rl_dequote_func_t, s: &[u8], arg: c_int) -> Option<Vec<u8>> {
    let p = c_strdup(s);
    let r = hook(p, arg);
    let v = if r.is_null() { None } else { Some(bytes_of(r)) };
    if r != p {
        cfree(r);
    }
    cfree(p);
    v
}

/// The default filename quoting function: put the first completer quote
/// character in front of the name.
unsafe extern "C" fn default_quote_filename(s: *mut c_char, _rtype: c_int, qcp: *mut c_char) -> *mut c_char {
    let q = cstr(rl_completer_quote_characters.get()).first().copied();
    let mut v = Vec::new();
    if let Some(q) = q {
        v.push(q);
        if !qcp.is_null() {
            *qcp = q as c_char;
        }
    }
    v.extend_from_slice(cstr(s));
    c_strdup(&v)
}

/// A C vector `[m0, m1, ..., NULL]` of malloc'd copies.
pub unsafe fn to_c_vec(m: &Matches) -> *mut *mut c_char {
    crate::util::c_strvec(m)
}

/// Copy and free a C match vector.
pub unsafe fn from_c_vec(v: *mut *mut c_char) -> Option<Matches> {
    if v.is_null() {
        return None;
    }
    let mut out = Vec::new();
    for p in strvec_iter(v) {
        out.push(bytes_of(p));
        cfree(p);
    }
    cfree(v);
    Some(out)
}

/// Free a C match vector.
pub unsafe fn free_c_vec(v: *mut *mut c_char) {
    if v.is_null() {
        return;
    }
    for p in strvec_iter(v) {
        cfree(p);
    }
    cfree(v);
}

// ------------------------------------------------------------ matches

/// Compute matches[0] (the longest common prefix) for `[_, m1, m2...]`.
fn compute_lcd(list: &mut Matches, text: &[u8]) {
    let n = list.len() - 1;
    if n == 1 {
        list[0] = list[1].clone();
        return;
    }
    let fold = case_fold();
    let mut low = usize::MAX;
    for i in 1..n {
        let l = common_prefix_len(&list[i], &list[i + 1], fold, false);
        low = low.min(l);
    }
    if low == 0 && !text.is_empty() {
        list[0] = text.to_vec();
        return;
    }
    if fold {
        // consistent answers: sort, then prefer a match with the case the
        // user typed (dequoted)
        let mut dtext = text.to_vec();
        unsafe {
            if rl_filename_completion_desired.get() != 0
                && rl_completion_found_quote.get() != 0
                && rl_filename_quoting_desired.get() != 0
            {
                if let Some(f) = rl_filename_dequoting_function.get() {
                    if let Some(d) = call_dequote(f, text, rl_completion_quote_character.get()) {
                        dtext = d;
                    }
                }
            }
        }
        let text = &dtext[..];
        list[1..].sort();
        let first = (1..=n).find(|&i| text.len() <= low && list[i].starts_with(text)).unwrap_or(1);
        list[0] = list[first][..low.min(list[first].len())].to_vec();
    } else {
        list[0] = list[1][..low.min(list[1].len())].to_vec();
    }
}

/// Run a generator and build `[lcd, matches...]`.
pub unsafe fn completion_matches(text: &[u8], gen: rl_compentry_func_t) -> Option<Matches> {
    let ctext = c_strdup(text);
    let mut list: Matches = vec![Vec::new()];
    let mut state = 0;
    loop {
        let r = gen(ctext, state);
        if r.is_null() {
            break;
        }
        list.push(bytes_of(r));
        cfree(r);
        state += 1;
    }
    cfree(ctext);
    if list.len() == 1 {
        return None;
    }
    compute_lcd(&mut list, text);
    if list.len() == 2 {
        // a single match is returned alone
        list.truncate(1);
    }
    Some(list)
}

#[no_mangle]
pub unsafe extern "C" fn rl_completion_matches(text: *const c_char, gen: Option<rl_compentry_func_t>) -> *mut *mut c_char {
    let Some(gen) = gen else { return null_mut() };
    match completion_matches(cstr(text), gen) {
        Some(m) => to_c_vec(&m),
        None => null_mut(),
    }
}

// ------------------------------------------------------------ word finding

pub struct Word {
    pub start: usize,
    pub found_quote: c_int,
    pub quote_char: u8,
    pub delimiter: u8,
}

unsafe fn break_chars() -> Vec<u8> {
    if let Some(h) = rl_completion_word_break_hook.get() {
        let p = h();
        if !p.is_null() {
            return bytes_of(p);
        }
    }
    let p = rl_completer_word_break_characters.get();
    if !p.is_null() {
        return bytes_of(p);
    }
    bytes_of(rl_basic_word_break_characters.get())
}

unsafe fn is_quoted(line_ptr: *mut c_char, i: usize) -> bool {
    match rl_char_is_quoted_p.get() {
        Some(f) => f(line_ptr, i as c_int) != 0,
        None => false,
    }
}

/// Find the start of the word before point (`end`).
pub unsafe fn find_completion_word(end: usize) -> Word {
    let line = buffer::line().to_vec();
    let lp = buffer::rl_line_buffer.get();
    let qchars = bytes_of(rl_completer_quote_characters.get());
    let mut found = 0;
    let mut quote_char = 0u8;
    let mut start = end;
    if !qchars.is_empty() {
        let mut pass_next = false;
        let mut i = 0;
        while i < end {
            let c = line[i];
            if pass_next {
                pass_next = false;
                i += 1;
                continue;
            }
            if c == b'\\' && quote_char != b'\'' {
                pass_next = true;
                found |= QF_BACKSLASH;
                i += 1;
                continue;
            }
            if quote_char != 0 {
                if c == quote_char {
                    quote_char = 0;
                    start = end;
                }
            } else if qchars.contains(&c) {
                quote_char = c;
                start = i + 1;
                found |= match c {
                    b'\'' => QF_SINGLE,
                    b'"' => QF_DOUBLE,
                    _ => QF_OTHER,
                };
            }
            i += 1;
        }
    }
    let brk = break_chars();
    if start == end && quote_char == 0 {
        let mut p = end;
        loop {
            p = crate::mb::prev_char(&line, p);
            if p == 0 {
                break;
            }
            let c = line[p];
            if !brk.contains(&c) {
                continue;
            }
            if rl_char_is_quoted_p.get().is_some() && found != 0 && is_quoted(lp, p) {
                continue;
            }
            break;
        }
        start = p;
    }
    let mut delimiter = 0;
    if start < line.len() {
        let c = line[start];
        let isbrk = brk.contains(&c)
            && (rl_char_is_quoted_p.get().is_none() || found == 0 || !is_quoted(lp, start));
        if isbrk {
            if contains(rl_basic_quote_characters.get(), c) && end - start > 1 {
                delimiter = c;
            }
            if !contains(rl_special_prefixes.get(), c) {
                start += 1;
            }
        }
    }
    Word { start: start.min(end), found_quote: found, quote_char, delimiter }
}

// ------------------------------------------------------------ generation

unsafe fn set_completion_defaults(what: c_int) {
    rl_filename_completion_desired.set(0);
    rl_filename_quoting_desired.set(1);
    rl_completion_type.set(what);
    rl_completion_suppress_append.set(0);
    rl_completion_suppress_quote.set(0);
    rl_completion_append_character.set(b' ' as c_int);
    rl_completion_mark_symlink_dirs.set(crate::vars::mark_symlinked_directories.get());
    rl_full_quoting_desired.set(0);
    rl_sort_completion_matches.set(1);
    rl_ignore_completion_duplicates.set(1);
    rl_attempted_completion_over.set(0);
}

fn reset_completion_state() {
    rl_completion_found_quote.set(0);
    rl_completion_quote_character.set(0);
}

unsafe fn gen_matches(text: &[u8], start: usize, end: usize, func: rl_compentry_func_t, w: &Word) -> Option<Matches> {
    rl_completion_found_quote.set(w.found_quote);
    rl_completion_quote_character.set(w.quote_char as c_int);
    if let Some(f) = rl_attempted_completion_function.get() {
        let ct = c_strdup(text);
        let r = f(ct, start as c_int, end as c_int);
        cfree(ct);
        if !r.is_null() || rl_attempted_completion_over.get() != 0 {
            rl_attempted_completion_over.set(0);
            return from_c_vec(r);
        }
    }
    completion_matches(text, func)
}

/// Remove duplicates (sorting first when wanted), run the ignore function
/// for filenames.  Returns None when nothing is left.
unsafe fn postprocess(mut m: Matches, text: &[u8]) -> Option<Matches> {
    if rl_ignore_completion_duplicates.get() != 0 && m.len() > 2 {
        let mut rest: Matches = m[1..].to_vec();
        if rl_sort_completion_matches.get() != 0 {
            rest.sort();
        }
        rest.dedup();
        let mut nm = vec![m[0].clone()];
        nm.extend(rest);
        if nm.len() == 2 {
            nm[0] = nm[1].clone();
            nm.truncate(1);
        } else if nm.len() != m.len() {
            compute_lcd(&mut nm, text);
        }
        m = nm;
    }
    if rl_filename_completion_desired.get() != 0 {
        if let Some(f) = rl_ignore_some_completions_function.get() {
            let before = m.len();
            let v = to_c_vec(&m);
            f(v);
            let mut r = from_c_vec(v)?;
            if r.is_empty() {
                return None;
            }
            // some removed: recompute the common prefix
            if r.len() > 1 && r.len() < before {
                let t = r[0].clone();
                compute_lcd(&mut r, &t);
                if r.len() == 2 {
                    r.truncate(1);
                }
            }
            m = r;
        }
    }
    Some(m)
}

/// Does the common prefix add something to the typed text?  (The text is
/// dequoted even when there are no matches.)
unsafe fn nontrivial_lcd(text: &[u8], m: Option<&Matches>) -> bool {
    let mut t = text.to_vec();
    if rl_completion_found_quote.get() != 0 && !text.is_empty() {
        if let Some(f) = rl_filename_dequoting_function.get() {
            if let Some(d) = call_dequote(f, text, rl_completion_quote_character.get()) {
                t = d;
            }
        }
    }
    match m {
        Some(m) if !m.is_empty() => t != m[0],
        _ => false,
    }
}

/// Is `m` different from the typed text (dequoted when necessary)?
unsafe fn compare_match(text: &[u8], m: &[u8]) -> bool {
    if rl_completion_found_quote.get() != 0 && !text.is_empty() {
        if let Some(f) = rl_filename_dequoting_function.get() {
            if let Some(t) = call_dequote(f, text, rl_completion_quote_character.get()) {
                return t != m;
            }
        }
    }
    text != m
}

// ------------------------------------------------------------ insertion

/// Quote a match for insertion when filename quoting applies.  May change
/// `qc` (the quote character in effect).
unsafe fn make_quoted_replacement(m: &[u8], mtype: c_int, qc: &mut u8) -> Vec<u8> {
    let mut should_quote = rl_filename_completion_desired.get() != 0 && rl_filename_quoting_desired.get() != 0;
    if should_quote {
        should_quote = *qc == 0 || contains(rl_completer_quote_characters.get(), *qc);
    }
    if should_quote {
        // quote when the name has a filename quote character, or when the
        // word had quoting but no quote is open (the first insertion then
        // opens one)
        let fq = rl_filename_quote_characters.get();
        should_quote = (rl_completion_found_quote.get() != 0 && *qc == 0)
            || (!fq.is_null() && m.iter().any(|&c| contains(fq, c)));
    }
    if should_quote {
        if let Some(f) = rl_filename_quoting_function.get() {
            let cm = c_strdup(m);
            let mut q: c_char = *qc as c_char;
            let r = f(cm, mtype, &mut q);
            *qc = q as u8;
            let v = if r.is_null() { m.to_vec() } else { bytes_of(r) };
            if r != cm {
                cfree(r);
            }
            cfree(cm);
            return v;
        }
    }
    m.to_vec()
}

/// Replace `[start, end)` with `text` as one undo group, point after it.
pub unsafe fn replace_text(text: &[u8], start: usize, end: usize) {
    crate::undo::rl_begin_undo_group();
    if start < end {
        buffer::delete(start, end);
    }
    buffer::set_point(start);
    if !text.is_empty() {
        buffer::insert(text);
    }
    crate::undo::rl_end_undo_group();
}

unsafe fn insert_match(m: &[u8], start: usize, mtype: c_int, qc: &mut u8) {
    let oqc = *qc;
    let rep = make_quoted_replacement(m, mtype, qc);
    let line = buffer::line().to_vec();
    let point = buffer::point();
    let mut start = start;
    if *qc != 0 && start > 0 && line[start - 1] == *qc && rep.first() == Some(qc) {
        // do not double an opening quote
        start -= 1;
    } else if *qc != oqc && start > 0 && line[start - 1] == oqc && rep.first() != Some(&oqc) {
        // the quoting function changed the quote character
        start -= 1;
    }
    let mut end = point;
    if *qc != 0 && end > 0 && line.get(point) == Some(qc) && rep.last() == Some(qc) {
        // do not double a closing quote
        end += 1;
    }
    if crate::vars::skip_completed_text.get() != 0 {
        let mut s = start;
        let mut r = 0;
        let l = buffer::line();
        while s < l.len() && r < rep.len() && l[s] == rep[r] {
            s += 1;
            r += 1;
        }
        // (nothing changes, and no undo entry is made, when the rest of
        // the match is already there)
        if s < end || r < rep.len() {
            replace_text(&rep[r..], s, end.max(s));
        }
        buffer::set_point(s + rep.len() - r);
    } else {
        replace_text(&rep, start, end);
    }
}

/// Is `path` a directory (following symlinks)?
pub fn path_isdir(path: &[u8]) -> bool {
    unsafe {
        let c = c_strdup(path);
        let mut st: libc::stat = core::mem::zeroed();
        let r = libc::stat(c, &mut st);
        cfree(c);
        r == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
    }
}

/// Add the closing quote / append character / directory slash after a
/// completed match.
unsafe fn append_to_match(text: &[u8], delimiter: u8, quote_char: u8, nontrivial: bool) {
    let mut app = Vec::new();
    let point = buffer::point();
    let line = buffer::line();
    if quote_char != 0 && point > 0 && rl_completion_suppress_quote.get() == 0 && line[point - 1] != quote_char {
        app.push(quote_char);
    }
    if delimiter != 0 {
        app.push(delimiter);
    } else if rl_completion_suppress_append.get() == 0 && rl_completion_append_character.get() != 0 {
        app.push(rl_completion_append_character.get() as u8);
    }
    if rl_filename_completion_desired.get() != 0 {
        let mut fname = filename::tilde_expand_bytes(text);
        if let Some(h) = rl_filename_stat_hook.get() {
            fname = call_icpp(h, &fname).1;
        }
        let c = c_strdup(&fname);
        let mut st: libc::stat = core::mem::zeroed();
        let s = if nontrivial && rl_completion_mark_symlink_dirs.get() == 0 {
            libc::lstat(c, &mut st)
        } else {
            libc::stat(c, &mut st)
        };
        cfree(c);
        let fmt = st.st_mode & libc::S_IFMT;
        if s == 0 && fmt == libc::S_IFDIR {
            if crate::vars::mark_directories.get() != 0 {
                let line = buffer::line();
                let point = buffer::point();
                let at = line.get(point).copied();
                if point > 0 && at.is_none() && line[point - 1] == b'/' {
                    // no double slash
                } else if at != Some(b'/') {
                    buffer::insert(b"/");
                }
            }
        } else if s == 0 && fmt == libc::S_IFLNK && path_isdir(&fname) {
            // a symlink to a directory: nothing
        } else if buffer::point() == buffer::end() && !app.is_empty() {
            buffer::insert(&app);
        }
    } else if buffer::point() == buffer::end() && !app.is_empty() {
        buffer::insert(&app);
    }
}

/// Insert all matches separated by spaces, replacing the word.
unsafe fn insert_all_matches(m: &Matches, start: usize, qc: &mut u8) {
    crate::undo::rl_begin_undo_group();
    let point = buffer::point();
    let line = buffer::line().to_vec();
    let mut start = start;
    if *qc != 0 && start > 0 && line[start - 1] == *qc {
        start -= 1;
    }
    buffer::delete(start, point);
    buffer::set_point(start);
    if m.len() > 1 {
        for s in &m[1..] {
            let r = make_quoted_replacement(s, SINGLE_MATCH, qc);
            buffer::insert(&r);
            buffer::insert(b" ");
        }
    } else {
        let r = make_quoted_replacement(&m[0], SINGLE_MATCH, qc);
        buffer::insert(&r);
        buffer::insert(b" ");
    }
    crate::undo::rl_end_undo_group();
}

// ------------------------------------------------------------ driver

fn default_entry() -> rl_compentry_func_t {
    rl_completion_entry_function.get().unwrap_or(filename::rl_filename_completion_function)
}

/// Run the whole completion for `what` (TAB, '?', '!', '@', '*', '%', '$').
pub unsafe fn complete_internal(what: c_int) -> c_int {
    ABORTED.set(false);
    set_state(RL_STATE_COMPLETING);
    set_completion_defaults(what);
    let saved = buffer::line().to_vec();
    let func = default_entry();
    let end = buffer::point();
    let w = if end > 0 { find_completion_word(end) } else { Word { start: end, found_quote: 0, quote_char: 0, delimiter: 0 } };
    buffer::set_point(end);
    let start = w.start;
    let text = buffer::line()[start..end].to_vec();
    let matches = gen_matches(&text, start, end, func, &w);
    let nontrivial = nontrivial_lcd(&text, matches.as_ref());
    let matches = match matches {
        Some(m) if !m.is_empty() => postprocess(m, &text),
        _ => None,
    };
    let Some(matches) = matches else {
        crate::terminal::rl_ding();
        CHANGED_BUFFER.set(false);
        unset_state(RL_STATE_COMPLETING);
        reset_completion_state();
        if what == b'$' as c_int || what == b'|' as c_int {
            if what == b'$' as c_int {
                export_matches(&[], &text, start, end);
            }
        }
        return 0;
    };
    let mut qc = w.quote_char;
    let r = match what as u8 {
        b'\t' | b'!' | b'@' => {
            if !matches[0].is_empty() {
                insert_match(&matches[0], start, if matches.len() > 1 { MULT_MATCH } else { SINGLE_MATCH }, &mut qc);
            }
            if matches.len() > 1 {
                if what == b'!' as c_int {
                    listing::display_matches(&matches);
                } else if what == b'@' as c_int {
                    if !nontrivial {
                        listing::display_matches(&matches);
                    }
                } else if crate::readline::rl_editing_mode.get() != crate::readline::VI_MODE {
                    crate::terminal::rl_ding();
                }
            } else {
                append_to_match(&matches[0], w.delimiter, qc, nontrivial);
            }
            0
        }
        b'*' => {
            insert_all_matches(&matches, start, &mut qc);
            0
        }
        b'?' => {
            listing::display_matches(&matches);
            0
        }
        b'$' => {
            export_matches(&matches, &text, start, end);
            0
        }
        b'|' => 0,
        _ => {
            let msg = format!("readline: bad value {} for what_to_do in rl_complete\n", what);
            crate::terminal::flush();
            let e = crate::util::lr_stderr();
            crate::util::fwrite_bytes(e, msg.as_bytes());
            crate::util::fflush(e);
            crate::terminal::rl_ding();
            crate::display::forget_screen();
            1
        }
    };
    CHANGED_BUFFER.set(buffer::line() != &saved[..]);
    unset_state(RL_STATE_COMPLETING);
    reset_completion_state();
    r
}

/// Called by the listing code when the user aborted a question (^G/EOF).
pub fn note_aborted() {
    ABORTED.set(true);
    crate::commands::misc::abort_internal();
}

pub fn aborted() -> bool {
    ABORTED.get()
}

unsafe fn last_was(f: rl_command_func_t) -> bool {
    !ABORTED.get() && crate::dispatch::same_fn(crate::dispatch::rl_last_func.get(), f)
}

#[no_mangle]
pub unsafe extern "C" fn rl_complete_internal(what: c_int) -> c_int {
    complete_internal(what)
}

#[no_mangle]
pub unsafe extern "C" fn rl_completion_mode(cfunc: Option<rl_command_func_t>) -> c_int {
    let same = match cfunc {
        Some(f) => last_was(f),
        None => false,
    };
    if same && !CHANGED_BUFFER.get() {
        b'?' as c_int
    } else if crate::vars::show_all_if_ambiguous.get() != 0 {
        b'!' as c_int
    } else if crate::vars::show_all_if_unmodified.get() != 0 {
        b'@' as c_int
    } else {
        b'\t' as c_int
    }
}

fn completion_disabled() -> bool {
    rl_inhibit_completion.get() != 0 || crate::vars::disable_completion.get() != 0
}

#[no_mangle]
pub unsafe extern "C" fn rl_complete(ignore: c_int, key: c_int) -> c_int {
    rl_completion_invoking_key.set(key);
    if completion_disabled() {
        return crate::commands::text::rl_insert(ignore, key);
    }
    complete_internal(rl_completion_mode(Some(rl_complete)))
}

#[no_mangle]
pub unsafe extern "C" fn rl_possible_completions(_ignore: c_int, key: c_int) -> c_int {
    rl_completion_invoking_key.set(key);
    complete_internal(b'?' as c_int)
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert_completions(_ignore: c_int, key: c_int) -> c_int {
    rl_completion_invoking_key.set(key);
    complete_internal(b'*' as c_int)
}

#[no_mangle]
pub unsafe extern "C" fn rl_delete_or_show_completions(count: c_int, key: c_int) -> c_int {
    if buffer::end() != 0 && buffer::point() == buffer::end() {
        rl_possible_completions(count, key)
    } else {
        crate::commands::text::rl_delete(count, key)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_export_completions(_count: c_int, key: c_int) -> c_int {
    rl_completion_invoking_key.set(key);
    let r = complete_internal(b'$' as c_int);
    if crate::numarg::rl_explicit_arg.get() != 0 {
        // with an argument the line is cleared afterwards
        buffer::delete(0, buffer::end());
        buffer::set_point(0);
        buffer::set_mark(0);
    }
    r
}

/// Write the matches for export-completions: count, text, start:end, then
/// the matches, one per line.
unsafe fn export_matches(m: &[Vec<u8>], text: &[u8], start: usize, end: usize) {
    if crate::readline::in_readline() {
        crate::display::move_to_end_for_output();
        crate::terminal::rl_crlf();
    }
    let mut out = Vec::new();
    out.extend_from_slice(m.len().to_string().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(text);
    out.push(b'\n');
    out.extend_from_slice(format!("{}:{}\n", start, end).as_bytes());
    for s in m {
        out.extend_from_slice(s);
        out.push(b'\n');
    }
    listing::write_listing(&out);
}

// ------------------------------------------------------------ menu completion

struct Menu {
    active: bool,
    orig_text: Vec<u8>,
    orig_start: usize,
    matches: Option<Matches>,
    index: isize,
    size: isize,
    nontrivial: bool,
    full: bool,
    delimiter: u8,
    quote_char: u8,
}

static MENU: CVar<Menu> = CVar::new(Menu {
    active: false,
    orig_text: Vec::new(),
    orig_start: 0,
    matches: None,
    index: 0,
    size: 0,
    nontrivial: false,
    full: false,
    delimiter: 0,
    quote_char: 0,
});

static OLD_MENU: CVar<Menu> = CVar::new(Menu {
    active: false,
    orig_text: Vec::new(),
    orig_start: 0,
    matches: None,
    index: 0,
    size: 0,
    nontrivial: false,
    full: false,
    delimiter: 0,
    quote_char: 0,
});

/// Generate matches for a fresh menu.  Returns false when there are none.
unsafe fn menu_start(menu: &CVar<Menu>, what: c_int, func: rl_compentry_func_t) -> bool {
    set_completion_defaults(what);
    let end = buffer::point();
    let w = if end > 0 { find_completion_word(end) } else { Word { start: end, found_quote: 0, quote_char: 0, delimiter: 0 } };
    buffer::set_point(end);
    let text = buffer::line()[w.start..end].to_vec();
    let matches = gen_matches(&text, w.start, end, func, &w);
    let nontrivial = nontrivial_lcd(&text, matches.as_ref());
    let matches = match matches {
        Some(m) if !m.is_empty() => postprocess(m, &text),
        _ => None,
    };
    let m = menu.get_mut();
    m.orig_text = text;
    m.orig_start = w.start;
    m.index = 0;
    m.nontrivial = nontrivial;
    m.full = false;
    m.delimiter = w.delimiter;
    m.quote_char = w.quote_char;
    m.size = matches.as_ref().map(|v| v.len() as isize).unwrap_or(0);
    m.matches = matches;
    m.active = m.matches.is_some();
    m.active
}

#[no_mangle]
pub unsafe extern "C" fn rl_menu_complete(count: c_int, _key: c_int) -> c_int {
    let cont = (last_was(rl_menu_complete) || last_was(rl_backward_menu_complete))
        && MENU.get_mut().active
        && !MENU.get_mut().full;
    if !cont {
        set_state(RL_STATE_COMPLETING);
        let func = rl_menu_completion_entry_function
            .get()
            .or(rl_completion_entry_function.get())
            .unwrap_or(filename::rl_filename_completion_function);
        ABORTED.set(false);
        let ok = menu_start(&MENU, b'%' as c_int, func);
        if !ok {
            crate::terminal::rl_ding();
            MENU.get_mut().active = false;
            CHANGED_BUFFER.set(false);
            unset_state(RL_STATE_COMPLETING);
            reset_completion_state();
            return 0;
        }
        let m = MENU.get_mut();
        let matches = m.matches.clone().unwrap();
        let mut qc = m.quote_char;
        if matches.len() == 1 {
            let start = m.orig_start;
            let delim = m.delimiter;
            let nontrivial = m.nontrivial;
            insert_match(&matches[0], start, SINGLE_MATCH, &mut qc);
            append_to_match(&matches[0], delim, qc, nontrivial);
            let m = MENU.get_mut();
            m.full = true;
            m.quote_char = qc;
            CHANGED_BUFFER.set(true);
            unset_state(RL_STATE_COMPLETING);
            reset_completion_state();
            return 0;
        }
        if crate::vars::show_all_if_ambiguous.get() != 0 {
            listing::display_matches(&matches);
            if ABORTED.get() {
                MENU.get_mut().active = false;
                unset_state(RL_STATE_COMPLETING);
                reset_completion_state();
                return 0;
            }
            let qi = rl_completion_query_items.get();
            if qi > 0 && matches.len() as c_int >= qi {
                crate::terminal::rl_ding();
                let m = MENU.get_mut();
                m.matches = None;
                m.active = false;
                m.full = true;
                unset_state(RL_STATE_COMPLETING);
                reset_completion_state();
                return 0;
            } else if crate::vars::menu_complete_display_prefix.get() != 0 {
                crate::terminal::rl_ding();
                unset_state(RL_STATE_COMPLETING);
                reset_completion_state();
                return 0;
            }
        } else if crate::vars::menu_complete_display_prefix.get() != 0 {
            let start = MENU.get_mut().orig_start;
            insert_match(&matches[0], start, MULT_MATCH, &mut qc);
            MENU.get_mut().quote_char = qc;
            crate::terminal::rl_ding();
            CHANGED_BUFFER.set(true);
            unset_state(RL_STATE_COMPLETING);
            reset_completion_state();
            return 0;
        }
        unset_state(RL_STATE_COMPLETING);
    }
    menu_cycle(&MENU, count);
    0
}

/// Move `count` entries through the menu and insert the entry.
unsafe fn menu_cycle(menu: &CVar<Menu>, count: c_int) {
    let m = menu.get_mut();
    if !m.active || m.size == 0 {
        crate::terminal::rl_ding();
        m.active = false;
        CHANGED_BUFFER.set(false);
        return;
    }
    let matches = m.matches.clone().unwrap();
    m.index += count as isize;
    m.index = m.index.rem_euclid(m.size);
    let idx = m.index as usize;
    let start = m.orig_start;
    let orig = m.orig_text.clone();
    let delim = m.delimiter;
    let mut qc = m.quote_char;
    if idx == 0 && m.size > 1 {
        crate::terminal::rl_ding();
        insert_match(&orig, start, MULT_MATCH, &mut qc);
    } else {
        insert_match(&matches[idx], start, SINGLE_MATCH, &mut qc);
        let nt = compare_match(&orig, &matches[idx]);
        append_to_match(&matches[idx], delim, qc, nt);
    }
    CHANGED_BUFFER.set(true);
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_menu_complete(count: c_int, key: c_int) -> c_int {
    rl_menu_complete(-count, key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_old_menu_complete(count: c_int, _key: c_int) -> c_int {
    let cont = last_was(rl_old_menu_complete) && OLD_MENU.get_mut().active;
    if !cont {
        set_state(RL_STATE_COMPLETING);
        ABORTED.set(false);
        let func = default_entry();
        let ok = menu_start(&OLD_MENU, b'%' as c_int, func);
        if !ok {
            crate::terminal::rl_ding();
            OLD_MENU.get_mut().active = false;
            CHANGED_BUFFER.set(false);
            unset_state(RL_STATE_COMPLETING);
            reset_completion_state();
            return 0;
        }
        let matches = OLD_MENU.get_mut().matches.clone().unwrap();
        if crate::vars::show_all_if_ambiguous.get() != 0 && matches.len() > 1 {
            listing::display_matches(&matches);
        }
        unset_state(RL_STATE_COMPLETING);
    }
    let m = OLD_MENU.get_mut();
    if !m.active || m.size == 0 {
        crate::terminal::rl_ding();
        m.active = false;
        return 0;
    }
    let matches = m.matches.clone().unwrap();
    // in the old menu completion matches[0] is skipped when there are
    // several matches; the original text takes its place
    m.index += count as isize;
    m.index = m.index.rem_euclid(m.size);
    let idx = m.index as usize;
    let start = m.orig_start;
    let orig = m.orig_text.clone();
    let delim = m.delimiter;
    let mut qc = m.quote_char;
    if idx == 0 && m.size > 1 {
        crate::terminal::rl_ding();
        insert_match(&orig, start, MULT_MATCH, &mut qc);
    } else {
        insert_match(&matches[idx], start, SINGLE_MATCH, &mut qc);
        let nt = compare_match(&orig, &matches[idx]);
        append_to_match(&matches[idx], delim, qc, nt);
    }
    CHANGED_BUFFER.set(true);
    0
}

/// Free menu state when a line ends.
pub fn line_finished() {
    MENU.get_mut().active = false;
    OLD_MENU.get_mut().active = false;
}

// ------------------------------------------------------------ misc

#[no_mangle]
pub unsafe extern "C" fn rl_reparse_colors() {
    colors::reparse();
}

pub fn is_completing() -> bool {
    is_state(RL_STATE_COMPLETING)
}

#[allow(dead_code)]
unsafe fn alloc_vec(n: usize) -> *mut *mut c_char {
    xmalloc((n + 1) * core::mem::size_of::<*mut c_char>()) as *mut *mut c_char
}
