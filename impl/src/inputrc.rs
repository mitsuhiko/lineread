//! The init file (inputrc) parser: `set`, key bindings (`"seq": cmd`,
//! `Control-x: cmd`, macros), `$if/$else/$endif/$include`, error messages.
//! Owner: bind/inputrc workstream.
use crate::bind::{generic_bind_raw, translate_keyseq_bytes};
use crate::prelude::*;
use crate::util::ByteBufExt;

/// File/line of the init file being read (for error messages).  `file` is
/// None after an `$include` returned (the reference loses the context).
struct Ctx {
    file: Option<Vec<u8>>,
    line: usize,
}
static CTX: CVar<Vec<Ctx>> = CVar::new(Vec::new());
/// Conditional stack: (this branch active, some branch taken, parent active).
/// Shared across included files.
static IFSTACK: CVar<Vec<(bool, bool, bool)>> = CVar::new(Vec::new());
/// Last init file read (rl_read_init_file).
static LAST_FILE: CVar<Option<Vec<u8>>> = CVar::new(None);

/// Print a parser/variable error to stderr: "readline: FILE: line N: MSG".
pub fn report_error(msg: &[u8]) {
    let mut m = b"readline: ".to_vec();
    if let Some(Ctx { file: Some(f), line }) = CTX.get_mut().last() {
        m.extend_from_slice(f);
        m.push_str(": line ");
        m.push_int(*line as i64);
        m.push_str(": ");
    }
    m.extend_from_slice(msg);
    m.push(b'\n');
    unsafe {
        crate::terminal::flush();
        let e = crate::util::lr_stderr();
        crate::util::fwrite_bytes(e, &m);
        crate::util::fflush(e);
    }
}

#[inline]
fn ws(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && ws(s[i]) {
        i += 1;
    }
    i
}

/// Index of the closing `delim` starting the scan at `i` (backslash
/// escapes skipped), or `s.len()` when there is none.
fn skip_to_delim(s: &[u8], mut i: usize, delim: u8) -> usize {
    while i < s.len() && s[i] != delim {
        if s[i] == b'\\' && i + 1 < s.len() {
            i += 1;
        }
        i += 1;
    }
    i.min(s.len())
}

fn active() -> bool {
    IFSTACK.get_mut().last().map(|e| e.0 && e.2).unwrap_or(true)
}

fn trim_end(s: &[u8]) -> &[u8] {
    let mut b = s.len();
    while b > 0 && ws(s[b - 1]) {
        b -= 1;
    }
    &s[..b]
}

fn strip_prefix_ci<'a>(s: &'a [u8], p: &[u8]) -> Option<&'a [u8]> {
    if s.len() >= p.len() && s[..p.len()].eq_ignore_ascii_case(p) {
        Some(&s[p.len()..])
    } else {
        None
    }
}

fn contains_ci(s: &[u8], pat: &[u8]) -> bool {
    s.len() >= pat.len() && s.windows(pat.len()).any(|w| w.eq_ignore_ascii_case(pat))
}

fn quote_err(what: &[u8], found: &[u8]) {
    let mut m = what.to_vec();
    m.push_str(" expected, found `");
    m.extend_from_slice(found);
    m.push(b'\'');
    report_error(&m);
}

/// Parse a comparison operator at the start of `s`: (op, length).
fn parse_op(s: &[u8]) -> Option<(&'static [u8], usize)> {
    for op in [&b"=="[..], b"!=", b">=", b"<=", b"=", b">", b"<"] {
        if s.starts_with(op) {
            return Some((op, op.len()));
        }
    }
    None
}

