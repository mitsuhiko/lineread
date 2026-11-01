//! The init file (inputrc) parser: `set`, key bindings (`"seq": cmd`,
//! `Control-x: cmd`, macros), `$if/$else/$endif/$include`, error messages.
//! Owner: bind/inputrc workstream.
use crate::bind::{generic_bind_raw, translate_keyseq_bytes};
use crate::prelude::*;
use crate::util::ByteBufExt;

/// File/line of the init file being read (for error messages).
struct Ctx {
    file: Vec<u8>,
    line: usize,
}
static CTX: CVar<Vec<Ctx>> = CVar::new(Vec::new());
/// Conditional stack: (this branch active, some branch taken, parent active)
static IFSTACK: CVar<Vec<(bool, bool, bool)>> = CVar::new(Vec::new());
/// Last init file read (rl_re_read_init_file).
static LAST_FILE: CVar<Option<Vec<u8>>> = CVar::new(None);

/// Print a parser/variable error to stderr: "readline: FILE: line N: MSG".
pub fn report_error(msg: &[u8]) {
    let mut m = b"readline: ".to_vec();
    if let Some(c) = CTX.get_mut().last() {
        m.extend_from_slice(&c.file);
        m.push_str(": line ");
        m.push_int(c.line as i64);
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

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    i
}

fn active() -> bool {
    IFSTACK.get_mut().last().map(|e| e.0 && e.2).unwrap_or(true)
}

/// `$if` test.
fn eval_cond(arg: &[u8]) -> bool {
    let arg = trim(arg);
    if let Some(rest) = strip_prefix_ci(arg, b"mode=") {
        let m = if crate::readline::rl_editing_mode.get() == crate::readline::EMACS_MODE { &b"emacs"[..] } else { b"vi" };
        return rest.eq_ignore_ascii_case(m);
    }
    if let Some(rest) = strip_prefix_ci(arg, b"term=") {
        let t = unsafe { cstr(crate::terminal::rl_terminal_name.get()).to_vec() };
        let short: &[u8] = t.split(|&c| c == b'-').next().unwrap_or(&t);
        return rest.eq_ignore_ascii_case(&t) || rest.eq_ignore_ascii_case(short);
    }
    if let Some(rest) = strip_prefix_ci(arg, b"version") {
        return version_cond(rest);
    }
    // variable comparison: NAME == VALUE
    if let Some(p) = find_sub(arg, b"==") {
        let name = trim(&arg[..p]);
        let val = trim(&arg[p + 2..]);
        return crate::vars::variable_value(name).map(|v| v.eq_ignore_ascii_case(val)).unwrap_or(false);
    }
    if let Some(p) = find_sub(arg, b"!=") {
        let name = trim(&arg[..p]);
        let val = trim(&arg[p + 2..]);
        return crate::vars::variable_value(name).map(|v| !v.eq_ignore_ascii_case(val)).unwrap_or(false);
    }
    // application name
    let word: &[u8] = arg.split(|&c| c == b' ' || c == b'\t').next().unwrap_or(arg);
    unsafe { word.eq_ignore_ascii_case(cstr(crate::readline::rl_readline_name.get())) }
}

fn version_cond(rest: &[u8]) -> bool {
    let r = trim(rest);
    let (op, n) = if r.starts_with(b">=") || r.starts_with(b"<=") || r.starts_with(b"==") || r.starts_with(b"!=") {
        (&r[..2], &r[2..])
    } else if r.starts_with(b">") || r.starts_with(b"<") || r.starts_with(b"=") {
        (&r[..1], &r[1..])
    } else {
        return false;
    };
    let n = trim(n);
    let mut parts = n.split(|&c| c == b'.');
    let major = parts.next().map(crate::vars::atoi).unwrap_or(0);
    let minor = parts.next().map(crate::vars::atoi).unwrap_or(0);
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

fn trim(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut b = s.len();
    while a < b && (s[a] == b' ' || s[a] == b'\t') {
        a += 1;
    }
    while b > a && (s[b - 1] == b' ' || s[b - 1] == b'\t' || s[b - 1] == b'\r') {
        b -= 1;
    }
    &s[a..b]
}

fn strip_prefix_ci<'a>(s: &'a [u8], p: &[u8]) -> Option<&'a [u8]> {
    if s.len() >= p.len() && s[..p.len()].eq_ignore_ascii_case(p) {
        Some(&s[p.len()..])
    } else {
        None
    }
}

