//! History expansion (history(3) "History Expansion"): event designators,
//! word designators, modifiers, quick substitution, quoting rules, and the
//! history tokenizer (`history_tokenize`, `history_arg_extract`).
//! Owner: history workstream.
use super::{entry_at, history_base, history_length, history_offset, search::search_list};
use crate::prelude::*;
use crate::util::c_strvec;

#[no_mangle]
pub static history_comment_char: CVar<c_char> = CVar::new(0);

#[no_mangle]
pub static history_expansion_char: CVar<c_char> = CVar::new(b'!' as c_char);

#[no_mangle]
pub static history_inhibit_expansion_function: CVar<Option<rl_linebuf_func_t>> = CVar::new(None);

#[no_mangle]
pub static history_no_expand_chars: CVar<*mut c_char> = CVar::new(cs_mut!(" \t\n\r="));

#[no_mangle]
pub static history_quotes_inhibit_expansion: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_quoting_state: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_search_delimiter_chars: CVar<*mut c_char> = CVar::new(core::ptr::null_mut());

#[no_mangle]
pub static history_subst_char: CVar<c_char> = CVar::new(b'^' as c_char);

#[no_mangle]
pub static history_word_delimiters: CVar<*mut c_char> = CVar::new(cs_mut!(" \t\n;&()|<>"));

/// State kept between expansions: the last substitution and the last
/// `!?string?` search.
struct State {
    lhs: Option<Vec<u8>>,
    rhs: Vec<u8>,
    search: Option<Vec<u8>>,
    /// the word containing the last substring search match
    search_match: Option<Vec<u8>>,
}
static ST: CVar<State> = CVar::new(State { lhs: None, rhs: Vec::new(), search: None, search_match: None });

fn st() -> &'static mut State {
    ST.get_mut()
}

fn is_white(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n'
}

fn in_set(c: u8, set: *const c_char) -> bool {
    c != 0 && unsafe { cstr(set) }.contains(&c)
}

fn is_word_delim(c: u8) -> bool {
    in_set(c, history_word_delimiters.get())
}

fn char_len(s: &[u8], i: usize) -> usize {
    if crate::mb::multibyte() && s[i] >= 0x80 {
        crate::mb::char_len_at(s, i).max(1)
    } else {
        1
    }
}

// ---------------------------------------------------------------- tokenizer

/// Skip a balanced `( ... )` group starting at `i` (`s[i] == '('`).
/// Quotes are not special inside; backslash escapes.
fn skip_parens(s: &[u8], mut i: usize) -> usize {
    let mut depth = 0;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    s.len()
}

/// End of the token starting at `i0` (not whitespace).
fn token_end(s: &[u8], i0: usize) -> usize {
    let c = s[i0];
    if b"()<>;&|".contains(&c) || (is_word_delim(c) && !is_white(c)) {
        return operator_end(s, i0, i0);
    }
    word_scan(s, i0, i0)
}

/// End of the shell operator at `s[i]` (part of the token from `i0`).
fn operator_end(s: &[u8], i0: usize, i: usize) -> usize {
    let len = s.len();
    let c = s[i];
    let peek = s.get(i + 1).copied().unwrap_or(0);
    if (c == b'<' || c == b'>') && peek == b'(' {
        if i + 2 >= len {
            return len;
        }
        return word_scan(s, i0, skip_parens(s, i + 1));
    }
    if peek == c && c != b'(' && c != b')' {
        if c == b'<' && (s.get(i + 2) == Some(&b'-') || s.get(i + 2) == Some(&b'<')) {
            return i + 3;
        }
        return i + 2;
    }
    if (c == b'&' && peek == b'>') || (c == b'>' && peek == b'|') {
        return i + 2;
    }
    if (c == b'<' || c == b'>') && peek == b'&' {
        return dup_target(s, i + 2);
    }
    i + 1
}

/// After `>&` / `<&`: an fd number or `-`.
fn dup_target(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && (s[i].is_ascii_digit() || s[i] == b'-') {
        i += 1;
    }
    i
}

