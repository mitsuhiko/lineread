//! Displaying completion matches: the "Display all N possibilities"
//! question, the column layout (vertical or horizontal), the `--More--`
//! pager, filename indicators (visible-stats, mark-directories), colors,
//! completion-prefix-display-length.
//! Owner: display & completion workstream.
use super::*;
use crate::terminal;

/// Write listing text to `rl_outstream` (the terminal buffer is flushed
/// first so the order of output is kept).
pub fn write_listing(b: &[u8]) {
    terminal::flush();
    unsafe {
        let o = crate::readline::outstream();
        crate::util::fwrite_bytes(o, b);
        crate::util::fflush(o);
    }
}

fn crlf() {
    unsafe {
        terminal::rl_crlf();
    }
}

/// The part of a match shown in listings: the basename for filenames.
pub fn printable_part(path: &[u8]) -> &[u8] {
    if rl_filename_completion_desired.get() == 0 {
        return path;
    }
    let Some(slash) = path.iter().rposition(|&c| c == b'/') else {
        return path;
    };
    if slash + 1 == path.len() {
        // trailing slash: show the last component with its slash
        if slash == 0 {
            return path;
        }
        let mut x = slash - 1;
        while x > 0 && path[x] != b'/' {
            x -= 1;
        }
        if path[x] == b'/' {
            return &path[x + 1..];
        }
        return path;
    }
    &path[slash + 1..]
}

/// Display width of a match as printed (control characters as ^X).
pub fn fnwidth(s: &[u8]) -> usize {
    let mut w = 0;
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b < 0x20 || b == 0x7f {
            w += 2;
            i += 1;
            continue;
        }
        let (wc, n) = crate::mb::decode_at(s, i);
        if n > 1 {
            let cw = crate::mb::wc_width(wc.unwrap_or(0));
            w += if cw < 0 { 1 } else { cw as usize };
            i += n;
        } else {
            w += 1;
            i += 1;
        }
    }
    w
}

/// Printable bytes of a name: control characters as ^X.
fn visible_bytes(s: &[u8], out: &mut Vec<u8>) {
    for &b in s {
        if b < 0x20 {
            out.push(b'^');
            out.push(b + 64);
        } else if b == 0x7f {
            out.extend_from_slice(b"^?");
        } else {
            out.push(b);
        }
    }
}

/// The visible-stats character for a file.
unsafe fn stat_char(path: &[u8]) -> Option<u8> {
    let mut p = path.to_vec();
    if let Some(h) = rl_filename_stat_hook.get() {
        p = call_icpp(h, &p).1;
    }
    let c = c_strdup(&p);
    let mut st: libc::stat = core::mem::zeroed();
    let r = libc::lstat(c, &mut st);
    let x = libc::access(c, libc::X_OK);
    cfree(c);
    if r == -1 {
        return None;
    }
    let fmt = st.st_mode & libc::S_IFMT;
    Some(match fmt {
        libc::S_IFDIR => b'/',
        libc::S_IFCHR => b'%',
        libc::S_IFBLK => b'#',
        libc::S_IFLNK => b'@',
        libc::S_IFSOCK => b'=',
        libc::S_IFIFO => b'|',
        libc::S_IFREG if x == 0 => b'*',
        _ => return None,
    })
}

/// The file indicator after a name (visible-stats or a directory slash).
unsafe fn indicator(to_print_off: usize, full: &[u8]) -> Option<u8> {
    let vs = crate::vars::visible_stats.get() != 0;
    let md = crate::vars::mark_directories.get() != 0;
    if rl_filename_completion_desired.get() == 0 || !(vs || md) {
        return None;
    }
    if to_print_off != 0 {
        // name has a directory part: rebuild the path from the directory
        let dir = &full[..to_print_off - 1];
        let dn: &[u8] = if dir.is_empty() {
            b"/"
        } else if dir[0] != b'/' {
            dir
        } else if dir.len() == 1 {
            b"//"
        } else if dir == b"//" {
            b"/"
        } else {
            dir
        };
        let mut s = filename::tilde_expand_bytes(dn);
        if let Some(h) = rl_directory_completion_hook.get() {
            s = call_icpp(h, &s).1;
        }
        let mut np = s.clone();
        if np.last() != Some(&b'/') {
            np.push(b'/');
        }
        np.extend_from_slice(&full[to_print_off..]);
        if vs {
            stat_char(&np)
        } else {
            if rl_directory_completion_hook.get().is_none() {
                if let Some(h) = rl_filename_stat_hook.get() {
                    np = call_icpp(h, &np).1;
                }
            }
            if path_isdir(&np) {
                Some(b'/')
            } else {
                None
            }
        }
    } else {
        let s = filename::tilde_expand_bytes(full);
        if vs {
            stat_char(&s)
        } else if path_isdir(&s) {
            Some(b'/')
        } else {
            None
        }
    }
}