fn find_sub(s: &[u8], pat: &[u8]) -> Option<usize> {
    s.windows(pat.len()).position(|w| w == pat)
}

/// Handle a `$` directive line.
fn directive(line: &[u8]) -> c_int {
    let body = &line[1..];
    let end = body.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(body.len());
    let word = &body[..end];
    let arg = trim(&body[end..]);
    let st = IFSTACK.get_mut();
    if word.eq_ignore_ascii_case(b"if") {
        let parent = st.last().map(|e| e.0 && e.2).unwrap_or(true);
        let v = parent && eval_cond(arg);
        IFSTACK.get_mut().push((v, v, parent));
        return 0;
    }
    if word.eq_ignore_ascii_case(b"else") {
        match st.last_mut() {
            Some(e) => {
                e.0 = !e.1;
                e.1 = true;
            }
            None => {
                report_error(b"$else found without matching $if");
                return 1;
            }
        }
        return 0;
    }
    if word.eq_ignore_ascii_case(b"endif") {
        if st.pop().is_none() {
            report_error(b"$endif without matching $if");
            return 1;
        }
        return 0;
    }
    if !active() {
        return 0;
    }
    if word.eq_ignore_ascii_case(b"include") {
        let path = expand_tilde(arg);
        let saved = core::mem::take(IFSTACK.get_mut());
        read_file(&path, false);
        *IFSTACK.get_mut() = saved;
        return 0;
    }
    let mut m = b"unknown parser directive `".to_vec();
    m.extend_from_slice(line);
    m.push(b'\'');
    report_error(&m);
    1
}

fn expand_tilde(p: &[u8]) -> Vec<u8> {
    unsafe {
        let c = c_strdup(p);
        let e = crate::tilde::tilde_expand(c);
        cfree(c);
        if e.is_null() {
            return p.to_vec();
        }
        let v = cstr(e).to_vec();
        cfree(e);
        v
    }
}

/// Parse a quoted string starting at `s[i]` (the quote); returns the raw
/// contents (escapes kept) and the index after the closing quote.
fn quoted(s: &[u8], i: usize) -> (Vec<u8>, usize) {
    let q = s[i];
    let mut j = i + 1;
    let mut out = Vec::new();
    while j < s.len() && s[j] != q {
        if s[j] == b'\\' && j + 1 < s.len() {
            out.push(s[j]);
            out.push(s[j + 1]);
            j += 2;
            continue;
        }
        out.push(s[j]);
        j += 1;
    }
    (out, (j + 1).min(s.len()))
}

/// A symbolic key name (`Control-a`, `Meta-Rubout`, `TAB`, `x`) to bytes.
fn keyname_to_seq(name: &[u8]) -> Vec<u8> {
    let mut notation = Vec::new();
    let mut rest = name;
    loop {
        if let Some(r) = strip_prefix_ci(rest, b"control-").or_else(|| strip_prefix_ci(rest, b"c-")) {
            notation.extend_from_slice(b"\\C-");
            rest = r;
            continue;
        }
        if let Some(r) = strip_prefix_ci(rest, b"meta-").or_else(|| strip_prefix_ci(rest, b"m-")) {
            notation.extend_from_slice(b"\\M-");
            rest = r;
            continue;
        }
        break;
    }
    let named: &[(&[u8], &[u8])] = &[
        (b"rubout", b"\\d"),
        (b"del", b"\\d"),
        (b"escape", b"\\e"),
        (b"esc", b"\\e"),
        (b"lfd", b"\\n"),
        (b"newline", b"\\n"),
        (b"ret", b"\\r"),
        (b"return", b"\\r"),
        (b"space", b" "),
        (b"spc", b" "),
        (b"tab", b"\\t"),
    ];
    let mut key: Vec<u8> = rest.to_vec();
    for (n, v) in named {
        if rest.eq_ignore_ascii_case(n) {
            key = v.to_vec();
            break;
        }
    }
    if key.len() > 1 && !key.starts_with(b"\\") {
        key.truncate(1);
    }
    notation.extend_from_slice(&key);
    translate_keyseq_bytes(&notation).0
}

