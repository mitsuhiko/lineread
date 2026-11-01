//! Key binding API: translating key sequences, binding/unbinding keys and
//! sequences, looking up functions, invoking key sequences, the dumpers.
//! Owner: bind workstream.
use crate::keymap::{self, current, entry};
use crate::prelude::*;
use crate::util::c_strdup;

#[no_mangle]
pub static rl_binding_keymap: CVar<Keymap> = CVar::new(null_mut());

#[no_mangle]
pub static rl_macro_display_hook: CVar<Option<rl_macro_print_func_t>> = CVar::new(None);

const ESC: usize = 27;

fn is_func(f: Option<rl_command_func_t>, g: rl_command_func_t) -> bool {
    f.map(|f| f as usize) == Some(g as usize)
}

// ---------------------------------------------------------------- keyseq text

/// Translate readline key-sequence notation (`\C-a`, `\M-x`, `\e`, octal,
/// hex, ...) into bytes.  Returns (bytes, ok).
pub fn translate_keyseq_bytes(seq: &[u8]) -> (Vec<u8>, bool) {
    let convert = crate::vars::convert_meta.get() != 0 || crate::vars::force_meta_prefix.get() != 0;
    let mut out = Vec::new();
    let mut i = 0;
    while i < seq.len() {
        let c = seq[i];
        if c != b'\\' || i + 1 >= seq.len() {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let c = seq[i];
        // \C- and \M- prefixes (can be combined in any order)
        if (c == b'C' || c == b'M') && seq.get(i + 1) == Some(&b'-') {
            let mut ctrl = false;
            let mut meta = false;
            while i + 1 < seq.len() && (seq[i] == b'C' || seq[i] == b'M') && seq[i + 1] == b'-' {
                if seq[i] == b'C' {
                    ctrl = true;
                } else {
                    meta = true;
                }
                i += 2;
                if i < seq.len() && seq[i] == b'\\' && i + 2 < seq.len() + 0 && (seq[i + 1] == b'C' || seq[i + 1] == b'M') && seq.get(i + 2) == Some(&b'-') {
                    i += 1;
                } else {
                    break;
                }
            }
            let mut k: u8 = if i < seq.len() {
                let (b, n) = escape_char(seq, i);
                i += n;
                b
            } else {
                // a trailing \C- or \M- applies to NUL
                0
            };
            if ctrl {
                k = if k == b'?' { 0x7f } else { k.to_ascii_uppercase() & 0x1f };
            }
            if meta {
                if convert && i <= seq.len() && !(i == seq.len() && k == 0 && !ctrl) {
                    out.push(0x1b);
                    out.push(k);
                } else {
                    out.push(k | 0x80);
                }
            } else {
                out.push(k);
            }
            continue;
        }
        let (b, n) = escape_after_backslash(seq, i);
        out.push(b);
        i += n;
    }
    (out, true)
}

/// A (possibly backslash-escaped) single character at `seq[i]`.
fn escape_char(seq: &[u8], i: usize) -> (u8, usize) {
    if seq[i] == b'\\' && i + 1 < seq.len() {
        let (b, n) = escape_after_backslash(seq, i + 1);
        (b, n + 1)
    } else {
        (seq[i], 1)
    }
}

/// The escape whose first character (after the backslash) is `seq[i]`.
fn escape_after_backslash(seq: &[u8], i: usize) -> (u8, usize) {
    let c = seq[i];
    match c {
        b'a' => (7, 1),
        b'b' => (8, 1),
        b'd' => (0x7f, 1),
        b'e' => (0x1b, 1),
        b'f' => (0x0c, 1),
        b'n' => (b'\n', 1),
        b'r' => (b'\r', 1),
        b't' => (b'\t', 1),
        b'v' => (0x0b, 1),
        b'0'..=b'7' => {
            let mut v: u32 = 0;
            let mut n = 0;
            while n < 3 && i + n < seq.len() && (b'0'..=b'7').contains(&seq[i + n]) {
                v = v * 8 + (seq[i + n] - b'0') as u32;
                n += 1;
            }
            ((v & 0xff) as u8, n)
        }
        b'x' => {
            let mut v: u32 = 0;
            let mut n = 1;
            while n < 3 && i + n < seq.len() && seq[i + n].is_ascii_hexdigit() {
                v = v * 16 + (seq[i + n] as char).to_digit(16).unwrap();
                n += 1;
            }
            if n == 1 {
                (b'x', 1)
            } else {
                ((v & 0xff) as u8, n)
            }
        }
        _ => (c, 1),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_translate_keyseq(seq: *const c_char, array: *mut c_char, len: *mut c_int) -> c_int {
    let (out, ok) = translate_keyseq_bytes(cstr(seq));
    if !array.is_null() {
        ptr::copy_nonoverlapping(out.as_ptr(), array as *mut u8, out.len());
        *array.add(out.len()) = 0;
    }
    if !len.is_null() {
        *len = out.len() as c_int;
    }
    if ok {
        0
    } else {
        1
    }
}

/// Readline notation for one key (`\C-a`, `\M-x`, `\e`, `\\`, `\"`).
pub fn untranslate_key(mut k: c_int, out: &mut Vec<u8>) {
    if k >= 128 {
        out.extend_from_slice(b"\\M-");
        k -= 128;
    }
    let k = k as u8;
    match k {
        27 => out.extend_from_slice(b"\\e"),
        127 => out.extend_from_slice(b"\\C-?"),
        0..=31 => {
            out.extend_from_slice(b"\\C-");
            out.push((k + 64).to_ascii_lowercase());
        }
        b'\\' | b'"' => {
            out.push(b'\\');
            out.push(k);
        }
        _ => out.push(k),
    }
}

/// Readline notation for a byte string.
pub fn untranslate_keyseq_bytes(seq: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &b in seq {
        untranslate_key(b as c_int, &mut out);
    }
    out
}

#[no_mangle]
pub unsafe extern "C" fn rl_untranslate_keyseq(key: c_int) -> *mut c_char {
    static BUF: CVar<[u8; 32]> = CVar::new([0; 32]);
    let mut v = Vec::new();
    untranslate_key(key & 0xff, &mut v);
    let b = BUF.get_mut();
    let n = v.len().min(31);
    b[..n].copy_from_slice(&v[..n]);
    b[n] = 0;
    b.as_mut_ptr() as *mut c_char
}

// ---------------------------------------------------------------- binding

/// Bind `keys` (raw bytes) in `map` to (type, data).  The workhorse of every
/// binding function.  Returns 0 on success.
pub unsafe fn generic_bind_raw(type_: c_char, keys: &[u8], data: KmData, map: Keymap) -> c_int {
    if keys.is_empty() || map.is_null() {
        if type_ == ISMACR {
            cfree(data.macro_);
        }
        return -1;
    }
    let convert = crate::vars::convert_meta.get() != 0;
    let mut map = map;
    let n = keys.len();
    for (i, &kb) in keys.iter().enumerate() {
        let mut k = kb as usize;
        if k >= 128 && convert && (*entry(map, ESC)).type_ == ISKMAP {
            map = (*entry(map, ESC)).function.map;
            k &= 0x7f;
        }
        let e = entry(map, k);
        if i + 1 < n {
            if (*e).type_ != ISKMAP {
                let old = *e;
                let sub = keymap::new_bare();
                if old.type_ == ISMACR || (old.type_ == ISFUNC && !old.is_null() && !is_func(old.function.func, crate::dispatch::rl_do_lowercase_version)) {
                    *sub.add(ANYOTHERKEY) = old;
                }
                *e = KEYMAP_ENTRY::keymap(sub);
            }
            map = (*e).function.map;
        } else {
            if (*e).type_ == ISKMAP {
                // binding a prefix: the function runs when no longer
                // sequence matches
                let sub = (*e).function.map;
                let a = sub.add(ANYOTHERKEY);
                if (*a).type_ == ISMACR {
                    cfree((*a).function.macro_);
                }
                *a = KEYMAP_ENTRY { type_, function: data };
            } else {
                if (*e).type_ == ISMACR {
                    cfree((*e).function.macro_);
                }
                *e = KEYMAP_ENTRY { type_, function: data };
            }
        }
    }
    rl_binding_keymap.set(map);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_generic_bind(type_: c_int, keyseq: *const c_char, data: *mut c_char, map: Keymap) -> c_int {
    if keyseq.is_null() {
        return -1;
    }
    let (keys, _) = translate_keyseq_bytes(cstr(keyseq));
    let d = KmData { raw: data as *mut c_void };
    generic_bind_raw(type_ as c_char, &keys, d, map)
}

/// Function bound to raw `keys` in `map` (NULL map = current keymap),
/// with the entry type.
pub unsafe fn lookup_raw(keys: &[u8], map: Keymap) -> Option<KEYMAP_ENTRY> {
    let mut map = if map.is_null() { current() } else { map };
    let convert = crate::vars::convert_meta.get() != 0;
    for (i, &kb) in keys.iter().enumerate() {
        let mut k = kb as usize;
        if k >= 128 && convert && (*entry(map, ESC)).type_ == ISKMAP {
            map = (*entry(map, ESC)).function.map;
            k &= 0x7f;
        }
        let e = *entry(map, k);
        if e.type_ == ISKMAP && i + 1 < keys.len() {
            map = e.function.map;
            continue;
        }
        return Some(e);
    }
    None
}

/// Bind if the full sequence is unbound (raw bytes).
pub fn bind_keyseq_if_unbound_raw(keys: &[u8], f: rl_command_func_t, map: Keymap) -> c_int {
    unsafe {
        if let Some(e) = lookup_raw(keys, map) {
            if !(e.is_null() || (e.type_ == ISFUNC && is_func(e.function.func, crate::dispatch::rl_do_lowercase_version))) {
                return 1;
            }
        }
        generic_bind_raw(ISFUNC, keys, KmData { func: Some(f) }, map)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_add_defun(name: *const c_char, f: Option<rl_command_func_t>, key: c_int) -> c_int {
    if key != -1 {
        rl_bind_key(key, f);
    }
    crate::funmap::rl_add_funmap_entry(name, f);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_key_in_map(key: c_int, f: Option<rl_command_func_t>, map: Keymap) -> c_int {
    if !(0..=255).contains(&key) || map.is_null() {
        return -1;
    }
    let mut map = map;
    let mut k = key as usize;
    if k >= 128 && crate::vars::convert_meta.get() != 0 && (*entry(map, ESC)).type_ == ISKMAP {
        map = (*entry(map, ESC)).function.map;
        k &= 0x7f;
    }
    let e = entry(map, k);
    if (*e).type_ == ISMACR {
        cfree((*e).function.macro_);
    }
    *e = KEYMAP_ENTRY { type_: ISFUNC, function: KmData { func: f } };
    rl_binding_keymap.set(map);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_key(key: c_int, f: Option<rl_command_func_t>) -> c_int {
    rl_bind_key_in_map(key, f, current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_key_if_unbound_in_map(key: c_int, f: Option<rl_command_func_t>, map: Keymap) -> c_int {
    if !(0..=255).contains(&key) || map.is_null() {
        return -1;
    }
    let keys = [key as u8];
    if let Some(e) = lookup_raw(&keys, map) {
        if !e.is_null() && !(e.type_ == ISFUNC && is_func(e.function.func, crate::dispatch::rl_do_lowercase_version)) {
            return 1;
        }
    }
    rl_bind_key_in_map(key, f, map)
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_key_if_unbound(key: c_int, f: Option<rl_command_func_t>) -> c_int {
    rl_bind_key_if_unbound_in_map(key, f, current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_unbind_key_in_map(key: c_int, map: Keymap) -> c_int {
    rl_bind_key_in_map(key, None, map)
}

#[no_mangle]
pub unsafe extern "C" fn rl_unbind_key(key: c_int) -> c_int {
    rl_bind_key_in_map(key, None, current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_keyseq_in_map(seq: *const c_char, f: Option<rl_command_func_t>, map: Keymap) -> c_int {
    rl_generic_bind(ISFUNC as c_int, seq, f.map(|f| f as *mut c_char).unwrap_or(null_mut()), map)
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_keyseq(seq: *const c_char, f: Option<rl_command_func_t>) -> c_int {
    rl_bind_keyseq_in_map(seq, f, current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_key(seq: *const c_char, f: Option<rl_command_func_t>, map: Keymap) -> c_int {
    rl_bind_keyseq_in_map(seq, f, map)
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_keyseq_if_unbound_in_map(seq: *const c_char, f: Option<rl_command_func_t>, map: Keymap) -> c_int {
    if seq.is_null() {
        return -1;
    }
    let (keys, _) = translate_keyseq_bytes(cstr(seq));
    if let Some(e) = lookup_raw(&keys, map) {
        if !(e.is_null() || (e.type_ == ISFUNC && is_func(e.function.func, crate::dispatch::rl_do_lowercase_version))) {
            return 1;
        }
    }
    generic_bind_raw(ISFUNC, &keys, KmData { func: f }, map)
}

#[no_mangle]
pub unsafe extern "C" fn rl_bind_keyseq_if_unbound(seq: *const c_char, f: Option<rl_command_func_t>) -> c_int {
    rl_bind_keyseq_if_unbound_in_map(seq, f, current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_macro_bind(seq: *const c_char, macro_: *const c_char, map: Keymap) -> c_int {
    let (m, _) = translate_keyseq_bytes(cstr(macro_));
    rl_generic_bind(ISMACR as c_int, seq, c_strdup(&m), map)
}

#[no_mangle]
pub unsafe extern "C" fn rl_unbind_function_in_map(f: Option<rl_command_func_t>, map: Keymap) -> c_int {
    let Some(f) = f else { return 0 };
    if map.is_null() {
        return 0;
    }
    let mut n = 0;
    for i in 0..KEYMAP_SIZE {
        let e = entry(map, i);
        if (*e).type_ == ISFUNC && is_func((*e).function.func, f) {
            *e = KEYMAP_ENTRY::EMPTY;
            n += 1;
        } else if (*e).type_ == ISKMAP {
            n += rl_unbind_function_in_map(Some(f), (*e).function.map);
        }
    }
    n
}

#[no_mangle]
pub unsafe extern "C" fn rl_unbind_command_in_map(cmd: *const c_char, map: Keymap) -> c_int {
    match crate::funmap::named_function(cstr(cmd)) {
        Some(f) => rl_unbind_function_in_map(Some(f), map),
        None => 0,
    }
}

unsafe fn function_of(keys: &[u8], map: Keymap, type_: *mut c_int) -> Option<rl_command_func_t> {
    let mut map = if map.is_null() { current() } else { map };
    let convert = crate::vars::convert_meta.get() != 0;
    for (i, &kb) in keys.iter().enumerate() {
        let mut k = kb as usize;
        if k >= 128 && convert && (*entry(map, ESC)).type_ == ISKMAP {
            map = (*entry(map, ESC)).function.map;
            k &= 0x7f;
        }
        let e = *entry(map, k);
        if e.type_ == ISKMAP {
            if i + 1 == keys.len() {
                if !type_.is_null() {
                    *type_ = ISKMAP as c_int;
                }
                return core::mem::transmute::<Keymap, Option<rl_command_func_t>>(e.function.map);
            }
            map = e.function.map;
            continue;
        }
        if !type_.is_null() {
            *type_ = e.type_ as c_int;
        }
        return e.function.func;
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn rl_function_of_keyseq(seq: *const c_char, map: Keymap, type_: *mut c_int) -> Option<rl_command_func_t> {
    if seq.is_null() {
        return None;
    }
    function_of(cstr(seq), map, type_)
}

#[no_mangle]
pub unsafe extern "C" fn rl_function_of_keyseq_len(seq: *const c_char, len: usize, map: Keymap, type_: *mut c_int) -> Option<rl_command_func_t> {
    if seq.is_null() {
        return None;
    }
    function_of(core::slice::from_raw_parts(seq as *const u8, len), map, type_)
}

// ---------------------------------------------------------------- stubs

#[no_mangle]
pub unsafe extern "C" fn rl_trim_arg_from_keyseq(_a0: *const c_char, _a1: usize, _a2: Keymap) -> c_int {
    todo_stub!("rl_trim_arg_from_keyseq");
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rl_invoking_keyseqs_in_map(_a0: Option<rl_command_func_t>, _a1: Keymap) -> *mut *mut c_char {
    todo_stub!("rl_invoking_keyseqs_in_map");
    null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn rl_invoking_keyseqs(f: Option<rl_command_func_t>) -> *mut *mut c_char {
    rl_invoking_keyseqs_in_map(f, current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_function_dumper(_a0: c_int) {
    todo_stub!("rl_function_dumper");
}

#[no_mangle]
pub unsafe extern "C" fn rl_macro_dumper(_a0: c_int) {
    todo_stub!("rl_macro_dumper");
}

#[no_mangle]
pub unsafe extern "C" fn rl_print_keybinding(_a0: *const c_char, _a1: Keymap, _a2: c_int) {
    todo_stub!("rl_print_keybinding");
}

#[no_mangle]
pub unsafe extern "C" fn rl_dump_functions(_count: c_int, _key: c_int) -> c_int {
    crate::display::output_before_listing();
    rl_function_dumper(crate::numarg::rl_explicit_arg.get());
    crate::display::after_listing();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_dump_macros(_count: c_int, _key: c_int) -> c_int {
    crate::display::output_before_listing();
    rl_macro_dumper(crate::numarg::rl_explicit_arg.get());
    crate::display::after_listing();
    0
}