/// Print one match (`to_print` is a suffix of `full`).  `prefix_bytes`:
/// length of the common prefix to abbreviate or color.  Returns the
/// printed width.
unsafe fn print_filename(out: &mut Vec<u8>, to_print: &[u8], full: &[u8], prefix_bytes: usize) -> usize {
    let off = full.len() - to_print.len();
    let mut width = 0;
    let colored_stats = crate::vars::colored_stats.get() != 0 && colors::enabled();
    let colored_prefix = crate::vars::colored_completion_prefix.get() != 0 && colors::enabled();
    let plen = crate::vars::completion_prefix_display_length.get();
    if prefix_bytes > 0 && plen > 0 {
        // abbreviated common prefix
        let rest = &to_print[prefix_bytes.min(to_print.len())..];
        if !rest.is_empty() {
            let ell = if rest[0] == b'.' { b'_' } else { b'.' };
            out.extend_from_slice(&[ell; 3]);
            width += 3;
            if colored_stats {
                colors::set_normal_color(out);
                colors::print_color_indicator(out, full);
            }
            visible_bytes(rest, out);
            width += fnwidth(rest);
            if colored_stats {
                colors::prep_non_filename_text(out);
            }
        } else {
            if colored_stats {
                colors::set_normal_color(out);
                colors::print_color_indicator(out, full);
            }
            visible_bytes(to_print, out);
            width += fnwidth(to_print);
            if colored_stats {
                colors::prep_non_filename_text(out);
            }
        }
    } else if prefix_bytes > 0 && colored_prefix {
        let pb = prefix_bytes.min(to_print.len());
        colors::set_normal_color(out);
        colors::print_prefix_color(out);
        visible_bytes(&to_print[..pb], out);
        colors::prep_non_filename_text(out);
        let rest = &to_print[pb..];
        if colored_stats {
            colors::set_normal_color(out);
            colors::print_color_indicator(out, full);
        }
        visible_bytes(rest, out);
        if colored_stats {
            colors::prep_non_filename_text(out);
        }
        width += fnwidth(to_print);
    } else if colored_stats {
        colors::set_normal_color(out);
        colors::print_color_indicator(out, full);
        visible_bytes(to_print, out);
        colors::prep_non_filename_text(out);
        width += fnwidth(to_print);
    } else {
        visible_bytes(to_print, out);
        width += fnwidth(to_print);
    }
    if let Some(c) = indicator(off, full) {
        out.push(c);
        width += 1;
    }
    width
}

/// Read y/n (or the pager keys).  Ok(1) yes, Ok(0) no, Ok(2) newline (pager
/// only), Err(()) aborted.
unsafe fn get_y_or_n(for_pager: bool) -> Result<c_int, ()> {
    if is_state(RL_STATE_CALLBACK) {
        return Ok(1);
    }
    loop {
        set_state(RL_STATE_MOREINPUT);
        let c = crate::input::rl_read_key();
        unset_state(RL_STATE_MOREINPUT);
        if c == b'y' as c_int || c == b'Y' as c_int || c == b' ' as c_int {
            return Ok(1);
        }
        if c == b'n' as c_int || c == b'N' as c_int || c == RUBOUT {
            return Ok(0);
        }
        if c == ABORT_CHAR || c < 0 {
            note_aborted();
            return Err(());
        }
        if for_pager && (c == b'\n' as c_int || c == b'\r' as c_int) {
            return Ok(2);
        }
        if for_pager && (c == b'q' as c_int || c == b'Q' as c_int) {
            return Ok(0);
        }
        crate::terminal::rl_ding();
    }
}

/// `--More--`: returns the new line count, or None to stop.
unsafe fn pager(lines: c_int) -> Result<Option<c_int>, ()> {
    write_listing(b"--More--");
    let r = get_y_or_n(true)?;
    // erase the prompt line
    terminal::out_byte(b'\r');
    terminal::out_cap("el");
    terminal::out_byte(b'\r');
    terminal::flush();
    Ok(match r {
        0 => None,
        2 => Some(lines - 1),
        _ => Some(0),
    })
}

