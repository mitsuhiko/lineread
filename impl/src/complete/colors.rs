//! Colored completion listings: LS_COLORS parsing (the `ls` format:
//! two-letter indicators and `*ext` entries with C-like escapes) and the
//! color sequences printed around names (colored-stats,
//! colored-completion-prefix).
//! Owner: display & completion workstream.
use super::*;

/// Indicator indices.
const LC: usize = 0; // left
const RC: usize = 1; // right
const EC: usize = 2; // end
const RS: usize = 3; // reset
const NO: usize = 4; // normal
const FI: usize = 5;
const DI: usize = 6;
const LN: usize = 7;
const PI: usize = 8;
const SO: usize = 9;
const BD: usize = 10;
const CD: usize = 11;
const MI: usize = 12;
const OR: usize = 13;
const EX: usize = 14;
const DO: usize = 15;
const SU: usize = 16;
const SG: usize = 17;
const ST: usize = 18;
const OW: usize = 19;
const TW: usize = 20;
const CA: usize = 21;
const MH: usize = 22;
const CL: usize = 23;
const NIND: usize = 24;

const NAMES: [&[u8; 2]; NIND] = [
    b"lc", b"rc", b"ec", b"rs", b"no", b"fi", b"di", b"ln", b"pi", b"so", b"bd", b"cd", b"mi", b"or", b"ex",
    b"do", b"su", b"sg", b"st", b"ow", b"tw", b"ca", b"mh", b"cl",
];

fn defaults() -> Vec<Option<Vec<u8>>> {
    let d: [Option<&[u8]>; NIND] = [
        Some(b"\x1b["),
        Some(b"m"),
        None,
        Some(b"0"),
        None,
        None,
        Some(b"01;34"),
        Some(b"01;36"),
        Some(b"33"),
        Some(b"01;35"),
        Some(b"01;33"),
        Some(b"01;33"),
        None,
        None,
        Some(b"01;32"),
        Some(b"01;35"),
        Some(b"37;41"),
        Some(b"30;43"),
        Some(b"37;44"),
        Some(b"34;42"),
        Some(b"30;42"),
        Some(b"30;41"),
        None,
        Some(b"\x1b[K"),
    ];
    d.iter().map(|o| o.map(|s| s.to_vec())).collect()
}

struct Colors {
    ind: Vec<Option<Vec<u8>>>,
    /// (extension, sequence), most recent first
    ext: Vec<(Vec<u8>, Vec<u8>)>,
    /// custom prefix color (`*readline-colored-completion-prefix`)
    prefix: Option<Vec<u8>>,
    loaded: bool,
}

static COLORS: CVar<Colors> = CVar::new(Colors { ind: Vec::new(), ext: Vec::new(), prefix: None, loaded: false });

fn c() -> &'static mut Colors {
    let c = COLORS.get_mut();
    if !c.loaded {
        c.ind = defaults();
        c.loaded = true;
    }
    c
}

pub fn enabled() -> bool {
    true
}

fn errmsg(msg: &[u8]) {
    let mut v = b"readline: ".to_vec();
    v.extend_from_slice(msg);
    v.push(b'\n');
    crate::terminal::flush();
    unsafe {
        let e = crate::util::lr_stderr();
        crate::util::fwrite_bytes(e, &v);
        crate::util::fflush(e);
    }
}

