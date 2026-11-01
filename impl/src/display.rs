//! Redisplay: prompt expansion, laying out prompt + line into screen rows,
//! and updating the terminal incrementally from a model of what is on the
//! screen.  Also messages (rl_message), prompt save/restore, clear-screen.
//!
//! Model: `Screen.rows` holds the cells currently displayed, rows relative
//! to the first row of the prompt; `(cur_row, cur_col)` is the physical
//! cursor.  `rl_redisplay` lays out the desired rows, rewrites the
//! differing tail of every changed row and moves the cursor to point.
//! Only the final terminal state matters (tests compare screens), so the
//! byte stream is free-form.
//! Owner: display workstream.
use crate::buffer;
use crate::input::{is_state, set_state, unset_state};
use crate::mb;
use crate::prelude::*;
use crate::terminal::{self, out, out_byte, out_cap};

#[no_mangle]
pub static rl_prompt: CVar<*mut c_char> = CVar::new(null_mut());
#[no_mangle]
pub static rl_display_prompt: CVar<*mut c_char> = CVar::new(null_mut());
#[no_mangle]
pub static rl_redisplay_function: CVar<Option<rl_voidfunc_t>> = CVar::new(Some(rl_redisplay));
#[no_mangle]
pub static rl_already_prompted: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_erase_empty_line: CVar<c_int> = CVar::new(0);

/// One screen cell: the bytes to output (invisible prompt sequences
/// preceding the character are included) and the column width.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cell {
    pub bytes: Vec<u8>,
    pub width: u8,
}

pub type Row = Vec<Cell>;

pub struct Screen {
    pub rows: Vec<Row>,
    pub cur_row: usize,
    pub cur_col: usize,
    /// lowest row the cursor reached (rows more than a screen height above
    /// it have scrolled off the terminal)
    pub maxrow: usize,
}

pub static SCREEN: CVar<Screen> = CVar::new(Screen { rows: Vec::new(), cur_row: 0, cur_col: 0, maxrow: 0 });

fn screen_rows() -> usize {
    terminal::screen_height().max(1) as usize
}

/// The first row still on the terminal.
fn visible_top() -> usize {
    let s = SCREEN.get_mut();
    s.maxrow.max(s.cur_row).saturating_sub(screen_rows() - 1)
}

fn note_row() {
    let s = SCREEN.get_mut();
    s.maxrow = s.maxrow.max(s.cur_row);
}

/// The expanded prompt redisplay shows (set by rl_expand_prompt, i.e. by
/// rl_set_prompt and rl_message); None: show rl_display_prompt.
static LOCAL_PROMPT: CVar<Option<Vec<u8>>> = CVar::new(None);
/// rl_save_prompt storage.
static SAVED_PROMPT: CVar<Vec<Option<Vec<u8>>>> = CVar::new(Vec::new());
/// rl_message saved the prompt (a message is displayed).
static MSG_SAVED: CVar<bool> = CVar::new(false);
/// C copy of the message (rl_display_prompt points to it).
static MESSAGE_C: CVar<*mut c_char> = CVar::new(null_mut());

fn row_width(r: &Row) -> usize {
    r.iter().map(|c| c.width as usize).sum()
}

/// Visible representation of a control/meta byte from the line buffer.
fn ctl_repr(b: u8, out: &mut Vec<u8>) {
    if b == 0x7f {
        out.extend_from_slice(b"^?");
    } else if b < 0x20 {
        out.push(b'^');
        out.push(b + 64);
    } else {
        // non-printable 8-bit byte: octal
        out.push(b'\\');
        out.push(b'0' + (b >> 6));
        out.push(b'0' + ((b >> 3) & 7));
        out.push(b'0' + (b & 7));
    }
}

/// Split a prompt into the lines before the last newline (printed once
/// when the line starts) and the last line (managed by redisplay).
pub fn split_prompt(p: &[u8]) -> (&[u8], &[u8]) {
    let mut invis = false;
    let mut last_nl = None;
    for (i, &b) in p.iter().enumerate() {
        match b {
            RL_PROMPT_START_IGNORE => invis = true,
            RL_PROMPT_END_IGNORE => invis = false,
            b'\n' if !invis => last_nl = Some(i),
            _ => {}
        }
    }
    match last_nl {
        Some(i) => (&p[..=i], &p[i + 1..]),
        None => (&p[..0], p),
    }
}