/// Lay out and print the matches (`m[0]` is the common prefix).  Errors
/// mean the user aborted at the pager.
pub unsafe fn display_match_list(m: &[Vec<u8>], len: usize, max: usize) -> Result<(), ()> {
    let sw = terminal::screen_width() as usize;
    let mut max = max;
    // common prefix to abbreviate/color
    let mut common_len = 0usize;
    let mut sind = 0usize;
    let plen = crate::vars::completion_prefix_display_length.get();
    let colored_prefix = crate::vars::colored_completion_prefix.get() != 0 && colors::enabled();
    if plen > 0 || colored_prefix {
        let t = printable_part(&m[0]);
        let temp = if rl_filename_completion_desired.get() != 0 {
            t.iter().rposition(|&c| c == b'/').map(|i| &t[i + 1..])
        } else {
            None
        };
        let tt = temp.unwrap_or(t);
        common_len = fnwidth(tt);
        sind = tt.len();
        if common_len > max || sind > max {
            common_len = 0;
            sind = 0;
        }
        if plen > 0 {
            if common_len > plen as usize && common_len > 3 {
                max -= common_len - 3;
            } else {
                common_len = 0;
                sind = 0;
            }
        }
    }
    let _ = common_len;
    max += 2;
    let cdw = crate::vars::completion_display_width.get();
    let cols = if cdw >= 0 && (cdw as usize) < sw { cdw as usize } else { sw };
    let mut limit: isize = (cols / max) as isize;
    if limit != 1 && limit as usize * max == cols {
        limit -= 1;
    }
    if cols < sw && limit < 0 {
        limit = 1;
    }
    if limit <= 0 {
        limit = 1;
    }
    let limit = limit as usize;
    let count = (len + limit - 1) / limit;
    let rows = terminal::screen_height();
    let page = crate::vars::page_completions.get() != 0;
    crlf();
    let mut lines: c_int = 0;
    let mut out = Vec::new();
    if crate::vars::print_completions_horizontally.get() == 0 {
        for i in 1..=count {
            let mut l = i;
            for j in 0..limit {
                if l > len || l >= m.len() {
                    break;
                }
                let full = &m[l];
                let tp = printable_part(full);
                out.clear();
                let pl = print_filename(&mut out, tp, full, sind);
                if j + 1 < limit {
                    if max <= pl {
                        out.push(b' ');
                    } else {
                        out.extend(core::iter::repeat(b' ').take(max - pl));
                    }
                }
                write_listing(&out);
                l += count;
            }
            crlf();
            lines += 1;
            if page && lines >= rows - 1 && i < count {
                match pager(lines)? {
                    None => return Ok(()),
                    Some(n) => lines = n,
                }
            }
        }
    } else {
        let mut i = 1;
        while i < m.len() && i <= len {
            let full = &m[i];
            let tp = printable_part(full);
            out.clear();
            let pl = print_filename(&mut out, tp, full, sind);
            write_listing(&out);
            if i + 1 < m.len() && i + 1 <= len {
                if limit == 1 || i % limit == 0 {
                    crlf();
                    lines += 1;
                    if page && lines >= rows - 1 {
                        match pager(lines)? {
                            None => return Ok(()),
                            Some(n) => lines = n,
                        }
                    }
                } else {
                    let pad = if max <= pl { 1 } else { max - pl };
                    write_listing(&vec![b' '; pad]);
                }
            }
            i += 1;
        }
        crlf();
    }
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn rl_display_match_list(matches: *mut *mut c_char, len: c_int, max: c_int) {
    if matches.is_null() {
        return;
    }
    let mut m = Vec::new();
    for p in strvec_iter(matches) {
        m.push(cstr(p).to_vec());
    }
    if m.is_empty() {
        return;
    }
    let _ = display_match_list(&m, len.max(0) as usize, max.max(0) as usize);
}

/// Display the matches for possible-completions (or a second TAB).
pub unsafe fn display_matches(m: &Matches) {
    crate::display::move_to_last_row();
    if m.len() == 1 {
        crlf();
        let mut out = Vec::new();
        print_filename(&mut out, printable_part(&m[0]), &m[0], 0);
        write_listing(&out);
        crlf();
        crate::display::rl_forced_update_display();
        return;
    }
    let mut sorted = m.clone();
    if rl_sort_completion_matches.get() != 0 {
        sorted[1..].sort();
    }
    let max = sorted[1..].iter().map(|s| fnwidth(printable_part(s))).max().unwrap_or(0);
    let len = sorted.len() - 1;
    if let Some(h) = rl_completion_display_matches_hook.get() {
        let v = to_c_vec(&sorted);
        h(v, len as c_int, max as c_int);
        free_c_vec(v);
        return;
    }
    let qi = rl_completion_query_items.get();
    if qi > 0 && len as c_int >= qi {
        crlf();
        write_listing(format!("Display all {} possibilities? (y or n)", len).as_bytes());
        match get_y_or_n(false) {
            Err(()) => return,
            Ok(0) => {
                crlf();
                crate::display::rl_forced_update_display();
                return;
            }
            Ok(_) => {}
        }
    }
    if display_match_list(&sorted, len, max).is_err() {
        return;
    }
    crate::display::rl_forced_update_display();
}
