//! Multibyte character handling.
//!
//! Policy: the line buffer is bytes.  Character boundaries and widths follow
//! the C library's current locale (`MB_CUR_MAX`, `mbrtowc`, `wcwidth`), like
//! the reference.  When `MB_CUR_MAX == 1` (or `byte-oriented` is set) every
//! byte is a character.  Invalid/incomplete sequences count as one byte each.
use crate::util::{lr_mb_cur_max, mbrtowc, wcwidth, MbState};
use core::ffi::c_char;

/// True when the locale is multibyte and `byte-oriented` is off.
pub fn multibyte() -> bool {
    unsafe { lr_mb_cur_max() > 1 && crate::vars::rl_byte_oriented.get() == 0 }
}

/// Decode the character starting at `buf[i]`: (wide char, byte length).
/// Invalid or incomplete sequences give `(byte, 1)` with `None` as char.
pub fn decode_at(buf: &[u8], i: usize) -> (Option<u32>, usize) {
    if i >= buf.len() {
        return (None, 0);
    }
    let b = buf[i];
    if b < 0x80 || !multibyte() {
        return (Some(b as u32), 1);
    }
    let mut st = MbState::new();
    let mut wc: libc::wchar_t = 0;
    let n = unsafe {
        mbrtowc(&mut wc, buf.as_ptr().add(i) as *const c_char, buf.len() - i, &mut st)
    };
    match n {
        0 => (Some(0), 1),
        n if n == usize::MAX || n == usize::MAX - 1 => (None, 1),
        n => (Some(wc as u32), n),
    }
}

/// Byte length of the character at `i` (>= 1 inside the buffer).
pub fn char_len_at(buf: &[u8], i: usize) -> usize {
    decode_at(buf, i).1.max(1)
}

/// Index of the next character boundary after `i`.
pub fn next_char(buf: &[u8], i: usize) -> usize {
    if i >= buf.len() {
        return buf.len();
    }
    i + char_len_at(buf, i)
}

/// Index of the character boundary before `i`.
pub fn prev_char(buf: &[u8], i: usize) -> usize {
    if i == 0 {
        return 0;
    }
    if !multibyte() {
        return i - 1;
    }
    // scan forward from the start: robust against invalid sequences
    let mut p = 0;
    let mut last = 0;
    while p < i {
        last = p;
        p = next_char(buf, p);
    }
    last
}

/// Move `count` characters forward from `i` (stops at the end).
pub fn forward_chars(buf: &[u8], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i >= buf.len() {
            break;
        }
        i = next_char(buf, i);
    }
    i
}

/// Move `count` characters backward from `i` (stops at 0).
pub fn backward_chars(buf: &[u8], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i == 0 {
            break;
        }
        i = prev_char(buf, i);
    }
    i
}

/// Snap `i` to the start of the character containing it.
pub fn align(buf: &[u8], i: usize) -> usize {
    if !multibyte() || i == 0 || i >= buf.len() {
        return i.min(buf.len());
    }
    let mut p = 0;
    while p < buf.len() {
        let n = next_char(buf, p);
        if n > i {
            return p;
        }
        p = n;
    }
    buf.len()
}

/// Display width of a wide character (`wcwidth`, -1 for non-printables).
pub fn wc_width(wc: u32) -> i32 {
    unsafe { wcwidth(wc as libc::wchar_t) }
}

/// Does the current LC_CTYPE locale use 8-bit characters (anything but
/// "C"/"POSIX")?  Decides the defaults of input-meta/output-meta/convert-meta.
pub fn locale_is_8bit() -> bool {
    unsafe {
        let p = libc::setlocale(libc::LC_CTYPE, core::ptr::null());
        if p.is_null() {
            return false;
        }
        let s = core::ffi::CStr::from_ptr(p).to_bytes();
        !(s == b"C" || s == b"POSIX")
    }
}

/// Is the character at `i` a word character (alphanumeric)?
pub fn is_word_char(buf: &[u8], i: usize) -> bool {
    let (wc, n) = decode_at(buf, i);
    match wc {
        Some(c) if n == 1 => {
            let b = c as u8;
            // bytes >= 0x80 are not word characters (glibc behaviour,
            // also on macOS: see docs/IMPLEMENTING.md)
            b.is_ascii_alphanumeric()
        }
        Some(c) => unsafe { crate::util::iswalnum(c) != 0 },
        None => false,
    }
}