/// The whole prompt being displayed (all lines).
pub fn full_prompt() -> Vec<u8> {
    match LOCAL_PROMPT.get_mut() {
        Some(p) => p.clone(),
        None => unsafe { cstr(rl_display_prompt.get()).to_vec() },
    }
}

/// The prompt bytes redisplay shows: the mode string and the last line of
/// the prompt (or of the message).
pub fn display_prompt_bytes() -> Vec<u8> {
    let mut p = Vec::new();
    unsafe {
        if crate::vars::mark_modified_lines.get() != 0
            && !crate::undo::rl_undo_list.get().is_null()
            && !crate::history::current_history().is_null()
        {
            p.push(b'*');
        }
        if !MSG_SAVED.get() && crate::vars::show_mode_in_prompt.get() != 0 {
            p.extend_from_slice(&mode_string());
        }
        let full = full_prompt();
        p.extend_from_slice(split_prompt(&full).1);
    }
    p
}

/// Print the lines of a multi-line prompt before its last line (done once
/// when a line starts).
pub fn print_prompt_prefix() {
    unsafe {
        let full = full_prompt();
        let (prefix, _) = split_prompt(&full);
        if prefix.is_empty() {
            return;
        }
        let mut out_b = Vec::new();
        let mut invis = false;
        for &b in prefix {
            match b {
                RL_PROMPT_START_IGNORE => invis = true,
                RL_PROMPT_END_IGNORE => invis = false,
                _ => {
                    let _ = invis;
                    out_b.push(b)
                }
            }
        }
        out(&out_b);
        terminal::flush();
    }
}

/// The mode indicator for show-mode-in-prompt (with invisible markers as
/// given by the user).
pub fn mode_string() -> Vec<u8> {
    if crate::readline::rl_editing_mode.get() == crate::readline::EMACS_MODE {
        crate::vars::emacs_mode_bytes()
    } else if crate::keymap::current() == crate::keymap::vi_insertion() {
        crate::vars::vi_ins_mode_bytes()
    } else {
        crate::vars::vi_cmd_mode_bytes()
    }
}

/// Layout state while building rows.
struct Lay {
    rows: Vec<Row>,
    col: usize,
    width: usize,
    pending: Vec<u8>,
}

impl Lay {
    fn new(width: usize) -> Self {
        Lay { rows: vec![Vec::new()], col: 0, width: width.max(1), pending: Vec::new() }
    }
    fn newline(&mut self) {
        if !self.pending.is_empty() {
            let b = core::mem::take(&mut self.pending);
            self.rows.last_mut().unwrap().push(Cell { bytes: b, width: 0 });
        }
        self.rows.push(Vec::new());
        self.col = 0;
    }
    /// Wrap first if a cell of width `w` does not fit.
    fn fit(&mut self, w: usize) {
        if self.col + w > self.width && self.col > 0 {
            self.newline();
        }
    }
    fn put(&mut self, bytes: &[u8], w: usize) {
        self.fit(w);
        let mut b = core::mem::take(&mut self.pending);
        b.extend_from_slice(bytes);
        self.rows.last_mut().unwrap().push(Cell { bytes: b, width: w as u8 });
        self.col += w;
    }
    /// Column counted from the start of the first row.
    fn abs_col(&self) -> usize {
        self.rows[..self.rows.len() - 1].iter().map(row_width).sum::<usize>() + self.col
    }
    fn pos(&self) -> (usize, usize) {
        (self.rows.len() - 1, self.col)
    }
    fn finish(&mut self) {
        if !self.pending.is_empty() {
            let b = core::mem::take(&mut self.pending);
            self.rows.last_mut().unwrap().push(Cell { bytes: b, width: 0 });
        }
    }
}

/// Lay out the prompt into `lay`.
fn layout_prompt(lay: &mut Lay, prompt: &[u8]) {
    let mut i = 0;
    let mut invis = false;
    while i < prompt.len() {
        let b = prompt[i];
        if b == RL_PROMPT_START_IGNORE {
            invis = true;
            i += 1;
            continue;
        }
        if b == RL_PROMPT_END_IGNORE && invis {
            invis = false;
            i += 1;
            continue;
        }
        if invis {
            lay.pending.push(b);
            i += 1;
            continue;
        }
        if b == b'\n' {
            lay.newline();
            i += 1;
            continue;
        }
        let (wc, n) = mb::decode_at(prompt, i);
        let w = match wc {
            Some(c) if n > 1 => mb::wc_width(c).max(0) as usize,
            _ => 1,
        };
        lay.put(&prompt[i..i + n], w);
        i += n;
    }
}

