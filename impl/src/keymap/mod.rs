//! Keymaps: the five exported default maps (defaults.rs), creating, copying
//! and freeing keymaps, keymap names, the current keymap.
//!
//! Keymaps handed to C are malloc'd arrays of KEYMAP_SIZE entries.
use crate::prelude::*;
use crate::util::{c_strdup, xmalloc};

pub mod defaults;
pub use defaults::*;

/// The current keymap (`_rl_keymap`).
pub static CURRENT: CVar<Keymap> = CVar::new(emacs_standard_keymap.as_ptr() as Keymap);

#[inline]
pub fn current() -> Keymap {
    CURRENT.get()
}
#[inline]
pub fn emacs_std() -> Keymap {
    emacs_standard_keymap.as_ptr() as Keymap
}
#[inline]
pub fn emacs_meta() -> Keymap {
    emacs_meta_keymap.as_ptr() as Keymap
}
#[inline]
pub fn emacs_ctlx() -> Keymap {
    emacs_ctlx_keymap.as_ptr() as Keymap
}
#[inline]
pub fn vi_movement() -> Keymap {
    vi_movement_keymap.as_ptr() as Keymap
}
#[inline]
pub fn vi_insertion() -> Keymap {
    vi_insertion_keymap.as_ptr() as Keymap
}

/// Entry `k` of keymap `m` (no bounds check beyond debug).
#[inline]
pub unsafe fn entry(m: Keymap, k: usize) -> *mut KEYMAP_ENTRY {
    debug_assert!(k < KEYMAP_SIZE);
    m.add(k)
}

struct NamedMap {
    name: Vec<u8>,
    map: Keymap,
}

/// The name table: the 8 built-in names first, then user names
/// (rl_set_keymap_name).
static NAMES: CVar<Vec<NamedMap>> = CVar::new(Vec::new());
/// Storage for the strings returned by rl_get_keymap_name.
static NAME_CSTRS: CVar<Vec<Vec<u8>>> = CVar::new(Vec::new());
const BUILTIN_NAMES: usize = 8;

fn names() -> &'static mut Vec<NamedMap> {
    let n = NAMES.get_mut();
    if n.is_empty() {
        for (name, map) in [
            (&b"emacs"[..], emacs_std()),
            (b"emacs-standard", emacs_std()),
            (b"emacs-meta", emacs_meta()),
            (b"emacs-ctlx", emacs_ctlx()),
            (b"vi", vi_movement()),
            (b"vi-move", vi_movement()),
            (b"vi-command", vi_movement()),
            (b"vi-insert", vi_insertion()),
        ] {
            n.push(NamedMap { name: name.to_vec(), map });
        }
    }
    n
}

/// Keymap by (case-insensitive) name, NULL if unknown.
pub fn keymap_by_name(name: &[u8]) -> Keymap {
    names()
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case(name))
        .map(|n| n.map)
        .unwrap_or(null_mut())
}

/// First name of a keymap.
pub fn keymap_name(map: Keymap) -> Option<&'static [u8]> {
    names().iter().find(|n| n.map == map).map(|n| &n.name[..])
}

