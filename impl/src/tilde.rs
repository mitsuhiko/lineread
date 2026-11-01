//! The tilde library (tilde.h): `~` and `~user` expansion with optional
//! hooks and additional word prefixes/suffixes, plus the tilde-expand
//! bindable command's word logic.
//! Owner: history workstream.
use crate::prelude::*;
use crate::util::strvec_iter;

#[no_mangle]
pub static tilde_additional_prefixes: CVar<*mut *mut c_char> = CVar::new(DEFAULT_PREFIXES.as_ptr() as *mut *mut c_char);

#[no_mangle]
pub static tilde_additional_suffixes: CVar<*mut *mut c_char> = CVar::new(DEFAULT_SUFFIXES.as_ptr() as *mut *mut c_char);

/// Default word prefixes/suffixes (C may read but never writes them).
static DEFAULT_PREFIXES: CVar<[*const c_char; 3]> = CVar::new([cs!(" ~"), cs!("\t~"), null()]);
static DEFAULT_SUFFIXES: CVar<[*const c_char; 3]> = CVar::new([cs!(" "), cs!("\n"), null()]);

#[no_mangle]
pub static tilde_expansion_failure_hook: CVar<Option<tilde_hook_func_t>> = CVar::new(None);

#[no_mangle]
pub static tilde_expansion_preexpansion_hook: CVar<Option<tilde_hook_func_t>> = CVar::new(None);

fn is_white(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n'
}

unsafe fn list(v: *mut *mut c_char) -> Vec<Vec<u8>> {
    strvec_iter(v).map(|p| cstr(p).to_vec()).collect()
}

/// Start of the next tilde word in `s`: 0 when `s` begins with `~`,
/// else the index of the last character of the first matching prefix.
fn find_start(s: &[u8], prefixes: &[Vec<u8>]) -> Option<usize> {
    if s.first() == Some(&b'~') {
        return Some(0);
    }
    for i in 0..s.len() {
        for p in prefixes {
            if !p.is_empty() && s[i..].starts_with(p) {
                return Some(i + p.len() - 1);
            }
        }
    }
    None
}

/// End of the tilde word at the start of `s`: the first `/` or suffix.
fn find_end(s: &[u8], suffixes: &[Vec<u8>]) -> usize {
    for i in 0..s.len() {
        if s[i] == b'/' || suffixes.iter().any(|x| !x.is_empty() && s[i..].starts_with(x)) {
            return i;
        }
    }
    s.len()
}

/// Call a tilde hook with `name`; returns the (freed) result.
unsafe fn call_hook(h: Option<tilde_hook_func_t>, name: &[u8]) -> Option<Vec<u8>> {
    let f = h?;
    let c = c_strdup(name);
    let r = f(c);
    cfree(c);
    if r.is_null() {
        return None;
    }
    let v = cstr(r).to_vec();
    cfree(r);
    Some(v)
}

unsafe fn home_dir() -> Option<Vec<u8>> {
    let h = libc::getenv(c"HOME".as_ptr());
    if !h.is_null() {
        return Some(cstr(h).to_vec());
    }
    let pw = libc::getpwuid(libc::getuid());
    if pw.is_null() {
        return None;
    }
    Some(cstr((*pw).pw_dir).to_vec())
}

/// Expand one word beginning with `~` (no other processing).
pub unsafe fn expand_word(w: &[u8]) -> Vec<u8> {
    if w.first() != Some(&b'~') {
        return w.to_vec();
    }
    let slash = w.iter().position(|&c| c == b'/').unwrap_or(w.len());
    let name = &w[1..slash];
    let rest = &w[slash..];
    let dir = if name.is_empty() {
        home_dir()
    } else if let Some(d) = call_hook(tilde_expansion_preexpansion_hook.get(), name) {
        Some(d)
    } else {
        let cn = c_strdup(name);
        let pw = libc::getpwnam(cn);
        cfree(cn);
        if !pw.is_null() {
            Some(cstr((*pw).pw_dir).to_vec())
        } else {
            call_hook(tilde_expansion_failure_hook.get(), name)
        }
    };
    match dir {
        Some(mut d) => {
            d.extend_from_slice(rest);
            d
        }
        None => w.to_vec(),
    }
}

/// Expand every tilde word of `s`.
pub unsafe fn expand(s: &[u8]) -> Vec<u8> {
    let prefixes = list(tilde_additional_prefixes.get());
    let suffixes = list(tilde_additional_suffixes.get());
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let Some(st) = find_start(rest, &prefixes) else {
            out.extend_from_slice(rest);
            break;
        };
        out.extend_from_slice(&rest[..st]);
        rest = &rest[st..];
        let end = find_end(rest, &suffixes).max(1);
        out.extend_from_slice(&expand_word(&rest[..end]));
        rest = &rest[end..];
    }
    out
}

#[no_mangle]
pub unsafe extern "C" fn tilde_expand(s: *const c_char) -> *mut c_char {
    c_strdup(&expand(cstr(s)))
}

#[no_mangle]
pub unsafe extern "C" fn tilde_expand_word(w: *const c_char) -> *mut c_char {
    if w.is_null() {
        return null_mut();
    }
    c_strdup(&expand_word(cstr(w)))
}

/// The tilde-expand command: expand the whitespace-delimited word around
/// point when it starts with `~`; point moves to the end of the word.
pub unsafe fn tilde_expand_command() -> c_int {
    use crate::buffer;
    let line = buffer::line().to_vec();
    let mut start = buffer::point().min(line.len());
    if start > 0 && (start == line.len() || is_white(line[start])) {
        start -= 1;
    }
    while start > 0 && !is_white(line[start - 1]) {
        start -= 1;
    }
    let mut end = start;
    while end < line.len() && !is_white(line[end]) {
        end += 1;
    }
    if start < line.len() && line[start] == b'~' {
        let exp = expand_word(&line[start..end]);
        crate::undo::rl_begin_undo_group();
        buffer::delete(start, end);
        buffer::insert_at(start, &exp);
        crate::undo::rl_end_undo_group();
        buffer::set_point(start + exp.len());
    }
    0
}