/// Lay out the line buffer; returns the cursor position for `point`.
fn layout_line(lay: &mut Lay, line: &[u8], point: usize) -> (usize, usize) {
    let mut cursor = None;
    let mut i = 0;
    let mut tmp = Vec::new();
    while i < line.len() {
        let b = line[i];
        let (wc, n) = mb::decode_at(line, i);
        // compute the representation and its width
        tmp.clear();
        let w;
        if b == b'\n' && crate::vars::horizontal_scroll_mode.get() == 0 && terminal::cap("cuu1").is_some() {
            // a newline in the line starts a new screen row
            if i == point {
                cursor = Some(lay.pos());
            }
            lay.newline();
            i += 1;
            continue;
        }
        if b == b'\t' {
            // tab stops count from the start of the line (prompt
            // included), the blanks may continue on the next row
            let abs = lay.abs_col();
            let sp = (abs / 8 + 1) * 8 - abs;
            if lay.col >= lay.width {
                lay.newline();
            }
            if i == point {
                cursor = Some(lay.pos());
            }
            for _ in 0..sp {
                lay.put(b" ", 1);
            }
            i += 1;
            continue;
        } else if n == 1 && (b < 0x20 || b == 0x7f) {
            ctl_repr(b, &mut tmp);
            w = tmp.len();
        } else if n == 1 && b >= 0x80 && (wc.is_none() || !mb::multibyte()) {
            if crate::vars::output_meta.get() != 0 || mb::multibyte() {
                tmp.push(b);
                w = 1;
            } else {
                ctl_repr(b, &mut tmp);
                w = tmp.len();
            }
        } else if n > 1 {
            tmp.extend_from_slice(&line[i..i + n]);
            let cw = mb::wc_width(wc.unwrap_or(0));
            w = if cw < 0 { 1 } else { cw as usize };
        } else {
            tmp.push(b);
            w = 1;
        }
        if w > 1 && tmp.len() == w && tmp.iter().all(|c| c.is_ascii()) {
            // ^X / \ooo: may be split across rows like plain characters
            lay.fit(1);
            if i == point {
                cursor = Some(lay.pos());
            }
            for k in 0..tmp.len() {
                let c = [tmp[k]];
                lay.put(&c, 1);
            }
        } else {
            lay.fit(w);
            if i == point {
                cursor = Some(lay.pos());
            }
            let t = tmp.clone();
            lay.put(&t, w);
        }
        i += n;
    }
    lay.finish();
    if let Some(c) = cursor {
        return c;
    }
    // point at the end
    if lay.col >= lay.width {
        lay.rows.push(Vec::new());
        lay.col = 0;
    }
    lay.pos()
}

/// Build the rows for the current prompt and line.
pub fn layout() -> (Vec<Row>, (usize, usize)) {
    let width = terminal::usable_width() as usize;
    let mut lay = Lay::new(width);
    let prompt = display_prompt_bytes();
    layout_prompt(&mut lay, &prompt);
    // invisible bytes at the end of the prompt stay with the prompt
    lay.finish();
    let line = buffer::line().to_vec();
    let point = buffer::point();
    let cursor = layout_line(&mut lay, &line, point);
    (lay.rows, cursor)
}

// ---------------------------------------------------------------- horizontal scrolling

/// Left margin (in columns) of the horizontally scrolled line.
static LAST_LMARGIN: CVar<usize> = CVar::new(0);

/// One-line display: horizontal-scroll-mode, or a terminal that cannot
/// move the cursor up.
pub fn horizontal_mode() -> bool {
    crate::vars::horizontal_scroll_mode.get() != 0 || terminal::cap("cuu1").is_none() || terminal::screen_height() <= 1
}