fn word_scan(s: &[u8], i0: usize, mut i: usize) -> usize {
    let len = s.len();
    let mut quote = 0u8;
    while i < len {
        let c = s[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            } else if c == b'\\' && quote != b'\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == b'\\' {
            i = (i + 2).min(len);
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            quote = c;
            i += 1;
            continue;
        }
        if (c == b'<' || c == b'>') && i > i0 && s[i0..i].iter().all(|d| d.is_ascii_digit()) {
            return operator_end(s, i0, i);
        }
        if b"$@!+?*<>".contains(&c) && s.get(i + 1) == Some(&b'(') {
            if i + 2 >= len {
                return i + 1;
            }
            i = skip_parens(s, i + 1);
            continue;
        }
        if is_word_delim(c) {
            break;
        }
        i += char_len(s, i);
    }
    i
}

/// Token spans of `s` (a comment character at the start of a token ends
/// the list).
pub fn tokenize_spans(s: &[u8]) -> Vec<(usize, usize)> {
    let cc = history_comment_char.get() as u8;
    let mut v = Vec::new();
    let mut i = 0;
    loop {
        while i < s.len() && is_white(s[i]) {
            i += 1;
        }
        if i >= s.len() || (cc != 0 && s[i] == cc) {
            break;
        }
        let e = token_end(s, i).max(i + 1).min(s.len());
        v.push((i, e));
        i = e;
    }
    v
}

pub fn tokenize(s: &[u8]) -> Vec<Vec<u8>> {
    tokenize_spans(s).into_iter().map(|(a, b)| s[a..b].to_vec()).collect()
}

/// The token containing byte `ind`.
fn find_word(s: &[u8], ind: usize) -> Option<Vec<u8>> {
    tokenize_spans(s).into_iter().find(|&(a, b)| ind >= a && ind < b).map(|(a, b)| s[a..b].to_vec())
}

#[no_mangle]
pub unsafe extern "C" fn history_tokenize(s: *const c_char) -> *mut *mut c_char {
    let t = tokenize(cstr(s));
    if t.is_empty() {
        return null_mut();
    }
    c_strvec(&t)
}

/// `history_arg_extract`: words `first..=last` joined by spaces.
pub fn arg_extract(first: c_int, last: c_int, s: &[u8]) -> Option<Vec<u8>> {
    let words = tokenize(s);
    let len = words.len() as c_int;
    if len == 0 {
        return None;
    }
    let (mut first, mut last) = (first, last);
    if last < 0 {
        last = len + last - 1;
    }
    if first < 0 {
        first = len + first - 1;
    }
    if last == b'$' as c_int {
        last = len - 1;
    }
    if first == b'$' as c_int {
        first = len - 1;
    }
    last += 1;
    if first >= len || last > len || first < 0 || last < 0 || first > last {
        return None;
    }
    Some(words[first as usize..last as usize].join(&b' '))
}

#[no_mangle]
pub unsafe extern "C" fn history_arg_extract(first: c_int, last: c_int, s: *const c_char) -> *mut c_char {
    match arg_extract(first, last, cstr(s)) {
        Some(v) => c_strdup(&v),
        None => null_mut(),
    }
}

// ---------------------------------------------------------------- events