/// Parse one escaped string from `src[*p..]` into `buf[*q..]`.  Stops at
/// `:` / NUL (and `=` when `equals_end`).  Returns false on a syntax error.
fn funky_string(buf: &mut [u8], q: &mut usize, src: &[u8], p: &mut usize, equals_end: bool) -> bool {
    #[derive(PartialEq)]
    enum S {
        Gnd,
        Backslash,
        Octal,
        Hex,
        Caret,
        End,
        Error,
    }
    let at = |i: usize| -> u8 { src.get(i).copied().unwrap_or(0) };
    let mut st = S::Gnd;
    let mut num: u32 = 0;
    let put = |buf: &mut [u8], q: &mut usize, b: u8| {
        if *q < buf.len() {
            buf[*q] = b;
        }
        *q += 1;
    };
    while st != S::End && st != S::Error {
        match st {
            S::Gnd => match at(*p) {
                b':' | 0 => st = S::End,
                b'\\' => {
                    st = S::Backslash;
                    *p += 1;
                }
                b'^' => {
                    st = S::Caret;
                    *p += 1;
                }
                b'=' if equals_end => st = S::End,
                c => {
                    put(buf, q, c);
                    *p += 1;
                }
            },
            S::Backslash => {
                let c = at(*p);
                match c {
                    b'0'..=b'7' => {
                        st = S::Octal;
                        num = (c - b'0') as u32;
                    }
                    b'x' | b'X' => {
                        st = S::Hex;
                        num = 0;
                    }
                    b'a' => num = 7,
                    b'b' => num = 8,
                    b'e' => num = 27,
                    b'f' => num = 12,
                    b'n' => num = 10,
                    b'r' => num = 13,
                    b't' => num = 9,
                    b'v' => num = 11,
                    b'?' => num = 127,
                    b'_' => num = b' ' as u32,
                    0 => st = S::Error,
                    _ => num = c as u32,
                }
                if st == S::Backslash {
                    put(buf, q, num as u8);
                    st = S::Gnd;
                }
                if st != S::Error {
                    *p += 1;
                }
            }
            S::Octal => {
                let c = at(*p);
                if !(b'0'..=b'7').contains(&c) {
                    put(buf, q, num as u8);
                    st = S::Gnd;
                } else {
                    num = (num << 3) + (c - b'0') as u32;
                    *p += 1;
                }
            }
            S::Hex => {
                let c = at(*p);
                let d = match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                };
                match d {
                    Some(d) => {
                        num = (num << 4) + d as u32;
                        *p += 1;
                    }
                    None => {
                        put(buf, q, num as u8);
                        st = S::Gnd;
                    }
                }
            }
            S::Caret => {
                st = S::Gnd;
                let c = at(*p);
                if (b'@'..=b'~').contains(&c) {
                    put(buf, q, c & 0x1f);
                    *p += 1;
                } else if c == b'?' {
                    put(buf, q, 127);
                    *p += 1;
                } else {
                    st = S::Error;
                }
            }
            S::End | S::Error => {}
        }
    }
    st != S::Error
}

/// Parse LS_COLORS (starting from the defaults).  On a parse error,
/// colored-stats and colored-completion-prefix are turned off.
pub fn parse() {
    let col = c();
    col.ind = defaults();
    col.ext.clear();
    col.prefix = None;
    let Some(env) = (unsafe { crate::util::getenv_bytes(c"LS_COLORS") }) else { return };
    if env.is_empty() {
        return;
    }
    let src = env.to_vec();
    let mut buf = src.clone();
    buf.push(0);
    let mut p = 0usize;
    let mut q = 0usize;
    let at = |i: usize| -> u8 { src.get(i).copied().unwrap_or(0) };
    let mut ok = true;
    loop {
        match at(p) {
            b':' => {
                p += 1;
            }
            0 => break,
            b'*' => {
                p += 1;
                let es = q;
                if !funky_string(&mut buf, &mut q, &src, &mut p, true) {
                    ok = false;
                    break;
                }
                let ext = buf[es..q.min(buf.len())].to_vec();
                if at(p) == b'=' {
                    p += 1;
                    let ss = q;
                    if !funky_string(&mut buf, &mut q, &src, &mut p, false) {
                        ok = false;
                        break;
                    }
                    let seq = buf[ss..q.min(buf.len())].to_vec();
                    col.ext.insert(0, (ext, seq));
                } else {
                    // the message shows the (unterminated) parse buffer
                    let tail = &buf[es..];
                    let n = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
                    let mut m = b"LS_COLORS: syntax error: ".to_vec();
                    m.extend_from_slice(&tail[..n]);
                    errmsg(&m);
                    ok = false;
                    break;
                }
            }
            c0 => {
                let c1 = at(p + 1);
                if c1 == 0 {
                    ok = false;
                    break;
                }
                let label = [c0, c1];
                p += 2;
                if at(p) != b'=' {
                    ok = false;
                    break;
                }
                p += 1;
                let mut good = false;
                if let Some(idx) = NAMES.iter().position(|n| **n == label) {
                    let ss = q;
                    if funky_string(&mut buf, &mut q, &src, &mut p, false) {
                        col.ind[idx] = Some(buf[ss..q.min(buf.len())].to_vec());
                        good = true;
                    }
                }
                if !good {
                    let mut m = b"LS_COLORS: unrecognized prefix: ".to_vec();
                    m.extend_from_slice(&label);
                    errmsg(&m);
                    while at(p) != 0 && at(p) != b':' {
                        p += 1;
                    }
                    if at(p) == 0 {
                        break;
                    }
                }
            }
        }
    }
    if !ok {
        errmsg(b"unparsable value for LS_COLORS environment variable");
        col.ind = defaults();
        col.ext.clear();
        crate::vars::colored_stats.set(0);
        crate::vars::colored_completion_prefix.set(0);
        return;
    }
    // the custom prefix color
    if let Some(i) = col.ext.iter().position(|(e, _)| e == b"readline-colored-completion-prefix") {
        col.prefix = Some(col.ext[i].1.clone());
    }
}