/// Lay out the prompt and line as a single row scrolled so the cursor is
/// visible; `<` / `>` mark text beyond the left / right edge.
fn layout_hscroll() -> (Vec<Row>, (usize, usize)) {
    // the last column is never used
    let sw = terminal::hscroll_width() as isize;
    let mut lay = Lay::new(usize::MAX / 4);
    let prompt = display_prompt_bytes();
    layout_prompt(&mut lay, &prompt);
    lay.finish();
    // a single row: the prompt's last line only
    let prow = lay.rows.pop().unwrap_or_default();
    let mut lay = Lay::new(usize::MAX / 4);
    lay.rows[0] = prow;
    lay.col = row_width(&lay.rows[0]);
    let line = buffer::line().to_vec();
    let point = buffer::point();
    let (_, ccol) = layout_line(&mut lay, &line, point);
    let cells: Row = lay.rows.into_iter().flatten().collect();
    let total: isize = cells.iter().map(|c| c.width as isize).sum();
    let cpos = ccol as isize;
    let last = LAST_LMARGIN.get() as isize;
    let t = (sw / 3).max(1);
    let phys = cpos - last;
    let lmargin = if phys > sw - 2 {
        (cpos - 2 * t).max(0)
    } else if cpos < sw - 2 {
        0
    } else if phys < 1 {
        ((cpos - 1) / t) * t
    } else {
        last
    };
    let lmargin = lmargin.max(0);
    LAST_LMARGIN.set(lmargin as usize);
    let mut row: Row = Vec::new();
    let mut col = 0isize;
    let right = lmargin + sw;
    for c in cells {
        let w = c.width as isize;
        if col < lmargin {
            col += w;
            continue;
        }
        if col >= right {
            break;
        }
        if col == lmargin && lmargin > 0 && w > 0 {
            row.push(Cell { bytes: b"<".to_vec(), width: 1 });
        } else if col + w >= right && right < total && w > 0 {
            row.push(Cell { bytes: b">".to_vec(), width: 1 });
        } else {
            row.push(c);
        }
        col += w;
    }
    (vec![row], (0, (cpos - lmargin).max(0) as usize))
}

// ---------------------------------------------------------------- output

fn width() -> usize {
    terminal::usable_width() as usize
}

/// Move the physical cursor to (row, col) of the model.
fn move_to(rows: &[Row], row: usize, col: usize) {
    let s = SCREEN.get_mut();
    let w = width();
    if s.cur_row > row {
        let n = s.cur_row - row;
        if n == 1 || terminal::cap("cuu").is_none() {
            for _ in 0..n {
                if !out_cap("cuu1") {
                    break;
                }
            }
        } else {
            out(&terminal::cap_param("cuu", n as i32).unwrap_or_default());
        }
        s.cur_row = row;
        if s.cur_col >= w {
            s.cur_col = w.saturating_sub(1);
            // pending-wrap state: resolve with a carriage return
            out_byte(b'\r');
            s.cur_col = 0;
        }
    } else if s.cur_row < row {
        out_byte(b'\r');
        for _ in 0..(row - s.cur_row) {
            out_byte(b'\n');
        }
        s.cur_row = row;
        s.cur_col = 0;
        s.maxrow = s.maxrow.max(row);
    }
    if s.cur_col == col {
        return;
    }
    if s.cur_col >= w {
        out_byte(b'\r');
        s.cur_col = 0;
    }
    if col == 0 {
        out_byte(b'\r');
        s.cur_col = 0;
        return;
    }
    if col < s.cur_col {
        let n = s.cur_col - col;
        // a carriage return and moving forward is used when that is
        // shorter than moving back (like the reference)
        let cr_faster = col + 1 < n;
        if !cr_faster {
            for _ in 0..n {
                if !out_cap("cub1") {
                    out_byte(8);
                }
            }
            s.cur_col = col;
            return;
        }
        out_byte(b'\r');
        s.cur_col = 0;
    }
    let n = col - s.cur_col;
    if n > 0 {
        if let Some(seq) = terminal::cap_param("cuf", n as i32) {
            out(&seq);
            s.cur_col = col;
        } else {
            // rewrite the cells between
            let r = rows.get(row);
            let mut c = 0;
            let mut buf = Vec::new();
            if let Some(r) = r {
                for cell in r {
                    if c >= s.cur_col && c < col {
                        buf.extend_from_slice(&cell.bytes);
                    }
                    c += cell.width as usize;
                    if c >= col {
                        break;
                    }
                }
            }
            if c == col && !buf.is_empty() {
                out(&buf);
            } else {
                for _ in 0..n {
                    out_cap("cuf1");
                }
            }
            s.cur_col = col;
        }
    }
}