/// `$if version OP N[.M]`.
fn version_cond(rest: &[u8]) -> bool {
    let r = &rest[skip_ws(rest, 0)..];
    let r = trim_end(r);
    if r.is_empty() {
        quote_err(b"comparison operator", b"end-of-line");
        return false;
    }
    let Some((op, n)) = parse_op(r) else {
        quote_err(b"comparison operator", r);
        return false;
    };
    let r = &r[skip_ws(r, n)..];
    // major[.minor]
    let mut i = 0;
    while i < r.len() && r[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        quote_err(b"numeric argument", r);
        return false;
    }
    let major = crate::vars::atoi(&r[..i]);
    let mut minor = 0;
    if i < r.len() && r[i] == b'.' {
        let s = i + 1;
        let mut j = s;
        while j < r.len() && r[j].is_ascii_digit() {
            j += 1;
        }
        if j == s {
            quote_err(b"numeric argument", r);
            return false;
        }
        minor = crate::vars::atoi(&r[s..j]);
        i = j;
    }
    if i < r.len() {
        if !ws(r[i]) {
            quote_err(b"numeric argument", r);
            return false;
        }
        let k = skip_ws(r, i);
        if k >= r.len() || r[k] == b'#' {
            // a trailing comment is fine
            i = r.len();
        }
    }
    if i < r.len() {
        let mut m = b"trailing garbage on line: `".to_vec();
        m.extend_from_slice(&r[i..]);
        m.push(b'\'');
        report_error(&m);
        return false;
    }
    let want = major * 256 + minor;
    let have = 0x0803;
    match op {
        b">=" => have >= want,
        b"<=" => have <= want,
        b"==" | b"=" => have == want,
        b"!=" => have != want,
        b">" => have > want,
        b"<" => have < want,
        _ => false,
    }
}

/// `$if VARIABLE OP VALUE` (only == / = / !=).
fn variable_cond(name: &[u8], rest: &[u8]) -> bool {
    let r = &rest[skip_ws(rest, 0)..];
    if r.is_empty() {
        quote_err(b"equality comparison operator", b"end-of-line");
        return false;
    }
    let (neg, n) = if r.starts_with(b"==") {
        (false, 2)
    } else if r.starts_with(b"!=") {
        (true, 2)
    } else if r.starts_with(b"=") {
        (false, 1)
    } else {
        quote_err(b"equality comparison operator", trim_end(r));
        return false;
    };
    let r = &r[skip_ws(r, n)..];
    let e = r.iter().position(|&c| ws(c)).unwrap_or(r.len());
    let val = &r[..e];
    if val.is_empty() {
        quote_err(b"argument", b"");
        return false;
    }
    let cur = crate::vars::variable_value(name).unwrap_or_default();
    cur.eq_ignore_ascii_case(val) != neg
}

/// `$if` test.
fn eval_cond(arg: &[u8]) -> bool {
    if let Some(rest) = strip_prefix_ci(arg, b"mode=") {
        let rest = trim_end(rest);
        let m = if crate::readline::rl_editing_mode.get() == crate::readline::EMACS_MODE { &b"emacs"[..] } else { b"vi" };
        return rest.eq_ignore_ascii_case(m);
    }
    if let Some(rest) = strip_prefix_ci(arg, b"term=") {
        let rest = trim_end(rest);
        let t = unsafe { cstr(crate::terminal::rl_terminal_name.get()).to_vec() };
        let short: &[u8] = t.split(|&c| c == b'-').next().unwrap_or(&t);
        return rest.eq_ignore_ascii_case(&t) || rest.eq_ignore_ascii_case(short);
    }
    // the first word: `version`, a variable name or the application name
    let e = arg.iter().position(|&c| ws(c)).unwrap_or(arg.len());
    let word = &arg[..e];
    if let Some(rest) = strip_prefix_ci(arg, b"version") {
        if rest.is_empty() || ws(rest[0]) || parse_op(rest).is_some() {
            return version_cond(rest);
        }
    }
    if crate::vars::is_variable(word) {
        return variable_cond(word, &arg[e..]);
    }
    unsafe { word.eq_ignore_ascii_case(cstr(crate::readline::rl_readline_name.get())) }
}

