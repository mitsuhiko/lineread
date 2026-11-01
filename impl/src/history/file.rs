//! History files (history(3) "Managing the History File"): reading
//! (ranges, timestamps, multi-line entries), writing (through a temporary
//! file renamed over the target), appending and truncating.
//! Return values are 0 or an errno value.
//! Owner: history workstream.
use super::{entry_at, history_length, new_entry, push_entry};
use crate::history::expand::history_comment_char;
use crate::prelude::*;
use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[no_mangle]
pub static history_file_version: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static history_lines_read_from_file: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_lines_written_to_file: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_multiline_entries: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_write_timestamps: CVar<c_int> = CVar::new(0);

#[cfg(target_os = "macos")]
const ENOTREG: c_int = libc::EFTYPE;
#[cfg(not(target_os = "macos"))]
const ENOTREG: c_int = libc::EINVAL;

/// The file to use: `name`, or `$HOME/.history` (None without HOME).
unsafe fn history_filename(name: *const c_char) -> Option<PathBuf> {
    if let Some(n) = cstr_opt(name) {
        return Some(PathBuf::from(OsStr::from_bytes(n)));
    }
    let home = cstr_opt(libc::getenv(c"HOME".as_ptr()))?;
    let mut p = home.to_vec();
    if !p.ends_with(b"/") {
        p.push(b'/');
    }
    p.extend_from_slice(b".history");
    Some(PathBuf::from(OsStr::from_bytes(&p)))
}

/// Follow symbolic links (also dangling ones) to the real target.
fn resolve_links(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    for _ in 0..32 {
        match std::fs::read_link(&cur) {
            Ok(t) => {
                cur = if t.is_absolute() {
                    t
                } else {
                    cur.parent().map(|d| d.join(&t)).unwrap_or(t)
                };
            }
            Err(_) => break,
        }
    }
    cur
}

fn errno_of(e: &std::io::Error) -> c_int {
    e.raw_os_error().unwrap_or(libc::EIO)
}

fn is_digit(c: Option<&u8>) -> bool {
    matches!(c, Some(c) if c.is_ascii_digit())
}

/// A timestamp line: the comment character followed by a digit.
fn is_ts(line: &[u8], cc: u8) -> bool {
    cc != 0 && line.first() == Some(&cc) && is_digit(line.get(1))
}

/// Complete lines of `data` as (start, content end, newline index).  A
/// final line without a newline is not a line.  The content drops one
/// trailing CR and stops at a NUL byte.
fn split_lines(data: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut v = Vec::new();
    let mut s = 0;
    for (i, &c) in data.iter().enumerate() {
        if c == b'\n' {
            let mut e = i;
            if e > s && data[e - 1] == b'\r' {
                e -= 1;
            }
            v.push((s, e, i));
            s = i + 1;
        }
    }
    v
}

fn content(data: &[u8], s: usize, e: usize) -> &[u8] {
    let l = &data[s..e];
    match l.iter().position(|&c| c == 0) {
        Some(n) => &l[..n],
        None => l,
    }
}

unsafe fn read_range(name: *const c_char, from: c_int, to: c_int) -> c_int {
    history_lines_read_from_file.set(0);
    let Some(path) = history_filename(name) else {
        return 0;
    };
    let data = match std::fs::metadata(&path) {
        Err(e) => return errno_of(&e),
        Ok(m) if !m.is_file() => return ENOTREG,
        Ok(_) => match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => return errno_of(&e),
        },
    };
    if data.is_empty() {
        return 0;
    }
    let saved_cc = history_comment_char.get();
    let mut cc = saved_cc as u8;
    if cc == 0 && data[0] == b'#' && is_digit(data.get(1)) {
        cc = b'#';
    }
    let has_ts = cc != 0 && data[0] == cc && is_digit(data.get(1));
    if has_ts && super::file::history_write_timestamps.get() != 0 {
        history_multiline_entries.add(1);
    }
    let multiline = history_multiline_entries.get() != 0;
    history_comment_char.set(cc as c_char);

    let lines = split_lines(&data);
    let mut idx = 0;
    let mut current: c_int = 0;
    // a timestamp seen while skipping lines: the reference keeps a pointer
    // into its unterminated buffer, so the stamp extends up to the end of
    // the first line it terminates later (or a NUL in the file).
    let mut skip_ts: Option<usize> = None;
    while current < from && idx < lines.len() {
        let (s, e, _) = lines[idx];
        if is_ts(content(&data, s, e), cc) {
            skip_ts = Some(s);
        } else {
            current += 1;
        }
        idx += 1;
    }
    let mut last_ts: Option<Vec<u8>> = None;
    let mut added = false;
    while idx < lines.len() {
        let (s, e, _) = lines[idx];
        idx += 1;
        let line = content(&data, s, e);
        if is_ts(line, cc) {
            last_ts = Some(line.to_vec());
            skip_ts = None;
            continue;
        }
        if let Some(ts0) = skip_ts.take() {
            let mut end = e;
            if let Some(n) = data[ts0..e].iter().position(|&c| c == 0) {
                end = ts0 + n;
            }
            last_ts = Some(data[ts0..end].to_vec());
        }
        if multiline && last_ts.is_none() && added && history_length.get() > 0 {
            let ent = entry_at(history_length.get() - 1);
            let mut t = cstr((*ent).line).to_vec();
            t.push(b'\n');
            t.extend_from_slice(line);
            cfree((*ent).line);
            (*ent).line = c_strdup(&t);
        } else if !line.is_empty() {
            let ts = match last_ts.take() {
                Some(t) => super::dup_timestamp(&t),
                None => super::now_timestamp(),
            };
            push_entry(new_entry(line, ts, null_mut()));
            added = true;
        }
        current += 1;
        if to >= 0 && current >= to {
            break;
        }
    }
    history_comment_char.set(saved_cc);
    history_lines_read_from_file.set(current);
    0
}