/// Clear from the cursor to the end of the row; without the capability
/// `n` blanks are written (the cursor moves past them).
fn clear_eol_n(n: usize) {
    if !out_cap("el") {
        for _ in 0..n {
            out_byte(b' ');
        }
        SCREEN.get_mut().cur_col += n;
    }
}

fn clear_eol() {
    clear_eol_n(0);
}

/// Bring the screen from `SCREEN.rows` to `new`, cursor at `cursor`.
/// Only rows that fit on the terminal around the cursor are drawn; when
/// the cursor moves above the first visible row the screen is cleared and
/// drawn from the cursor's row.
fn update(new: Vec<Row>, cursor: (usize, usize)) {
    let w = width();
    let h = screen_rows();
    let top = visible_top();
    let wtop = if cursor.0 < top {
        // the cursor's row scrolled off: clear and redraw from it
        if !out_cap("clear") {
            out_byte(b'\n');
        }
        let s = SCREEN.get_mut();
        s.rows.clear();
        s.cur_row = cursor.0;
        s.cur_col = 0;
        s.maxrow = cursor.0 + h - 1;
        cursor.0
    } else if cursor.0 >= top + h {
        cursor.0 + 1 - h
    } else {
        top
    };
    let wend = wtop + h;
    let old = core::mem::take(&mut SCREEN.get_mut().rows);
    let n = old.len().max(new.len()).min(wend);
    for r in wtop..n {
        let empty = Vec::new();
        let o = old.get(r).unwrap_or(&empty);
        let nr = new.get(r);
        let nrow = nr.unwrap_or(&empty);
        if o == nrow && r < old.len() {
            continue;
        }
        let mut i = 0;
        while i < o.len() && i < nrow.len() && o[i] == nrow[i] {
            i += 1;
        }
        let col: usize = nrow[..i].iter().map(|c| c.width as usize).sum();
        if r >= new.len() {
            // row no longer used: clear it
            move_to(&new, r.min(new.len().max(1) - 1), 0);
            let s = SCREEN.get_mut();
            if r > s.cur_row {
                out_byte(b'\r');
                for _ in s.cur_row..r {
                    out_byte(b'\n');
                }
                s.cur_row = r;
                s.cur_col = 0;
            }
            note_row();
            clear_eol_n(row_width(o));
            continue;
        }
        {
            let s = SCREEN.get_mut();
            if s.cur_row + 1 == r && s.cur_col >= w && terminal::has_auto_margins() && i < nrow.len() {
                // at the right margin: the next character written wraps
                // (the reference relies on the terminal's auto-margins)
                s.cur_row = r;
                s.cur_col = 0;
                if col > 0 {
                    out(&nrow[0].bytes);
                    s.cur_col = nrow[0].width as usize;
                }
                note_row();
            }
        }
        move_to(&new, r, col);
        let mut buf = Vec::new();
        let mut wsum = 0;
        for c in &nrow[i..] {
            buf.extend_from_slice(&c.bytes);
            wsum += c.width as usize;
        }
        out(&buf);
        let s = SCREEN.get_mut();
        s.cur_col += wsum;
        if row_width(o) > row_width(nrow) || (i < o.len() && row_width(o) > col + wsum) {
            if s.cur_col < w {
                let n = row_width(o).saturating_sub(s.cur_col);
                clear_eol_n(n);
            }
        }
    }
    // remember what is displayed (rows outside the window as before)
    let mut rows = new;
    for r in 0..rows.len() {
        if (r < wtop || r >= wend) && r < old.len() {
            rows[r] = old[r].clone();
        } else if (r < wtop || r >= wend) && r >= old.len() {
            rows[r] = Vec::new();
        }
    }
    move_to(&rows, cursor.0, cursor.1);
    SCREEN.get_mut().rows = rows;
}

#[no_mangle]
pub unsafe extern "C" fn rl_redisplay() {
    if crate::readline::rl_outstream.get().is_null() && !crate::readline::initialized() {
        return;
    }
    if crate::tty::ECHOING.get() == 0 {
        // the terminal does not echo (rl_tty_set_echoing): show nothing
        return;
    }
    set_state(RL_STATE_REDISPLAYING);
    if horizontal_mode() {
        let (rows, cursor) = layout_hscroll();
        update(rows, cursor);
    } else {
        let (rows, cursor) = layout();
        update(rows, cursor);
    }
    terminal::flush();
    unset_state(RL_STATE_REDISPLAYING);
}

