//! Terminal capabilities (terminfo via the `terminfo` crate), screen size,
//! output primitives (everything goes through `rl_outstream`), the bell.
//! Owner: display workstream.
use crate::prelude::*;
use crate::util::{fflush, fwrite_bytes};
use std::collections::HashMap;

#[no_mangle]
pub static rl_terminal_name: CVar<*const c_char> = CVar::new(null());
#[no_mangle]
pub static rl_prefer_env_winsize: CVar<c_int> = CVar::new(0);

/// Loaded terminal description.
pub struct Term {
    pub name: Vec<u8>,
    /// terminfo string capabilities (padding stripped), by terminfo name
    pub strings: HashMap<&'static str, Vec<u8>>,
    /// the same with padding specifications (rl_get_termcap)
    pub raw: HashMap<&'static str, Vec<u8>>,
    pub auto_wrap: bool,
    pub eat_newline_glitch: bool,
    pub has_meta: bool,
    pub dumb: bool,
    pub ti_cols: i32,
    pub ti_lines: i32,
}

pub static TERM: CVar<Option<Term>> = CVar::new(None);
/// Screen size: physical columns (as found), rows, and the width readline
/// uses (one less than the columns unless the terminal has `am`+`xenl`;
/// see `usable_width`).  0 = not determined yet.
pub static SCREEN_COLS: CVar<c_int> = CVar::new(0);
pub static SCREEN_ROWS: CVar<c_int> = CVar::new(0);
static SCREEN_WIDTH: CVar<c_int> = CVar::new(0);
/// Auto-margins usable (`am` and `xenl`); None before the terminal is
/// initialised.
static AUTOWRAP: CVar<Option<bool>> = CVar::new(None);
/// Output buffer, flushed by `flush()`.
static OUTBUF: CVar<Vec<u8>> = CVar::new(Vec::new());

/// terminfo names we load
const CAPS: &[&str] = &[
    "cr", "clear", "el", "el1", "ed", "cuu1", "cuu", "cud1", "cud", "cuf1", "cuf", "cub1", "cub",
    "hpa", "ich", "ich1", "smir", "rmir", "dch", "dch1", "bel", "flash", "smkx", "rmkx", "smm",
    "rmm", "kcuu1", "kcud1", "kcuf1", "kcub1", "khome", "kend", "kdch1", "kich1", "kll", "pad",
    "smso", "rmso", "bold", "sgr0", "civis", "cnorm", "cvvis", "E3", "ind", "home", "cup", "smul",
    "rmul", "rev",
];

/// termcap name -> terminfo name (rl_get_termcap)
const TERMCAP: &[(&str, &str)] = &[
    ("@7", "kend"), ("DC", "dch"), ("E3", "E3"), ("IC", "ich"), ("ce", "el"), ("cl", "clear"),
    ("cr", "cr"), ("dc", "dch1"), ("ei", "rmir"), ("ic", "ich1"), ("im", "smir"), ("kD", "kdch1"),
    ("kH", "kll"), ("kI", "kich1"), ("kd", "kcud1"), ("ke", "rmkx"), ("kh", "khome"),
    ("kl", "kcub1"), ("kr", "kcuf1"), ("ks", "smkx"), ("ku", "kcuu1"), ("le", "cub1"),
    ("mm", "smm"), ("mo", "rmm"), ("nd", "cuf1"), ("pc", "pad"), ("se", "rmso"), ("so", "smso"),
    ("up", "cuu1"), ("vb", "flash"), ("ve", "cnorm"), ("vi", "civis"), ("vs", "cvvis"),
    ("md", "bold"), ("me", "sgr0"), ("cd", "ed"), ("ch", "hpa"), ("bl", "bel"), ("do", "cud1"),
];

