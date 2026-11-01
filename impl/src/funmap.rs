//! The function name map (`funmap`): bindable command names.
//!
//! `funmap` is a malloc'd NULL-terminated array of `FUNMAP *`.  Entries added
//! with `rl_add_funmap_entry` before `rl_initialize_funmap` come first; the
//! defaults (funmap_table.rs, the reference's order) are appended once.
//! Lookups by name are case-insensitive and the first entry wins.
use crate::prelude::*;
use crate::util::{fflush, fwrite_bytes, xmalloc, xrealloc};

#[no_mangle]
pub static funmap: CVar<*mut *mut FUNMAP> = CVar::new(null_mut());

static LEN: CVar<usize> = CVar::new(0);
static CAP: CVar<usize> = CVar::new(0);
static INITIALIZED: CVar<bool> = CVar::new(false);

unsafe fn push(name: *const c_char, f: Option<rl_command_func_t>) -> c_int {
    let len = LEN.get();
    if len + 2 > CAP.get() {
        let cap = (CAP.get() * 2).max(256);
        let p = xrealloc(funmap.get() as *mut c_void, cap * core::mem::size_of::<*mut FUNMAP>());
        funmap.set(p as *mut *mut FUNMAP);
        CAP.set(cap);
    }
    let e = xmalloc(core::mem::size_of::<FUNMAP>()) as *mut FUNMAP;
    (*e).name = name;
    (*e).function = f;
    let v = funmap.get();
    *v.add(len) = e;
    *v.add(len + 1) = null_mut();
    LEN.set(len + 1);
    (len + 1) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rl_add_funmap_entry(name: *const c_char, f: Option<rl_command_func_t>) -> c_int {
    // the name is retained, not copied (the caller keeps it alive)
    push(name, f)
}

#[no_mangle]
pub unsafe extern "C" fn rl_initialize_funmap() {
    if INITIALIZED.get() {
        return;
    }
    INITIALIZED.set(true);
    for (name, f) in crate::funmap_table::DEFAULT_FUNMAP {
        push(name.as_ptr(), Some(*f));
    }
}

/// All entries in order (initialises the defaults).
pub fn entries() -> impl Iterator<Item = (&'static [u8], Option<rl_command_func_t>)> {
    unsafe {
        rl_initialize_funmap();
        let v = funmap.get();
        let n = LEN.get();
        (0..n).map(move |i| {
            let e = *v.add(i);
            (cstr((*e).name), (*e).function)
        })
    }
}

/// Function bound to a name (case-insensitive, first entry wins).
pub fn named_function(name: &[u8]) -> Option<rl_command_func_t> {
    entries().find(|(n, _)| n.eq_ignore_ascii_case(name)).and_then(|(_, f)| f)
}

/// First name of a function (what the dumpers print), if any.
pub fn function_name(f: rl_command_func_t) -> Option<&'static [u8]> {
    entries().find(|(_, g)| g.map(|g| g as usize) == Some(f as usize)).map(|(n, _)| n)
}

#[no_mangle]
pub unsafe extern "C" fn rl_named_function(name: *const c_char) -> Option<rl_command_func_t> {
    named_function(cstr_opt(name)?)
}

/// Sorted (strcmp order) names; the array is malloc'd, the strings are
/// the funmap's own (caller frees only the array).
#[no_mangle]
pub unsafe extern "C" fn rl_funmap_names() -> *mut *const c_char {
    rl_initialize_funmap();
    let v = funmap.get();
    let n = LEN.get();
    let mut names: Vec<*const c_char> = (0..n).map(|i| (**v.add(i)).name).collect();
    names.sort_by(|a, b| cstr(*a).cmp(cstr(*b)));
    let out = xmalloc((names.len() + 1) * core::mem::size_of::<*const c_char>()) as *mut *const c_char;
    for (i, p) in names.iter().enumerate() {
        *out.add(i) = *p;
    }
    *out.add(names.len()) = null();
    out
}

#[no_mangle]
pub unsafe extern "C" fn rl_list_funmap_names() {
    let names = rl_funmap_names();
    let out = crate::readline::outstream();
    let mut buf = Vec::new();
    for p in crate::util::strvec_iter(names as *const *mut c_char) {
        buf.extend_from_slice(cstr(p));
        buf.push(b'\n');
    }
    fwrite_bytes(out, &buf);
    fflush(out);
    cfree(names);
}