/// Handle a `$` directive line.
fn directive(line: &[u8]) -> c_int {
    let i = skip_ws(line, 1);
    let body = &line[i..];
    let end = body.iter().position(|&c| ws(c)).unwrap_or(body.len());
    let word = &body[..end];
    let arg = &body[skip_ws(body, end)..];
    let st = IFSTACK.get_mut();
    if word.eq_ignore_ascii_case(b"if") {
        let parent = st.last().map(|e| e.0 && e.2).unwrap_or(true);
        // the test is evaluated (and may report errors) only when the
        // enclosing branch is active
        let v = parent && eval_cond(arg);
        IFSTACK.get_mut().push((v, v, parent));
        return 0;
    }
    if word.eq_ignore_ascii_case(b"else") {
        match st.last_mut() {
            // (each $else toggles the branch)
            Some(e) => e.0 = !e.0,
            None => {
                report_error(b"$else found without matching $if");
                return 0;
            }
        }
        return 0;
    }
    if word.eq_ignore_ascii_case(b"endif") {
        if st.pop().is_none() {
            report_error(b"$endif without matching $if");
        }
        return 0;
    }
    if word.eq_ignore_ascii_case(b"include") {
        if !active() {
            return 0;
        }
        let path = expand_tilde(arg);
        read_file(&path, false);
        // the reference forgets the including file's name afterwards
        if let Some(c) = CTX.get_mut().last_mut() {
            c.file = None;
        }
        return 0;
    }
    let mut m = word.to_vec();
    m.push_str(": unknown parser directive");
    report_error(&m);
    1
}

/// Tilde expansion of a file name (`~`, `~/...`, `~user/...`).
pub fn expand_tilde(p: &[u8]) -> Vec<u8> {
    unsafe {
        let c = c_strdup(p);
        let e = crate::tilde::tilde_expand(c);
        cfree(c);
        if !e.is_null() {
            let v = cstr(e).to_vec();
            cfree(e);
            if !v.starts_with(b"~") || !p.starts_with(b"~") {
                return v;
            }
        }
        if p == b"~" || p.starts_with(b"~/") {
            if let Some(h) = crate::util::getenv_bytes(c"HOME") {
                let mut v = h.to_vec();
                v.extend_from_slice(&p[1..]);
                return v;
            }
        }
        p.to_vec()
    }
}

/// The key named by the last component of a key name (`rubout`, `tab`, or
/// the first character).
fn key_from_name(kname: &[u8]) -> u8 {
    let named: &[(&[u8], u8)] = &[
        (b"DEL", 0x7f),
        (b"ESC", 0x1b),
        (b"ESCAPE", 0x1b),
        (b"LFD", b'\n'),
        (b"NEWLINE", b'\n'),
        (b"RET", b'\r'),
        (b"RETURN", b'\r'),
        (b"RUBOUT", 0x7f),
        (b"SPACE", b' '),
        (b"SPC", b' '),
        (b"TAB", b'\t'),
    ];
    for (n, v) in named {
        if kname.eq_ignore_ascii_case(n) {
            return *v;
        }
    }
    kname.first().copied().unwrap_or(0)
}

/// A symbolic key name (`Control-a`, `Meta-Rubout`, `TAB`, `x`) to a key.
/// Err(()) after reporting an unknown modifier.
fn keyname_to_key(name: &[u8]) -> Result<c_int, ()> {
    let kname = match name.iter().rposition(|&c| c == b'-') {
        Some(p) => &name[p + 1..],
        None => name,
    };
    let mut key = key_from_name(kname) as c_int;
    let mut found = false;
    if [&b"Control-"[..], b"C-", b"CTRL-"].iter().any(|p| contains_ci(name, p)) {
        key = (key as u8).to_ascii_uppercase() as c_int & 0x1f;
        found = true;
    }
    if [&b"Meta-"[..], b"M-"].iter().any(|p| contains_ci(name, p)) {
        key |= 0x80;
        found = true;
    }
    if !found && kname.len() != name.len() {
        let mut m = name.to_vec();
        m.push_str(": unknown key modifier");
        report_error(&m);
        return Err(());
    }
    Ok(key)
}