// ---- character movement that treats zero-width characters (combining
// marks, joiners) as part of the preceding character

fn zero_width_at(buf: &[u8], i: usize) -> Option<usize> {
    if i >= buf.len() {
        return None;
    }
    match decode_at(buf, i) {
        (Some(c), n) if c != 0 && n > 1 && wc_width(c) == 0 => Some(n),
        _ => None,
    }
}

/// Move `count` characters forward from `i`, skipping zero-width
/// characters (they belong to the character before them).
pub fn forward_chars_nz(buf: &[u8], i: usize, count: usize) -> usize {
    if !multibyte() {
        return (i + count).min(buf.len());
    }
    let mut p = i;
    while let Some(n) = zero_width_at(buf, p) {
        p += n;
    }
    let mut count = count;
    while count > 0 && p < buf.len() {
        let (wc, n) = decode_at(buf, p);
        match wc {
            None => {
                p += 1;
                count -= 1;
            }
            Some(0) => break,
            Some(c) => {
                p += n.max(1);
                if wc_width(c) != 0 || n == 1 {
                    count -= 1;
                }
            }
        }
    }
    while let Some(n) = zero_width_at(buf, p) {
        p += n;
    }
    p
}

/// The start of the last character before `seed` that has a non-zero
/// width (invalid bytes count as characters).
pub fn prev_char_nz(buf: &[u8], seed: usize) -> usize {
    if !multibyte() {
        return seed.saturating_sub(1);
    }
    if utf8_locale() {
        return prev_utf8_nz(buf, seed.min(buf.len()));
    }
    let mut prev = 0;
    let mut p = 0;
    while p < seed && p < buf.len() {
        let (wc, n) = decode_at(buf, p);
        match wc {
            None => {
                prev = p;
                p += 1;
            }
            Some(0) if n == 1 && buf[p] == 0 => break,
            Some(c) => {
                if n == 1 || wc_width(c) != 0 {
                    prev = p;
                }
                p += n.max(1);
            }
        }
    }
    prev
}

/// Move `count` characters backward (see `prev_char_nz`).
pub fn backward_chars_nz(buf: &[u8], i: usize, count: usize) -> usize {
    let mut p = i;
    for _ in 0..count {
        if p == 0 {
            break;
        }
        p = prev_char_nz(buf, p);
    }
    p
}

/// Is the current locale's codeset UTF-8?
pub fn utf8_locale() -> bool {
    unsafe {
        let p = libc::nl_langinfo(libc::CODESET);
        if p.is_null() {
            return false;
        }
        let c = core::ffi::CStr::from_ptr(p).to_bytes();
        c.eq_ignore_ascii_case(b"UTF-8") || c.eq_ignore_ascii_case(b"utf8")
    }
}

/// Start of the UTF-8 character before `p` judged by the bytes only
/// (continuation bytes belong to the lead byte before them).
fn utf8_char_start_before(buf: &[u8], p: usize) -> usize {
    let mut q = p - 1;
    let b = buf[q];
    if b < 0x80 {
        return q;
    }
    let save = q;
    let mut b = b;
    if b & 0xc0 == 0x80 {
        while q > 0 {
            q -= 1;
            b = buf[q];
            if b & 0xc0 != 0x80 {
                break;
            }
        }
    }
    if b & 0xc0 == 0xc0 {
        q
    } else {
        save
    }
}

/// UTF-8 `prev_char_nz`: zero-width characters are skipped; an invalid
/// lead byte after another non-ASCII byte goes with the character before
/// it (as the reference behaves).
fn prev_utf8_nz(buf: &[u8], seed: usize) -> usize {
    let mut p = seed;
    while p > 0 {
        let q = utf8_char_start_before(buf, p);
        let b = buf[q];
        if b < 0x80 || b & 0xc0 != 0xc0 {
            return q;
        }
        match decode_at(buf, q) {
            (None, _) => {
                if q > 0 && buf[q - 1] >= 0x80 {
                    return utf8_char_start_before(buf, q);
                }
                return q;
            }
            (Some(c), _) if wc_width(c) == 0 => {
                p = q;
                continue;
            }
            _ => return q,
        }
    }
    0
}