/// At initialisation: parse LS_COLORS when a color option is on.
pub fn init() {
    if crate::vars::colored_stats.get() != 0 || crate::vars::colored_completion_prefix.get() != 0 {
        parse();
    }
}

pub fn reparse() {
    parse();
}

fn ind(i: usize) -> Option<&'static [u8]> {
    c().ind[i].as_deref()
}

fn is_colored(i: usize) -> bool {
    match ind(i) {
        None => false,
        Some(s) => !(s.is_empty() || s == b"0" || s == b"00"),
    }
}

fn put(out: &mut Vec<u8>, i: usize) {
    if let Some(s) = ind(i) {
        out.extend_from_slice(s);
    }
}

fn put_seq(out: &mut Vec<u8>, s: &[u8]) {
    if is_colored(NO) {
        // reset so attributes do not combine
        put(out, LC);
        put(out, RC);
    }
    put(out, LC);
    out.extend_from_slice(s);
    put(out, RC);
}

pub fn set_normal_color(out: &mut Vec<u8>) {
    if is_colored(NO) {
        put(out, LC);
        put(out, NO);
        put(out, RC);
    }
}

pub fn prep_non_filename_text(out: &mut Vec<u8>) {
    if ind(EC).is_some() {
        put(out, EC);
    } else {
        put(out, LC);
        put(out, RS);
        put(out, RC);
    }
    put(out, CL);
}

pub fn print_prefix_color(out: &mut Vec<u8>) {
    let s: Option<Vec<u8>> = match &c().prefix {
        Some(p) => Some(p.clone()),
        None => ind(SO).map(|s| s.to_vec()),
    };
    if let Some(s) = s {
        put_seq(out, &s);
    }
}

/// Print the color start sequence for the file `name`.
pub fn print_color_indicator(out: &mut Vec<u8>, name: &[u8]) {
    let path = filename::tilde_expand_bytes(name);
    let cp = unsafe { c_strdup(&path) };
    let mut st: libc::stat = unsafe { core::mem::zeroed() };
    let r = unsafe { libc::lstat(cp, &mut st) };
    let mut tst: libc::stat = unsafe { core::mem::zeroed() };
    let linkok = r == 0 && unsafe { libc::stat(cp, &mut tst) } == 0;
    unsafe { cfree(cp) };
    let mut mode = st.st_mode;
    let mut nlink = st.st_nlink;
    let typ;
    if r != 0 {
        typ = MI;
    } else {
        let is_link = (mode & libc::S_IFMT) == libc::S_IFLNK;
        if is_link && ind(LN) == Some(b"target") && linkok {
            mode = tst.st_mode;
            nlink = tst.st_nlink;
        }
        let fmt = mode & libc::S_IFMT;
        typ = match fmt {
            libc::S_IFREG => {
                if mode & libc::S_ISUID != 0 && is_colored(SU) {
                    SU
                } else if mode & libc::S_ISGID != 0 && is_colored(SG) {
                    SG
                } else if mode & 0o111 != 0 && is_colored(EX) {
                    EX
                } else if nlink > 1 && is_colored(MH) {
                    MH
                } else {
                    FI
                }
            }
            libc::S_IFDIR => {
                let sticky = mode & libc::S_ISVTX != 0;
                let ow = mode & 0o002 != 0;
                if sticky && ow && is_colored(TW) {
                    TW
                } else if ow && is_colored(OW) {
                    OW
                } else if sticky && is_colored(ST) {
                    ST
                } else {
                    DI
                }
            }
            libc::S_IFLNK => {
                if !linkok && is_colored(OR) {
                    OR
                } else {
                    LN
                }
            }
            libc::S_IFIFO => PI,
            libc::S_IFSOCK => SO,
            libc::S_IFBLK => BD,
            libc::S_IFCHR => CD,
            _ => OR,
        };
    }
    let _ = CA;
    let _ = DO;
    let mut seq: Option<Vec<u8>> = None;
    if typ == FI {
        for (e, s) in &c().ext {
            if name.len() >= e.len() && &name[name.len() - e.len()..] == &e[..] {
                seq = Some(s.clone());
                break;
            }
        }
    }
    let s = match seq {
        Some(s) => Some(s),
        None => ind(typ).map(|s| s.to_vec()),
    };
    if let Some(s) = s {
        put_seq(out, &s);
    }
}