/// Parse and execute one init-file line.  Returns 0 on success.
pub fn parse_line(line: &[u8]) -> c_int {
    let i = skip_ws(line, 0);
    let line = &line[i..];
    let line = if line.ends_with(b"\r") { &line[..line.len() - 1] } else { line };
    if line.is_empty() || line[0] == b'#' {
        return 0;
    }
    if line[0] == b'$' {
        return directive(line);
    }
    if !active() {
        return 0;
    }
    if line.len() > 3 && line[..3].eq_ignore_ascii_case(b"set") && (line[3] == b' ' || line[3] == b'\t') {
        let j = skip_ws(line, 3);
        let rest = &line[j..];
        let ne = rest.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(rest.len());
        let name = &rest[..ne];
        let mut val = trim(&rest[ne..]);
        // boolean values end at whitespace
        if crate::vars::variable_value(name).map(|v| v == b"on" || v == b"off").unwrap_or(false) {
            let e = val.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(val.len());
            val = &val[..e];
        }
        if !crate::vars::is_variable(name) {
            let mut m = name.to_vec();
            m.push_str(": unknown variable name");
            report_error(&m);
            return 1;
        }
        return crate::vars::variable_bind(name, Some(val));
    }
    // key binding
    let (keys, mut j) = if line[0] == b'"' || line[0] == b'\'' {
        let (raw, j) = quoted(line, 0);
        (translate_keyseq_bytes(&raw).0, j)
    } else {
        let mut j = 0;
        while j < line.len() && line[j] != b':' && line[j] != b' ' && line[j] != b'\t' {
            j += 1;
        }
        (keyname_to_seq(&line[..j]), j)
    };
    j = skip_ws(line, j);
    if j >= line.len() || line[j] != b':' {
        let w: &[u8] = line.split(|&c| c == b' ' || c == b'\t').next().unwrap_or(line);
        let mut m = w.to_vec();
        m.push_str(": no key sequence terminator");
        report_error(&m);
        return 1;
    }
    j = skip_ws(line, j + 1);
    let value = &line[j..];
    let map = crate::keymap::current();
    unsafe {
        if !value.is_empty() && (value[0] == b'"' || value[0] == b'\'') {
            let (raw, _) = quoted(value, 0);
            let m = translate_keyseq_bytes(&raw).0;
            generic_bind_raw(ISMACR, &keys, KmData { macro_: c_strdup(&m) }, map);
            return 0;
        }
        let e = value.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(value.len());
        let fname = &value[..e];
        let f = crate::funmap::named_function(fname);
        if let Some(f) = f {
            generic_bind_raw(ISFUNC, &keys, KmData { func: Some(f) }, map);
        } else if fname.eq_ignore_ascii_case(b"") {
        } else {
            // unknown function names bind nothing
        }
    }
    0
}

/// Read an init file.  Returns 0 or an errno value.
pub fn read_file(path: &[u8], top: bool) -> c_int {
    use std::os::unix::ffi::OsStrExt;
    let Ok(data) = std::fs::read(std::ffi::OsStr::from_bytes(path)) else {
        return std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::ENOENT);
    };
    if top {
        LAST_FILE.set(Some(path.to_vec()));
        IFSTACK.get_mut().clear();
    }
    CTX.get_mut().push(Ctx { file: path.to_vec(), line: 0 });
    for (n, l) in data.split(|&c| c == b'\n').enumerate() {
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
    unsafe {
        if let Some(p) = crate::util::getenv_bytes(c"INPUTRC") {
            if !p.is_empty() {
                let p = expand_tilde(p);
                read_file(&p, true);
                return;
            }
        }
        let home = expand_tilde(b"~/.inputrc");
        if read_file(&home, true) != 0 {
            read_file(b"/etc/inputrc", true);
        }
    }
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
                    _ => expand_tilde(b"~/.inputrc"),
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