/// Parse the event at `s[*idx]` (the expansion character).  Returns the
/// history entry index (0-based) or None; `*idx` ends after the event.
fn event(s: &[u8], idx: &mut usize, qc: u8) -> Option<c_int> {
    let ec = history_expansion_char.get() as u8;
    let mut i = *idx;
    if s.get(i) != Some(&ec) {
        return None;
    }
    i += 1;
    *idx = i;
    let len = history_length.get();
    if s.get(i) == Some(&ec) {
        *idx = i + 1;
        return if len > 0 { Some(len - 1) } else { None };
    }
    // numbers
    let mut sign = 1;
    let mut j = i;
    if s.get(j) == Some(&b'-') && s.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
        sign = -1;
        j += 1;
    }
    if s.get(j).is_some_and(|c| c.is_ascii_digit()) {
        let mut n: i64 = 0;
        while let Some(c) = s.get(j).filter(|c| c.is_ascii_digit()) {
            n = (n * 10 + (c - b'0') as i64).min(i32::MAX as i64);
            j += 1;
        }
        *idx = j;
        let which = if sign < 0 { len as i64 - n } else { n - history_base.get() as i64 };
        if which < 0 || which >= len as i64 || (sign < 0 && n == 0) {
            return None;
        }
        return Some(which as c_int);
    }
    // string searches
    let substring = s.get(i) == Some(&b'?');
    if substring {
        i += 1;
    }
    let start = i;
    while i < s.len() {
        let c = s[i];
        if substring {
            if c == b'?' || c == b'\n' {
                break;
            }
        } else {
            if is_white(c) || c == b':' || (qc != 0 && c == qc) {
                break;
            }
            if i > start && b"^$*%-".contains(&c) {
                break;
            }
            if in_set(c, history_search_delimiter_chars.get()) {
                break;
            }
        }
        i += char_len(s, i);
    }
    let i = i.min(s.len());
    let mut text = s[start..i].to_vec();
    let mut end = i;
    if substring && s.get(i) == Some(&b'?') {
        end = i + 1;
    }
    *idx = end;
    if text.is_empty() {
        if !substring {
            return None;
        }
        text = st().search.clone()?;
    }
    let old = history_offset.get();
    if history_offset.get() > len {
        history_offset.set(len);
    }
    let r = search_list(&text, -1, !substring);
    let found = history_offset.get();
    history_offset.set(old);
    if r < 0 {
        return None;
    }
    if substring {
        let line = super::line_at(found).unwrap_or(&[]);
        st().search_match = find_word(line, r as usize);
        st().search = Some(text);
    }
    Some(found)
}

#[no_mangle]
pub unsafe extern "C" fn get_history_event(s: *const c_char, idx: *mut c_int, qc: c_int) -> *mut c_char {
    if s.is_null() || idx.is_null() {
        return null_mut();
    }
    let b = cstr(s);
    let mut i = (*idx).max(0) as usize;
    let r = event(b, &mut i, qc as u8);
    *idx = i as c_int;
    match r {
        Some(n) => {
            let e = entry_at(n);
            if e.is_null() {
                null_mut()
            } else {
                (*e).line
            }
        }
        None => null_mut(),
    }
}

// ---------------------------------------------------------------- expansion

enum Err {
    EventNotFound,
    BadWord,
    SubstFailed,
    BadModifier,
    NoPrevSubst,
}

fn errmsg(s: &[u8], a: usize, b: usize, e: Err) -> Vec<u8> {
    let a = a.min(s.len());
    let b = b.clamp(a, s.len());
    let mut v = s[a..b].to_vec();
    v.extend_from_slice(match e {
        Err::EventNotFound => b": event not found",
        Err::BadWord => b": bad word specifier",
        Err::SubstFailed => b": substitution failed",
        Err::BadModifier => b": unrecognized history modifier",
        Err::NoPrevSubst => b": no previous substitution",
    });
    v
}

/// Word designator selection on `line`; Ok(None) when there is none.
enum Words {
    All,
    Range(i32, i32),
    Percent,
    /// `x-`: from x to the next-to-last word
    ToPenultimate(i32),
    /// `*` with no first word: arguments, empty if none
    Args,
}

fn parse_num(s: &[u8], i: &mut usize) -> i32 {
    let mut n: i64 = 0;
    while let Some(c) = s.get(*i).filter(|c| c.is_ascii_digit()) {
        n = (n * 10 + (c - b'0') as i64).min(i32::MAX as i64);
        *i += 1;
    }
    n as i32
}