/// Remove terminfo padding specifications (`$<5>`, `$<2*/>`).
fn strip_padding(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'$' && s.get(i + 1) == Some(&b'<') {
            if let Some(j) = s[i..].iter().position(|&c| c == b'>') {
                i += j + 1;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

fn load(name: &[u8]) -> Term {
    let sname = String::from_utf8_lossy(name).into_owned();
    let mut t = Term {
        name: name.to_vec(),
        strings: HashMap::new(),
        raw: HashMap::new(),
        auto_wrap: false,
        eat_newline_glitch: false,
        has_meta: false,
        dumb: true,
        ti_cols: 0,
        ti_lines: 0,
    };
    if sname.is_empty() {
        return t;
    }
    let Ok(db) = terminfo::Database::from_name(&sname) else { return t };
    for &c in CAPS {
        if let Some(terminfo::Value::String(v)) = db.raw(c) {
            t.strings.insert(c, strip_padding(v));
            t.raw.insert(c, v.to_vec());
        }
    }
    let flag = |n: &str| matches!(db.raw(n), Some(terminfo::Value::True));
    let num = |n: &str| match db.raw(n) {
        Some(terminfo::Value::Number(v)) => *v,
        _ => 0,
    };
    t.auto_wrap = flag("am");
    t.eat_newline_glitch = flag("xenl");
    t.has_meta = flag("km");
    t.ti_cols = num("cols");
    t.ti_lines = num("lines");
    // a terminal we cannot move the cursor up on is "dumb"
    t.dumb = !t.strings.contains_key("cuu1") && !t.strings.contains_key("cuu");
    t
}

pub fn term() -> &'static Term {
    let t = TERM.get_mut();
    if t.is_none() {
        let name = unsafe { crate::util::getenv_bytes(c"TERM").unwrap_or(b"dumb").to_vec() };
        *t = Some(load(&name));
    }
    t.as_ref().unwrap()
}

/// A capability string (None if the terminal lacks it).
pub fn cap(name: &str) -> Option<&'static [u8]> {
    term().strings.get(name).map(|v| &v[..])
}

/// Expand a parameterised capability.
pub fn cap_param(name: &str, p: i32) -> Option<Vec<u8>> {
    let s = cap(name)?;
    terminfo::expand!(s; p).ok()
}

/// Initialise for terminal `name` (NULL/empty: $TERM).
pub unsafe fn init_terminal(name: Option<&[u8]>) {
    let n: Vec<u8> = match name {
        Some(n) if !n.is_empty() => n.to_vec(),
        _ => crate::util::getenv_bytes(c"TERM").unwrap_or(b"dumb").to_vec(),
    };
    let n = if n.is_empty() { b"dumb".to_vec() } else { n };
    // a custom redisplay function gets no terminal capabilities
    let custom = !matches!(crate::display::rl_redisplay_function.get(), Some(f) if f as usize == crate::display::rl_redisplay as *const () as usize);
    let t = if custom { load(b"") } else { load(&n) };
    let known = !t.strings.is_empty();
    let autowrap = t.auto_wrap && t.eat_newline_glitch;
    let ansi = is_ansi(&n, &t);
    TERM.set(Some(t));
    line_setup();
    if rl_terminal_name.get().is_null() {
        set_terminal_name(&n);
    }
    AUTOWRAP.set(Some(autowrap));
    if SCREEN_WIDTH.get() <= 0 || SCREEN_ROWS.get() <= 0 {
        get_screen_size(false);
    } else if !known && !autowrap {
        // a preset size on a terminal without capabilities
        SCREEN_WIDTH.set((SCREEN_WIDTH.get() - 1).max(1));
    }
    if !ansi {
        crate::vars::enable_bracketed_paste.set(0);
        crate::vars::enable_active_region.set(0);
    }
}

/// Does the terminal understand ANSI sequences (bracketed paste, region
/// highlighting)?  Known names, else the capabilities tell.
fn is_ansi(name: &[u8], t: &Term) -> bool {
    const NAMES: &[&[u8]] = &[
        b"xterm", b"rxvt", b"eterm", b"screen", b"tmux", b"vt100", b"vt102", b"vt220", b"vt320", b"ansi",
        b"scoansi", b"cygwin", b"linux", b"konsole", b"bvterm",
    ];
    if t.strings.is_empty() {
        return false;
    }
    if NAMES.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    let csi = |c: &str| t.strings.get(c).map(|v| v.starts_with(b"\x1b[")).unwrap_or(false);
    csi("el")
}

/// Storage for rl_terminal_name when the library sets it.
static TERM_NAME: CVar<Vec<u8>> = CVar::new(Vec::new());

fn set_terminal_name(n: &[u8]) {
    let mut v = n.to_vec();
    v.push(0);
    *TERM_NAME.get_mut() = v;
    rl_terminal_name.set(TERM_NAME.get_mut().as_ptr() as *const c_char);
}

/// Determine the screen size: tty (TIOCGWINSZ), then COLUMNS/LINES, then
/// terminfo, then 80x24.
pub unsafe fn get_screen_size(ignore_env: bool) {
    let mut cols = 0;
    let mut rows = 0;
    let fd = {
        let s = crate::readline::rl_instream.get();
        if s.is_null() {
            0
        } else {
            libc::fileno(s)
        }
    };
    let mut ws: libc::winsize = core::mem::zeroed();
    if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 {
        cols = ws.ws_col as c_int;
        rows = ws.ws_row as c_int;
    }
    if !ignore_env && (rl_prefer_env_winsize.get() != 0 || cols <= 0) {
        if let Some(c) = crate::util::getenv_bytes(c"COLUMNS") {
            let v = crate::vars::atoi(c);
            if v > 0 {
                cols = v;
            }
        }
    }
    if !ignore_env && (rl_prefer_env_winsize.get() != 0 || rows <= 0) {
        if let Some(c) = crate::util::getenv_bytes(c"LINES") {
            let v = crate::vars::atoi(c);
            if v > 0 {
                rows = v;
            }
        }
    }
    let t = term();
    if cols <= 0 {
        cols = t.ti_cols;
    }
    if rows <= 0 {
        rows = t.ti_lines;
    }
    if cols <= 1 {
        cols = 80;
    }
    if rows <= 0 {
        rows = 24;
    }
    set_size(rows, cols);
    export_size();
}

fn set_size(rows: c_int, cols: c_int) {
    SCREEN_ROWS.set(rows);
    SCREEN_COLS.set(cols);
    let w = if AUTOWRAP.get() == Some(false) { cols - 1 } else { cols };
    SCREEN_WIDTH.set(w.max(1));
}

/// Export LINES/COLUMNS (rl_change_environment).
fn export_size() {
    if crate::signals::rl_change_environment.get() == 0 {
        return;
    }
    unsafe {
        let l = format!("{}\0", SCREEN_ROWS.get());
        let c = format!("{}\0", SCREEN_COLS.get());
        libc::setenv(c"LINES".as_ptr(), l.as_ptr() as *const c_char, 1);
        libc::setenv(c"COLUMNS".as_ptr(), c.as_ptr() as *const c_char, 1);
    }
}

/// Columns usable for text: one less when the terminal cannot write the
/// last column without wrapping (no `am`+`xenl`).
pub fn usable_width() -> c_int {
    if SCREEN_WIDTH.get() <= 0 {
        unsafe { get_screen_size(false) };
    }
    SCREEN_WIDTH.get().max(1)
}

/// Width of the horizontally scrolled line (the last column is not used).
pub fn hscroll_width() -> c_int {
    usable_width()
}

/// After the init file was read: horizontal scrolling on a terminal with
/// auto-margins does not use the last column.
pub fn init_file_read() {
    if crate::vars::horizontal_scroll_mode.get() != 0 && AUTOWRAP.get() == Some(true) {
        SCREEN_WIDTH.set((SCREEN_WIDTH.get() - 1).max(1));
    }
}

/// Does the terminal wrap automatically (`am`)?
pub fn has_auto_margins() -> bool {
    term().auto_wrap
}

// ---- output

pub fn out(b: &[u8]) {
    OUTBUF.get_mut().extend_from_slice(b);
}
pub fn out_byte(b: u8) {
    OUTBUF.get_mut().push(b);
}
pub fn out_cap(name: &str) -> bool {
    match cap(name) {
        Some(s) => {
            out(s);
            true
        }
        None => false,
    }
}
/// The stream terminal output goes to: `rl_outstream` as it was when
/// readline last started a line or initialised (applications may point
/// `rl_outstream` elsewhere temporarily, e.g. to capture a listing).
static OUT_STREAM: CVar<*mut libc::FILE> = CVar::new(null_mut());

/// Remember `rl_outstream` as the terminal stream.
pub fn line_setup() {
    OUT_STREAM.set(crate::readline::outstream());
}

pub fn term_stream() -> *mut libc::FILE {
    let o = OUT_STREAM.get();
    if o.is_null() {
        crate::readline::outstream()
    } else {
        o
    }
}

/// Screen width as used by readline (one less without auto-margins).
pub fn screen_width() -> c_int {
    usable_width()
}

pub fn screen_height() -> c_int {
    if SCREEN_ROWS.get() <= 0 {
        unsafe { get_screen_size(false) };
    }
    SCREEN_ROWS.get()
}

pub fn flush() {
    unsafe {
        let b = OUTBUF.get_mut();
        if !b.is_empty() {
            let o = term_stream();
            fwrite_bytes(o, b);
            b.clear();
            fflush(o);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_ding() -> c_int {
    // (rings only once the terminal was prepped with echoing, else -1)
    if crate::tty::ECHOING.get() != 0 {
        match crate::vars::bell_style.get() {
            crate::vars::BELL_NONE => {}
            crate::vars::BELL_VISIBLE => {
                if !out_cap("flash") {
                    out_byte(7);
                }
                flush();
            }
            _ => {
                out_byte(7);
                flush();
            }
        }
        return 0;
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rl_crlf() -> c_int {
    out_byte(b'\n');
    flush();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_reset_terminal(name: *const c_char) -> c_int {
    // rl_terminal_name keeps its value (observed)
    let old = cstr_opt(rl_terminal_name.get()).map(|v| v.to_vec());
    init_terminal(cstr_opt(name));
    if let Some(o) = old {
        set_terminal_name(&o);
    }
    crate::display::screen_changed();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_resize_terminal() {
    get_screen_size(true);
    crate::display::screen_resized();
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_screen_size(rows: c_int, cols: c_int) {
    if rows > 0 {
        SCREEN_ROWS.set(rows);
    }
    if cols > 0 {
        SCREEN_COLS.set(cols);
        let w = if AUTOWRAP.get() == Some(false) { cols - 1 } else { cols };
        SCREEN_WIDTH.set(w.max(1));
    }
    crate::display::screen_changed();
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_screen_size(rows: *mut c_int, cols: *mut c_int) {
    if !rows.is_null() {
        *rows = SCREEN_ROWS.get();
    }
    if !cols.is_null() {
        *cols = usable_width();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_reset_screen_size() {
    get_screen_size(false);
    crate::display::screen_changed();
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_termcap(name: *const c_char) -> *mut c_char {
    static STORE: CVar<Vec<Vec<u8>>> = CVar::new(Vec::new());
    let Some(n) = cstr_opt(name) else { return null_mut() };
    let Some(&(_, ti)) = TERMCAP.iter().find(|(tc, _)| tc.as_bytes() == n) else { return null_mut() };
    if AUTOWRAP.get().is_none() {
        // the terminal is not initialised yet
        return null_mut();
    }
    let Some(v) = term().raw.get(ti) else { return null_mut() };
    let store = STORE.get_mut();
    let mut s = v.to_vec();
    s.push(0);
    if let Some(e) = store.iter_mut().find(|e| **e == s) {
        return e.as_mut_ptr() as *mut c_char;
    }
    store.push(s);
    store.last_mut().unwrap().as_mut_ptr() as *mut c_char
}