/// `set VAR VALUE` (the line after the `set` word starts at `i`).
fn set_command(line: &[u8], mut i: usize) -> c_int {
    if i < line.len() && line[i] == b':' {
        i += 1;
    }
    i = skip_ws(line, i);
    let rest = &line[i..];
    let ne = rest.iter().position(|&c| ws(c)).unwrap_or(rest.len());
    let name = &rest[..ne];
    let raw = &rest[skip_ws(rest, ne)..];
    if !crate::vars::is_variable(name) {
        let mut m = name.to_vec();
        m.push_str(": unknown variable name");
        report_error(&m);
        return 1;
    }
    let val: &[u8] = if crate::vars::is_bool_variable(name) {
        let e = raw.iter().position(|&c| ws(c)).unwrap_or(raw.len());
        &raw[..e]
    } else if raw.first() == Some(&b'"') {
        let e = skip_to_delim(raw, 1, b'"');
        &raw[1..e]
    } else {
        trim_end(raw)
    };
    crate::vars::variable_bind(name, Some(val));
    0
}

/// Parse and execute one init-file line.  Returns 0 on success.
pub fn parse_line(line: &[u8]) -> c_int {
    let i = skip_ws(line, 0);
    let line = &line[i..];
    if line.is_empty() || line[0] == b'#' {
        return 0;
    }
    if line[0] == b'$' {
        return directive(line);
    }
    if !active() {
        return 0;
    }
    // the key part: a quoted key sequence or a key name
    let mut j;
    let quoted = line[0] == b'"';
    if quoted {
        j = skip_to_delim(line, 1, b'"');
        if j >= line.len() {
            let mut m = line.to_vec();
            m.push_str(": no closing `\"' in key binding");
            report_error(&m);
            return 1;
        }
        j += 1;
        // anything up to the separator is ignored
        while j < line.len() && line[j] != b':' && !ws(line[j]) {
            j += 1;
        }
        if j >= line.len() {
            let mut m = line.to_vec();
            m.push_str(": no key sequence terminator");
            report_error(&m);
            return 1;
        }
    } else {
        j = 0;
        while j < line.len() && line[j] != b':' && !ws(line[j]) {
            j += 1;
        }
        let word = &line[..j];
        if word.eq_ignore_ascii_case(b"set") {
            return set_command(line, j);
        }
        if word.is_empty() {
            let mut m = b"`".to_vec();
            m.extend_from_slice(line);
            m.push_str("': invalid key binding: missing key sequence");
            report_error(&m);
            return 1;
        }
        if j >= line.len() {
            let mut m = word.to_vec();
            m.push_str(": no key sequence terminator");
            report_error(&m);
            return 1;
        }
    }
    let keypart = &line[..j];
    if j < line.len() && line[j] == b':' {
        j += 1;
        // old-style `key:= value` equivalences are ignored
        if j < line.len() && line[j] == b'=' {
            return 0;
        }
    }
    j = skip_ws(line, j);
    let value_start = j;
    // isolate the value: a quoted macro, else a function name; both end at
    // the next whitespace
    let mut e = j;
    if e < line.len() && (line[e] == b'"' || line[e] == b'\'') {
        let q = line[e];
        e = skip_to_delim(line, e + 1, q);
        if e >= line.len() {
            let mut m = b"`".to_vec();
            m.extend_from_slice(&line[value_start..]);
            m.push_str("': missing closing quote for macro");
            report_error(&m);
            return 1;
        }
        e += 1;
    }
    while e < line.len() && !ws(line[e]) {
        e += 1;
    }
    let value = &line[value_start..e];
    let map = crate::keymap::current();
    unsafe {
        // the key sequence
        // (a key name binds its single key: macros and prefix-meta as that
        // byte, functions like rl_bind_key)
        let keys: Vec<u8>;
        let mut prefix_meta = false;
        let mut name_key: Option<c_int> = None;
        if quoted {
            let close = skip_to_delim(keypart, 1, b'"');
            keys = translate_keyseq_bytes(&keypart[1..close]).0;
        } else {
            let Ok(k) = keyname_to_key(keypart) else { return 1 };
            keys = vec![k as u8];
            name_key = Some(k);
            prefix_meta = value.eq_ignore_ascii_case(b"prefix-meta");
        }
        if !value.is_empty() && (value[0] == b'"' || value[0] == b'\'') {
            let mut body = &value[1..];
            if body.last() == Some(&value[0]) {
                body = &body[..body.len() - 1];
            }
            let m = translate_keyseq_bytes(body).0;
            generic_bind_raw(ISMACR, &keys, KmData { macro_: c_strdup(&m) }, map);
            return 0;
        }
        if prefix_meta {
            let meta = crate::keymap::emacs_meta();
            generic_bind_raw(ISKMAP, &keys, KmData { map: meta }, map);
            return 0;
        }
        // unknown function names unbind the key
        let f = crate::funmap::named_function(value);
        match name_key {
            Some(k) => {
                crate::bind::rl_bind_key_in_map(k, f, map);
            }
            None => {
                generic_bind_raw(ISFUNC, &keys, KmData { func: f }, map);
            }
        }
    }
    0
}