pub fn new_bare() -> Keymap {
    unsafe {
        let m = xmalloc(KEYMAP_SIZE * core::mem::size_of::<KEYMAP_ENTRY>()) as Keymap;
        for i in 0..KEYMAP_SIZE {
            *m.add(i) = KEYMAP_ENTRY::EMPTY;
        }
        m
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_make_bare_keymap() -> Keymap {
    new_bare()
}

#[no_mangle]
pub unsafe extern "C" fn rl_empty_keymap(map: Keymap) -> c_int {
    if map.is_null() {
        return 1;
    }
    for i in 0..ANYOTHERKEY {
        if !(*map.add(i)).is_null() {
            return 0;
        }
    }
    1
}

/// Shallow copy (submaps are shared), like the reference.
#[no_mangle]
pub unsafe extern "C" fn rl_copy_keymap(map: Keymap) -> Keymap {
    let m = new_bare();
    if !map.is_null() {
        for i in 0..KEYMAP_SIZE {
            *m.add(i) = *map.add(i);
        }
    }
    m
}

#[no_mangle]
pub unsafe extern "C" fn rl_make_keymap() -> Keymap {
    let m = new_bare();
    let ins = KEYMAP_ENTRY::func(crate::commands::text::rl_insert);
    *m.add(b'\t' as usize) = ins;
    for i in b' '..=b'~' {
        *m.add(i as usize) = ins;
    }
    for i in 128..256 {
        *m.add(i) = ins;
    }
    let rub = KEYMAP_ENTRY::func(crate::commands::text::rl_rubout);
    *m.add(127) = rub;
    *m.add(8) = rub;
    m
}

/// Free the submaps and macros of `map` (not `map` itself).
#[no_mangle]
pub unsafe extern "C" fn rl_discard_keymap(map: Keymap) {
    if map.is_null() {
        return;
    }
    for i in 0..KEYMAP_SIZE {
        let e = &mut *map.add(i);
        match e.type_ {
            ISKMAP => {
                let sub = e.function.map;
                rl_discard_keymap(sub);
                if !is_static_keymap(sub) {
                    cfree(sub);
                }
            }
            ISMACR => cfree(e.function.macro_),
            _ => {}
        }
        *e = KEYMAP_ENTRY::EMPTY;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_free_keymap(map: Keymap) {
    rl_discard_keymap(map);
    if !is_static_keymap(map) {
        cfree(map);
    }
}

/// One of the five exported arrays (never freed).
pub fn is_static_keymap(m: Keymap) -> bool {
    m == emacs_std() || m == emacs_meta() || m == emacs_ctlx() || m == vi_movement() || m == vi_insertion()
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_keymap() -> Keymap {
    current()
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_keymap(map: Keymap) {
    if !map.is_null() {
        CURRENT.set(map);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_keymap_by_name(name: *const c_char) -> Keymap {
    match cstr_opt(name) {
        Some(n) => keymap_by_name(n),
        None => null_mut(),
    }
}

/// Returned strings stay valid for the life of the process.
unsafe fn name_cstr(name: &[u8]) -> *mut c_char {
    let store = NAME_CSTRS.get_mut();
    if let Some(s) = store.iter_mut().find(|s| &s[..s.len() - 1] == name) {
        return s.as_mut_ptr() as *mut c_char;
    }
    let mut v = name.to_vec();
    v.push(0);
    store.push(v);
    store.last_mut().unwrap().as_mut_ptr() as *mut c_char
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_keymap_name(map: Keymap) -> *mut c_char {
    match keymap_name(map) {
        Some(n) => name_cstr(n),
        None => null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_keymap_name(name: *const c_char, map: Keymap) -> c_int {
    let Some(name) = cstr_opt(name) else { return -1 };
    if map.is_null() {
        return -1;
    }
    let n = names();
    // built-in names and built-in maps cannot be renamed
    for (i, e) in n.iter().enumerate() {
        if i < BUILTIN_NAMES && (e.name.eq_ignore_ascii_case(name) || e.map == map) {
            return -1;
        }
    }
    if let Some(i) = n.iter().position(|e| e.map == map) {
        n[i].name = name.to_vec();
        return i as c_int;
    }
    if let Some(i) = n.iter().position(|e| e.name.eq_ignore_ascii_case(name)) {
        n[i].map = map;
        return i as c_int;
    }
    n.push(NamedMap { name: name.to_vec(), map });
    let _ = c_strdup; // (names are kept in Rust storage)
    (n.len() - 1) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_keymap_from_edit_mode() {
    match crate::readline::rl_editing_mode.get() {
        crate::readline::EMACS_MODE => CURRENT.set(emacs_std()),
        crate::readline::VI_MODE => CURRENT.set(vi_insertion()),
        _ => {}
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_get_keymap_name_from_edit_mode() -> *mut c_char {
    match crate::readline::rl_editing_mode.get() {
        crate::readline::EMACS_MODE => name_cstr(b"emacs"),
        crate::readline::VI_MODE => name_cstr(b"vi"),
        _ => name_cstr(b"none"),
    }
}

/// Key sequences bound at initialisation when still unbound (cursor keys,
/// xterm modified keys, bracketed paste), observed from the reference.
pub fn bind_default_keyseqs() {
    use crate::bind::bind_keyseq_if_unbound_raw as b;
    use crate::commands::movement::*;
    use crate::commands::text::*;
    use crate::histcmds::*;
    use crate::kill::rl_kill_word;
    use crate::search::*;
    type F = rl_command_func_t;
    let arrows: &[(&[u8], F)] = &[
        (b"\x1b[A", rl_get_previous_history),
        (b"\x1b[B", rl_get_next_history),
        (b"\x1b[C", rl_forward_char),
        (b"\x1b[D", rl_backward_char),
        (b"\x1b[H", rl_beg_of_line),
        (b"\x1b[F", rl_end_of_line),
        (b"\x1bOA", rl_get_previous_history),
        (b"\x1bOB", rl_get_next_history),
        (b"\x1bOC", rl_forward_char),
        (b"\x1bOD", rl_backward_char),
        (b"\x1bOH", rl_beg_of_line),
        (b"\x1bOF", rl_end_of_line),
        (b"\x1b[1;5C", rl_forward_word),
        (b"\x1b[1;5D", rl_backward_word),
        (b"\x1b[1;3C", rl_forward_word),
        (b"\x1b[1;3D", rl_backward_word),
        (b"\x1b[2~", rl_overwrite_mode),
        (b"\x1b[3~", rl_delete),
        (b"\x1b[3;5~", rl_kill_word),
        (b"\x1b[5~", rl_history_search_backward),
        (b"\x1b[6~", rl_history_search_forward),
    ];
    for map in [emacs_std(), vi_movement(), vi_insertion()] {
        for (seq, f) in arrows {
            b(seq, *f, map);
        }
    }
    // vi command mode: ESC followed by an unbound key does nothing
    unsafe {
        let esc = *vi_movement().add(27);
        if esc.type_ == ISKMAP {
            let sub = esc.function.map;
            if (*sub.add(ANYOTHERKEY)).is_null() {
                *sub.add(ANYOTHERKEY) = KEYMAP_ENTRY::func(crate::vi::vi_esc_noop);
            }
        }
    }
    // bound whatever enable-bracketed-paste says (it only controls the
    // terminal mode)
    b(b"\x1b[200~", rl_bracketed_paste_begin, emacs_std());
    b(b"\x1b[200~", rl_bracketed_paste_begin, vi_insertion());
}