/// Parse a word designator at `*i` (just after the event).
fn word_spec(s: &[u8], i: &mut usize) -> Words {
    let mut j = *i;
    let c = s.get(j).copied().unwrap_or(0);
    if c == b':' {
        let n = s.get(j + 1).copied().unwrap_or(0);
        if n.is_ascii_digit() || b"^$*%-".contains(&n) {
            j += 1;
        } else {
            return Words::All;
        }
    } else if !b"^$*%-".contains(&c) || c == 0 {
        return Words::All;
    }
    let c = s[j];
    let first: i32;
    match c {
        b'%' => {
            *i = j + 1;
            return Words::Percent;
        }
        b'*' => {
            *i = j + 1;
            return Words::Args;
        }
        b'$' => {
            *i = j + 1;
            return Words::Range(-1, -1);
        }
        b'^' => {
            first = 1;
            j += 1;
        }
        b'-' => {
            first = 0;
        }
        _ => {
            first = parse_num(s, &mut j);
        }
    }
    match s.get(j).copied().unwrap_or(0) {
        b'*' => {
            *i = j + 1;
            Words::Range(first, -1)
        }
        b'^' => {
            *i = j + 1;
            Words::Range(first, 1)
        }
        b'-' => {
            j += 1;
            let n = s.get(j).copied().unwrap_or(0);
            if n.is_ascii_digit() {
                let last = parse_num(s, &mut j);
                *i = j;
                Words::Range(first, last)
            } else if n == b'$' {
                *i = j + 1;
                Words::Range(first, -1)
            } else if n == b'^' {
                *i = j + 1;
                Words::Range(first, 1)
            } else {
                *i = j;
                Words::ToPenultimate(first)
            }
        }
        _ => {
            *i = j;
            Words::Range(first, first)
        }
    }
}

/// Apply a word selection; None means a bad word specifier.
fn select_words(line: &[u8], w: &Words) -> Option<Vec<u8>> {
    let words = tokenize(line);
    let n = words.len() as i32;
    let last_ix = n - 1;
    let join = |a: i32, b: i32| -> Vec<u8> {
        if a > b || a < 0 {
            return Vec::new();
        }
        words[a as usize..=b as usize].join(&b' ')
    };
    match *w {
        Words::All => Some(line.to_vec()),
        Words::Percent => Some(st().search_match.clone().unwrap_or_default()),
        Words::Args => Some(if n > 1 { join(1, last_ix) } else { Vec::new() }),
        Words::ToPenultimate(f) => {
            if f > last_ix || n == 0 {
                return None;
            }
            Some(join(f, last_ix - 1))
        }
        Words::Range(-1, -1) if n == 0 => Some(line.to_vec()),
        Words::Range(f, l) => {
            let f = if f < 0 { last_ix } else { f };
            let l = if l < 0 { last_ix } else { l };
            if n == 0 || f > last_ix || l > last_ix || f > l || f < 0 {
                return None;
            }
            Some(join(f, l))
        }
    }
}

fn quote(s: &[u8]) -> Vec<u8> {
    let mut v = vec![b'\''];
    for &c in s {
        if c == b'\'' {
            v.extend_from_slice(b"'\\''");
        } else {
            v.push(c);
        }
    }
    v.push(b'\'');
    v
}