/// Call the application's redisplay function (or ours).
pub unsafe fn redisplay() {
    match rl_redisplay_function.get() {
        Some(f) => f(),
        None => rl_redisplay(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_on_new_line() -> c_int {
    LAST_LMARGIN.set(0);
    let s = SCREEN.get_mut();
    s.rows.clear();
    s.cur_row = 0;
    s.cur_col = 0;
    s.maxrow = 0;
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_on_new_line_with_prompt() -> c_int {
    // the prompt is already on the screen; the cursor is after it
    let mut lay = Lay::new(width());
    layout_prompt(&mut lay, &display_prompt_bytes());
    lay.finish();
    let (r, c) = lay.pos();
    let s = SCREEN.get_mut();
    s.rows = lay.rows;
    s.cur_row = r;
    s.cur_col = c;
    s.maxrow = r;
    0
}

/// Move the cursor to the start of the display and clear everything shown.
pub fn clear_displayed() {
    let rows = core::mem::take(&mut SCREEN.get_mut().rows);
    move_to(&rows, 0, 0);
    let s = SCREEN.get_mut();
    if !out_cap("ed") {
        let n = rows.len();
        for r in 0..n {
            clear_eol();
            if r + 1 < n {
                out(b"\n");
            }
        }
        if n > 1 {
            for _ in 1..n {
                out_cap("cuu1");
            }
        }
        out_byte(b'\r');
    }
    s.cur_row = 0;
    s.cur_col = 0;
    s.maxrow = 0;
}

/// Forget what is on the screen: the next redisplay draws the prompt and
/// line from the current cursor position (assumed at column 0).
pub fn forget_screen() {
    unsafe {
        rl_on_new_line();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_forced_update_display() -> c_int {
    forget_screen();
    redisplay();
    0
}

/// Move the cursor to the last row of the displayed line (column kept
/// where possible); used before printing a listing below the line.
pub fn move_to_last_row() {
    let rows = core::mem::take(&mut SCREEN.get_mut().rows);
    if !rows.is_empty() {
        let mut last = rows.len() - 1;
        if rows[last].is_empty() && last > 0 && SCREEN.get_mut().cur_row < last {
            last -= 1;
        }
        let s = SCREEN.get_mut();
        if s.cur_row != last {
            let col = s.cur_col.min(row_width(&rows[last]));
            move_to(&rows, last, col);
        }
    }
    SCREEN.get_mut().rows = rows;
    terminal::flush();
}

/// Move past the end of the displayed line before writing output below.
pub fn move_to_end_for_output() {
    move_to_end();
    terminal::flush();
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_visible_line() -> c_int {
    clear_displayed();
    terminal::flush();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_reset_line_state() -> c_int {
    rl_on_new_line();
    rl_display_prompt.set(if rl_prompt.get().is_null() { cs_mut!("") } else { rl_prompt.get() });
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_redraw_prompt_last_line() {
    clear_displayed();
    redisplay();
}

/// Move the cursor past the end of the displayed line (before printing a
/// newline at accept-line or before a listing).
pub fn move_to_end() {
    let rows = core::mem::take(&mut SCREEN.get_mut().rows);
    if !rows.is_empty() {
        let mut last = rows.len() - 1;
        if rows[last].is_empty() && last > 0 {
            last -= 1;
        }
        let col = row_width(&rows[last]);
        move_to(&rows, last, col);
    }
    SCREEN.get_mut().rows = rows;
}

/// accept-line: cursor after the line, then a newline.
pub fn update_final() {
    if rl_erase_empty_line.get() != 0 && buffer::rl_point.get() == 0 && buffer::rl_end.get() == 0 {
        // erase the prompt instead of moving to a new line
        clear_displayed();
        terminal::flush();
        unsafe { rl_on_new_line() };
        return;
    }
    move_to_end();
    // nothing displayed (e.g. a redisplay function that draws nothing):
    // no newline
    let s = SCREEN.get_mut();
    if s.cur_col > 0 || s.cur_row > 0 || s.rows.iter().any(|r| !r.is_empty()) {
        out_byte(b'\n');
    }
    terminal::flush();
    unsafe { rl_on_new_line() };
}

/// Before printing a listing (dumpers, completions): newline after the line.
pub fn output_before_listing() {
    move_to_end();
    out_byte(b'\n');
    terminal::flush();
}

/// After a listing: redraw the prompt and line below it.
pub fn after_listing() {
    unsafe {
        rl_on_new_line();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_refresh_line(_count: c_int, _key: c_int) -> c_int {
    clear_displayed();
    redisplay();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_screen(_count: c_int, _key: c_int) -> c_int {
    if crate::numarg::rl_explicit_arg.get() != 0 {
        return rl_refresh_line(_count, _key);
    }
    if !out_cap("clear") {
        out_byte(b'\n');
    }
    print_prompt_prefix();
    rl_on_new_line();
    redisplay();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_display(_count: c_int, _key: c_int) -> c_int {
    out_cap("clear");
    out_cap("E3");
    rl_on_new_line();
    redisplay();
    0
}

// ---------------------------------------------------------------- prompts

#[no_mangle]
pub unsafe extern "C" fn rl_set_prompt(prompt: *const c_char) -> c_int {
    let old = rl_prompt.get();
    rl_prompt.set(crate::util::c_strdup_ptr(prompt));
    cfree(old);
    rl_display_prompt.set(if rl_prompt.get().is_null() { cs_mut!("") } else { rl_prompt.get() });
    rl_expand_prompt(rl_prompt.get());
    0
}

/// Visible length of a prompt as `rl_expand_prompt` counts it: bytes
/// outside `\001..\002` (a stray `\002` counts as visible).
fn visible_prompt_len(p: &[u8]) -> c_int {
    let mut n = 0;
    let mut ignoring = false;
    for &b in p {
        match b {
            RL_PROMPT_START_IGNORE if !ignoring => ignoring = true,
            RL_PROMPT_END_IGNORE if ignoring => ignoring = false,
            _ if ignoring => {}
            _ => n += 1,
        }
    }
    n
}

/// Visible characters on the last line of a prompt.
pub fn prompt_last_line_width(p: &[u8]) -> c_int {
    let mut lay = Lay::new(usize::MAX / 2);
    layout_prompt(&mut lay, p);
    lay.col as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rl_expand_prompt(prompt: *mut c_char) -> c_int {
    let p = cstr(prompt);
    if p.is_empty() {
        LOCAL_PROMPT.set(None);
        return 0;
    }
    LOCAL_PROMPT.set(Some(p.to_vec()));
    match p.iter().rposition(|&c| c == b'\n') {
        // multi-line: the length of everything up to the last newline
        Some(i) => visible_prompt_len(&p[..=i]),
        None => visible_prompt_len(p),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_save_prompt() {
    SAVED_PROMPT.get_mut().push(LOCAL_PROMPT.get_mut().take());
}

#[no_mangle]
pub unsafe extern "C" fn rl_restore_prompt() {
    if let Some(p) = SAVED_PROMPT.get_mut().pop() {
        LOCAL_PROMPT.set(p);
    }
}

/// Rust side of rl_message (c/shim.c formats the arguments).
#[no_mangle]
pub unsafe extern "C" fn lr_message_str(msg: *const c_char) -> c_int {
    let m = cstr(msg).to_vec();
    set_message(&m);
    redisplay();
    0
}

/// Show `m` instead of the prompt (saving the prompt the first time).
pub fn set_message(m: &[u8]) {
    unsafe {
        if !MSG_SAVED.get() {
            rl_save_prompt();
            MSG_SAVED.set(true);
        }
        let old = MESSAGE_C.replace(c_strdup(m));
        rl_display_prompt.set(MESSAGE_C.get());
        cfree(old);
        LOCAL_PROMPT.set(Some(m.to_vec()));
    }
}

/// Is a message shown instead of the prompt?
pub fn message_shown() -> bool {
    MSG_SAVED.get()
}

/// Drop the message (without redisplay).
fn clear_message_state() {
    unsafe {
        rl_display_prompt.set(if rl_prompt.get().is_null() { cs_mut!("") } else { rl_prompt.get() });
        if MSG_SAVED.get() {
            rl_restore_prompt();
            MSG_SAVED.set(false);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_message() -> c_int {
    clear_message_state();
    redisplay();
    0
}

/// "(arg: N) " while a numeric argument is being read.
pub fn show_arg_message() {
    unsafe {
        let n = crate::numarg::count();
        let m = format!("(arg: {}) ", n);
        set_message(m.as_bytes());
        redisplay();
    }
}

pub fn clear_arg_message() {
    if drop_message() {
        unsafe { redisplay() };
    }
}

/// Forget the message (the prompt is displayed again at the next
/// redisplay).  Returns whether there was one.
pub fn drop_message() -> bool {
    unsafe {
        if MSG_SAVED.get() {
            clear_message_state();
            return true;
        }
        false
    }
}

pub fn mode_string_changed() {}

/// Terminal type or size changed: the screen model is unreliable.
pub fn screen_changed() {}

/// After SIGWINCH: redraw the line for the new width.
pub fn screen_resized() {
    unsafe {
        if is_state(RL_STATE_INITIALIZED) && crate::readline::in_readline() {
            let custom = rl_redisplay_function.get().map(|f| f as usize) != Some(rl_redisplay as *const () as usize);
            if custom {
                // an application redisplay function gets a forced update
                // (the line is redrawn where the cursor is), but only when
                // the size differs from the last resize (observed)
                static LAST: CVar<Option<(c_int, c_int)>> = CVar::new(None);
                let now = (terminal::SCREEN_ROWS.get(), terminal::SCREEN_COLS.get());
                if LAST.replace(Some(now)) != Some(now) {
                    rl_forced_update_display();
                }
                return;
            }
            clear_displayed();
            redisplay();
        }
    }
}

// ---------------------------------------------------------------- paren blinking

/// Find the opening bracket matching `closer` scanning back from `from`
/// (quoted stretches are skipped).
fn find_matching_open(line: &[u8], from: isize, closer: u8) -> isize {
    let opener = match closer {
        b']' => b'[',
        b'}' => b'{',
        b')' => b'(',
        _ => return -1,
    };
    let quotes = unsafe { cstr(crate::complete::rl_basic_quote_characters.get()).to_vec() };
    let mut level = 1;
    let mut delim = 0u8;
    let mut i = from;
    while i > -1 {
        let c = line[i as usize];
        if delim != 0 && c == delim {
            delim = 0;
        } else if quotes.contains(&c) && c != 0 {
            delim = c;
        } else if delim == 0 && c == closer {
            level += 1;
        } else if delim == 0 && c == opener {
            level -= 1;
        }
        if level == 0 {
            break;
        }
        i -= 1;
    }
    i
}

/// insert-close: insert the bracket; with blink-matching-paren show the
/// matching opener for `timeout_us` or until input arrives.
pub unsafe fn insert_close(count: c_int, key: c_int, timeout_us: c_int) -> c_int {
    // (always inserted, also in overwrite mode)
    if crate::numarg::rl_explicit_arg.get() != 0 || crate::vars::blink_matching_paren.get() == 0 {
        crate::commands::text::insert_chars(&[key as u8], count.max(0) as usize);
        return 0;
    }
    crate::commands::text::insert_chars(&[key as u8], 1);
    redisplay();
    let line = buffer::line().to_vec();
    let point = buffer::point();
    let m = find_matching_open(&line, point as isize - 2, key as u8);
    if m < 0 {
        return 1;
    }
    let orig = buffer::rl_point.get();
    buffer::rl_point.set(m as c_int);
    redisplay();
    crate::input::input_available(timeout_us as i64);
    buffer::rl_point.set(orig);
    0
}

// ---------------------------------------------------------------- chars

#[no_mangle]
pub unsafe extern "C" fn rl_show_char(c: c_int) -> c_int {
    // shown as M-x / C-x (emacs notation), written to rl_outstream
    let mut v = Vec::new();
    let mut b = c as u8;
    if b >= 0x80 && crate::vars::output_meta.get() == 0 {
        v.extend_from_slice(b"M-");
        b &= 0x7f;
    }
    if (b < 0x20 && b != b'\t') || b == 0x7f {
        v.extend_from_slice(b"C-");
        b = if b == 0x7f { b'?' } else { (b | 0x40).to_ascii_uppercase() };
    }
    v.push(b);
    terminal::flush();
    let o = crate::readline::outstream();
    crate::util::fwrite_bytes(o, &v);
    crate::util::fflush(o);
    v.len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rl_character_len(c: c_int, pos: c_int) -> c_int {
    let b = c as u8;
    if b == b'\t' {
        return ((pos | 7) + 1) - pos;
    }
    if b >= 0x80 && crate::vars::output_meta.get() == 0 {
        return 4;
    }
    if b < 0x20 || b == 0x7f {
        return 2;
    }
    1
}