/// Read an init file.  Returns 0 or an errno value.
pub fn read_file(path: &[u8], top: bool) -> c_int {
    use std::os::unix::ffi::OsStrExt;
    let data = match std::fs::read(std::ffi::OsStr::from_bytes(path)) {
        Ok(d) => d,
        Err(e) => return e.raw_os_error().unwrap_or(libc::ENOENT),
    };
    if top {
        LAST_FILE.set(Some(path.to_vec()));
        IFSTACK.get_mut().clear();
    }
    CTX.get_mut().push(Ctx { file: Some(path.to_vec()), line: 0 });
    let mut lines: Vec<&[u8]> = data.split(|&c| c == b'\n').collect();
    if data.ends_with(b"\n") {
        lines.pop();
    }
    for (n, l) in lines.into_iter().enumerate() {
        CTX.get_mut().last_mut().unwrap().line = n + 1;
        parse_line(l);
    }
    CTX.get_mut().pop();
    if top {
        IFSTACK.get_mut().clear();
    }
    0
}

/// $INPUTRC, else ~/.inputrc, else /etc/inputrc.
pub fn read_startup_init_file() {
    unsafe { rl_read_init_file(null()) };
}

#[no_mangle]
pub unsafe extern "C" fn rl_parse_and_bind(line: *mut c_char) -> c_int {
    let l = cstr(line).to_vec();
    parse_line(&l)
}

#[no_mangle]
pub unsafe extern "C" fn rl_read_init_file(path: *const c_char) -> c_int {
    let p: Vec<u8> = match cstr_opt(path) {
        Some(p) if !p.is_empty() => expand_tilde(p),
        _ => match LAST_FILE.get_mut().clone() {
            Some(p) => p,
            None => {
                let e = crate::util::getenv_bytes(c"INPUTRC").map(|v| v.to_vec());
                match e {
                    Some(v) if !v.is_empty() => expand_tilde(&v),
                    _ => {
                        let home = expand_tilde(b"~/.inputrc");
                        let r = read_file(&home, true);
                        if r == 0 {
                            return 0;
                        }
                        return read_file(b"/etc/inputrc", true);
                    }
                }
            }
        },
    };
    read_file(&p, true)
}

#[no_mangle]
pub unsafe extern "C" fn rl_re_read_init_file(_count: c_int, _key: c_int) -> c_int {
    let r = rl_read_init_file(null());
    crate::keymap::rl_set_keymap_from_edit_mode();
    r
}