#[no_mangle]
pub unsafe extern "C" fn read_history(name: *const c_char) -> c_int {
    read_range(name, 0, -1)
}

#[no_mangle]
pub unsafe extern "C" fn read_history_range(name: *const c_char, from: c_int, to: c_int) -> c_int {
    read_range(name, from, to)
}

/// The text written for entries `[first, history_length)`.
unsafe fn render(first: c_int) -> Vec<u8> {
    let mut out = Vec::new();
    for i in first.max(0)..history_length.get() {
        let e = entry_at(i);
        if e.is_null() {
            continue;
        }
        if history_write_timestamps.get() != 0 {
            let ts = cstr((*e).timestamp);
            if !ts.is_empty() {
                out.extend_from_slice(ts);
                out.push(b'\n');
            }
        }
        out.extend_from_slice(cstr((*e).line));
        out.push(b'\n');
    }
    out
}

/// Replace the contents of `path` (after following links): regular or
/// missing files are written to a temporary file (mode 0600) renamed over
/// the target, anything else is written in place.
fn replace_file(path: &Path, text: &[u8]) -> c_int {
    let target = resolve_links(path);
    let regular_or_missing = match std::fs::metadata(&target) {
        Ok(m) => m.is_file(),
        Err(_) => true,
    };
    if !regular_or_missing {
        let r = std::fs::OpenOptions::new().write(true).truncate(true).create(true).mode(0o600).open(&target);
        return match r {
            Ok(mut f) => match f.write_all(text) {
                Ok(()) => 0,
                Err(e) => errno_of(&e),
            },
            Err(e) => errno_of(&e),
        };
    }
    let mut tmp = target.as_os_str().as_bytes().to_vec();
    tmp.extend_from_slice(format!("-{}.tmp", std::process::id()).as_bytes());
    let tmp = PathBuf::from(OsStr::from_bytes(&tmp));
    let r = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp);
    let mut f = match r {
        Ok(f) => f,
        Err(e) => return errno_of(&e),
    };
    if let Err(e) = f.write_all(text) {
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        return errno_of(&e);
    }
    drop(f);
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return errno_of(&e);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn write_history(name: *const c_char) -> c_int {
    let Some(path) = history_filename(name) else {
        return 0;
    };
    history_lines_written_to_file.set(history_length.get());
    let r = replace_file(&path, &render(0));
    if r != 0 {
        history_lines_written_to_file.set(0);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn append_history(n: c_int, name: *const c_char) -> c_int {
    let Some(path) = history_filename(name) else {
        return 0;
    };
    let n = n.min(history_length.get());
    history_lines_written_to_file.set(n);
    let mut f = match std::fs::OpenOptions::new().append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            history_lines_written_to_file.set(0);
            return errno_of(&e);
        }
    };
    if n <= 0 {
        return 0;
    }
    match f.write_all(&render(history_length.get() - n)) {
        Ok(()) => 0,
        Err(e) => errno_of(&e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn history_truncate_file(name: *const c_char, n: c_int) -> c_int {
    history_lines_written_to_file.set(0);
    let Some(path) = history_filename(name) else {
        return 0;
    };
    let target = resolve_links(&path);
    match std::fs::metadata(&target) {
        Err(e) => return errno_of(&e),
        Ok(m) if !m.is_file() => return 0,
        Ok(_) => {}
    }
    let data = match std::fs::read(&target) {
        Ok(d) => d,
        Err(e) => return errno_of(&e),
    };
    let n = n.max(0) as usize;
    let lines = split_lines(&data);
    history_lines_written_to_file.set(n.min(lines.len()) as c_int);
    let start = if n == 0 {
        data.len()
    } else if history_write_timestamps.get() != 0 {
        // keep the last n entries, each starting with its timestamp line
        let cc = history_comment_char.get() as u8;
        let ts: Vec<usize> = lines.iter().filter(|l| is_ts(&data[l.0..l.1], cc)).map(|l| l.0).collect();
        if n >= ts.len() {
            return 0;
        }
        ts[ts.len() - n]
    } else {
        if n >= lines.len() {
            return 0;
        }
        lines[lines.len() - n].0
    };
    replace_file(&target, &data[start..])
}