fn quote_words(s: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    for (n, field) in s.split(|&c| is_white(c)).enumerate() {
        if n > 0 {
            // the separator that ended the previous field
            let pos: usize = s.split(|&c| is_white(c)).take(n).map(|f| f.len() + 1).sum::<usize>() - 1;
            v.push(s[pos]);
        }
        v.extend_from_slice(&quote(field));
    }
    v
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() || needle.len() > hay.len() - from {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// The replacement text: `&` is the matched text, `\&` a literal `&`.
fn replacement(rhs: &[u8], lhs: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < rhs.len() {
        if rhs[i] == b'\\' && rhs.get(i + 1) == Some(&b'&') {
            v.push(b'&');
            i += 2;
        } else if rhs[i] == b'&' {
            v.extend_from_slice(lhs);
            i += 1;
        } else {
            v.push(rhs[i]);
            i += 1;
        }
    }
    v
}

/// Substitute in `text`; None when nothing matched.
fn substitute(text: &[u8], lhs: &[u8], rhs: &[u8], global: bool, bywords: bool) -> Option<Vec<u8>> {
    let rep = replacement(rhs, lhs);
    if bywords {
        // after a replacement, continue after the end of the word (the
        // reference's word bookkeeping starts at the second character,
        // so a match at 0 leaves the rest of the first word open)
        let mut t = text.to_vec();
        let mut any = false;
        let mut we = 0usize;
        let mut si = 0usize;
        while si < t.len() {
            if si > we {
                while si < t.len() && is_white(t[si]) {
                    si += 1;
                }
                if si >= t.len() {
                    break;
                }
                we = token_end(&t, si);
            }
            if t[si..].starts_with(lhs) {
                any = true;
                t.splice(si..si + lhs.len(), rep.iter().copied());
                si = we;
            }
            si += 1;
        }
        let out = t;
        return if any { Some(out) } else { None };
    }
    let mut out = Vec::new();
    let mut pos = 0;
    let mut any = false;
    while let Some(p) = find(text, lhs, pos) {
        any = true;
        out.extend_from_slice(&text[pos..p]);
        out.extend_from_slice(&rep);
        pos = p + lhs.len();
        if !global {
            break;
        }
    }
    if !any {
        return None;
    }
    out.extend_from_slice(&text[pos..]);
    Some(out)
}

/// Read a substitution part up to the delimiter; `\delim` is the
/// delimiter, `\&` is kept for the rhs.  Returns (text, index after).
fn subst_part(s: &[u8], mut i: usize, delim: u8, is_rhs: bool) -> (Vec<u8>, usize, bool) {
    let mut v = Vec::new();
    while i < s.len() {
        let c = s[i];
        let n = char_len(s, i);
        if n > 1 {
            v.extend_from_slice(&s[i..(i + n).min(s.len())]);
            i += n;
            continue;
        }
        if c == b'\\' && i + 1 < s.len() && s[i + 1] == delim {
            v.push(delim);
            i += 2;
            continue;
        }
        if c == b'\\' && is_rhs && i + 1 < s.len() && s[i + 1] == b'&' {
            v.extend_from_slice(b"\\&");
            i += 2;
            continue;
        }
        if c == delim {
            return (v, i + 1, true);
        }
        v.push(c);
        i += 1;
    }
    (v, i, false)
}

/// Expand one history reference at `s[start]`.  Returns the text and the
/// index after the reference.
fn expand_one(s: &[u8], start: usize, qc: u8, so_far: &[u8], print: &mut bool) -> Result<(Vec<u8>, usize), Vec<u8>> {
    let ec = history_expansion_char.get() as u8;
    let mut i = start + 1;
    let line: Vec<u8>;
    let c = s.get(i).copied().unwrap_or(0);
    if c == b'#' {
        i += 1;
        line = so_far.to_vec();
    } else if c != 0 && b":$*%^".contains(&c) {
        let len = history_length.get();
        if len == 0 {
            return Err(errmsg(s, start, i, Err::EventNotFound));
        }
        line = super::line_at(len - 1).unwrap_or(&[]).to_vec();
    } else {
        let mut j = start;
        match event(s, &mut j, qc) {
            Some(n) => {
                line = super::line_at(n).unwrap_or(&[]).to_vec();
                i = j;
            }
            None => return Err(errmsg(s, start, j, Err::EventNotFound)),
        }
    }
    // word designator
    let wstart = i;
    let w = word_spec(s, &mut i);
    let mut text = match select_words(&line, &w) {
        Some(t) => t,
        None => return Err(errmsg(s, wstart, i, Err::BadWord)),
    };
    // modifiers
    let mstart = i;
    let mut quoting = 0u8;
    while s.get(i) == Some(&b':') {
        let mut c = s.get(i + 1).copied().unwrap_or(0);
        let mut j = i + 1;
        let mut global = false;
        let mut bywords = false;
        if c == b'g' || c == b'a' {
            global = true;
            j += 1;
            c = s.get(j).copied().unwrap_or(0);
        } else if c == b'G' {
            bywords = true;
            j += 1;
            c = s.get(j).copied().unwrap_or(0);
        }
        match c {
            b'h' => {
                if let Some(p) = text.iter().rposition(|&c| c == b'/') {
                    text.truncate(p);
                }
                i = j + 1;
            }
            b't' => {
                if let Some(p) = text.iter().rposition(|&c| c == b'/') {
                    text = text[p + 1..].to_vec();
                }
                i = j + 1;
            }
            b'r' => {
                if let Some(p) = text.iter().rposition(|&c| c == b'.') {
                    text.truncate(p);
                }
                i = j + 1;
            }
            b'e' => {
                if let Some(p) = text.iter().rposition(|&c| c == b'.') {
                    text = text[p..].to_vec();
                }
                i = j + 1;
            }
            b'p' => {
                *print = true;
                i = j + 1;
            }
            b'q' => {
                quoting = b'q';
                i = j + 1;
            }
            b'x' => {
                quoting = b'x';
                i = j + 1;
            }
            b's' | b'&' => {
                let (lhs, rhs);
                if c == b's' {
                    let delim = s.get(j + 1).copied().unwrap_or(0);
                    if delim == 0 {
                        i = j + 1;
                        continue;
                    }
                    let (l, k, _) = subst_part(s, j + 2, delim, false);
                    let (r, k2, _) = subst_part(s, k, delim, true);
                    i = k2;
                    let l = if l.is_empty() {
                        match st().lhs.clone().or_else(|| st().search.clone()) {
                            Some(p) => p,
                            None => return Err(errmsg(s, mstart, i, Err::NoPrevSubst)),
                        }
                    } else {
                        l
                    };
                    st().lhs = Some(l.clone());
                    st().rhs = r.clone();
                    lhs = l;
                    rhs = r;
                } else {
                    i = j + 1;
                    match st().lhs.clone() {
                        Some(l) => {
                            lhs = l;
                            rhs = st().rhs.clone();
                        }
                        None => return Err(errmsg(s, mstart, i, Err::NoPrevSubst)),
                    }
                }
                match substitute(&text, &lhs, &rhs, global, bywords) {
                    Some(t) => text = t,
                    None => return Err(errmsg(s, mstart, i, Err::SubstFailed)),
                }
            }
            _ => return Err(errmsg(s, j, j + 1, Err::BadModifier)),
        }
    }
    match quoting {
        b'q' => text = quote(&text),
        b'x' => text = quote_words(&text),
        _ => {}
    }
    Ok((text, i))
}

fn call_inhibit(s: &[u8], i: usize) -> bool {
    match history_inhibit_expansion_function.get() {
        Some(f) => unsafe {
            let c = c_strdup(s);
            let r = f(c, i as c_int);
            cfree(c);
            r != 0
        },
        None => false,
    }
}

/// Index of the closing single quote at or after `i` (or `s.len()`).
fn squote_close(s: &[u8], mut i: usize, allow_esc: bool) -> usize {
    while i < s.len() && s[i] != b'\'' {
        if allow_esc && s[i] == b'\\' && i + 1 < s.len() {
            i += 1;
        }
        i += char_len(s, i);
    }
    i.min(s.len())
}

/// Result of scanning: does `s` contain an expansion that should happen.
fn needs_expansion(s: &[u8]) -> bool {
    let ec = history_expansion_char.get() as u8;
    let cc = history_comment_char.get() as u8;
    let inhibit = history_quotes_inhibit_expansion.get() != 0;
    let mut dquote = history_quoting_state.get() == b'"' as c_int;
    let mut squote = history_quoting_state.get() == b'\'' as c_int && !inhibit;
    let mut i = 0;
    if history_quoting_state.get() == b'\'' as c_int && inhibit {
        i = squote_close(s, 0, false);
        if i >= s.len() {
            return false;
        }
        i += 1;
    }
    while i < s.len() {
        let c = s[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if c == b'"' && !squote {
            dquote = !dquote;
        } else if c == b'\'' && !inhibit && !dquote {
            squote = !squote;
        } else if c == b'\'' && inhibit && !dquote {
            let esc = i > 0 && s[i - 1] == b'$';
            i = squote_close(s, i + 1, esc);
            if i >= s.len() {
                return false;
            }
        } else if cc != 0 && c == cc && !(dquote && inhibit) && (i == 0 || is_word_delim(s[i - 1])) {
            return false;
        } else if c == ec {
            let n = s.get(i + 1).copied().unwrap_or(0);
            if n != 0 && !in_set(n, history_no_expand_chars.get()) && !(dquote && n == b'"') && !(inhibit && i > 0 && s[i - 1] == b'\\') && !call_inhibit(s, i) {
                return true;
            }
        }
        i += char_len(s, i);
    }
    false
}

/// history_expand: (status, output).
pub fn expand(input: &[u8]) -> (c_int, Vec<u8>) {
    let ec = history_expansion_char.get() as u8;
    let sc = history_subst_char.get() as u8;
    if ec == 0 {
        return (0, input.to_vec());
    }
    let inhibit = history_quotes_inhibit_expansion.get() != 0;
    let squote_state = history_quoting_state.get() == b'\'' as c_int;
    let owned;
    let s: &[u8] = if sc != 0 && input.first() == Some(&sc) && !(squote_state && inhibit) {
        let mut v = vec![ec, ec, b':', b's'];
        v.extend_from_slice(input);
        owned = v;
        &owned
    } else {
        if !needs_expansion(input) {
            return (0, input.to_vec());
        }
        input
    };
    let cc = history_comment_char.get() as u8;
    let mut out = Vec::new();
    let mut dquote = history_quoting_state.get() == b'"' as c_int;
    let mut squote = squote_state && !inhibit;
    let mut print = false;
    let mut changed = false;
    let mut i = 0;
    if squote_state && inhibit {
        let e = squote_close(s, 0, false);
        let e = if e >= s.len() { s.len() } else { e + 1 };
        out.extend_from_slice(&s[..e]);
        i = e;
    }
    while i < s.len() {
        let c = s[i];
        if c == b'\\' {
            let e = (i + 2).min(s.len());
            out.extend_from_slice(&s[i..e]);
            i = e;
            continue;
        }
        if c == b'"' && !squote {
            dquote = !dquote;
        } else if c == b'\'' && !inhibit && !dquote {
            squote = !squote;
        } else if c == b'\'' && inhibit && !dquote {
            let esc = i > 0 && s[i - 1] == b'$';
            let e = squote_close(s, i + 1, esc);
            let e = if e >= s.len() { s.len() } else { e + 1 };
            out.extend_from_slice(&s[i..e]);
            i = e;
            continue;
        } else if cc != 0 && c == cc && !(dquote && inhibit) && (i == 0 || is_word_delim(s[i - 1])) {
            out.extend_from_slice(&s[i..]);
            break;
        } else if c == ec {
            let n = s.get(i + 1).copied().unwrap_or(0);
            if n != 0 && !in_set(n, history_no_expand_chars.get()) && !(dquote && n == b'"') && !(inhibit && i > 0 && s[i - 1] == b'\\') {
                let mut probe = out.clone();
                probe.push(c);
                probe.push(n);
                if !call_inhibit(&probe, out.len()) {
                    let qc = if dquote {
                        b'"'
                    } else if squote {
                        b'\''
                    } else {
                        0
                    };
                    match expand_one(s, i, qc, &out, &mut print) {
                        Ok((t, e)) => {
                            out.extend_from_slice(&t);
                            i = e;
                            changed = true;
                            continue;
                        }
                        Err(m) => return (-1, m),
                    }
                }
            }
        }
        let n = char_len(s, i);
        out.extend_from_slice(&s[i..(i + n).min(s.len())]);
        i += n;
    }
    if print {
        (2, out)
    } else if changed {
        (1, out)
    } else {
        (0, out)
    }
}

#[no_mangle]
pub unsafe extern "C" fn history_expand(s: *const c_char, out: *mut *mut c_char) -> c_int {
    let (r, v) = expand(cstr(s));
    if !out.is_null() {
        *out = c_strdup(&v);
    }
    r
}
