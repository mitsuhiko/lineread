//! C interop helpers: C strings as byte slices, libc allocation for memory
//! that crosses the ABI, stdio output, and libc functions the libc crate
//! does not declare.
use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use libc::{size_t, wchar_t, FILE};

/// `wint_t` (int on macOS, unsigned int on glibc: same ABI).
pub type wint_t = u32;

extern "C" {
    // c/shim.c
    pub fn lr_mb_cur_max() -> size_t;
    pub fn lr_stdin() -> *mut FILE;
    pub fn lr_stdout() -> *mut FILE;
    pub fn lr_stderr() -> *mut FILE;
    // libc, not (portably) in the libc crate
    pub fn mbrtowc(pwc: *mut wchar_t, s: *const c_char, n: size_t, ps: *mut MbState) -> size_t;
    pub fn wcrtomb(s: *mut c_char, wc: wchar_t, ps: *mut MbState) -> size_t;
    pub fn wcwidth(wc: wchar_t) -> c_int;
    pub fn iswalnum(wc: wint_t) -> c_int;
    pub fn iswupper(wc: wint_t) -> c_int;
    pub fn iswlower(wc: wint_t) -> c_int;
    pub fn towupper(wc: wint_t) -> wint_t;
    pub fn towlower(wc: wint_t) -> wint_t;
}

/// `mbstate_t` (opaque; 128 bytes on macOS, 8 on glibc).  Zero = initial.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct MbState(pub [u8; 128]);
impl MbState {
    pub const fn new() -> Self {
        MbState([0; 128])
    }
}

/// Literal `const char *` for statics: `cs!("text")`.
#[macro_export]
macro_rules! cs {
    ($s:expr) => {
        concat!($s, "\0").as_ptr() as *const core::ffi::c_char
    };
}
/// Literal `char *` for statics that C may reassign but must never write
/// through or free (e.g. the default of `history_word_delimiters`).
#[macro_export]
macro_rules! cs_mut {
    ($s:expr) => {
        concat!($s, "\0").as_ptr() as *mut core::ffi::c_char
    };
}

/// Marks an unimplemented public function.  Grep for `todo_stub!` to find
/// work.  With `LINEREAD_TRACE_STUBS=1` in the environment every call is
/// logged to stderr.
#[macro_export]
macro_rules! todo_stub {
    ($name:expr) => {
        $crate::util::stub_hit($name)
    };
}

pub fn stub_hit(name: &str) {
    static mut CHECKED: i32 = -1;
    unsafe {
        if CHECKED < 0 {
            CHECKED = (!libc::getenv(c"LINEREAD_TRACE_STUBS".as_ptr()).is_null()) as i32;
        }
        if CHECKED == 1 {
            let msg = format!("lineread: stub {name}\n");
            libc::write(2, msg.as_ptr() as *const c_void, msg.len());
        }
    }
}

/// The bytes of a C string (without the NUL).  NULL gives `None`.
#[inline]
pub unsafe fn cstr_opt<'a>(p: *const c_char) -> Option<&'a [u8]> {
    if p.is_null() {
        None
    } else {
        Some(core::ffi::CStr::from_ptr(p).to_bytes())
    }
}

/// The bytes of a C string; NULL gives the empty slice.
#[inline]
pub unsafe fn cstr<'a>(p: *const c_char) -> &'a [u8] {
    cstr_opt(p).unwrap_or(&[])
}

/// `malloc` that aborts on failure (like xmalloc).
pub unsafe fn xmalloc(n: usize) -> *mut c_void {
    let p = libc::malloc(n.max(1));
    if p.is_null() {
        libc::abort();
    }
    p
}

pub unsafe fn xrealloc(p: *mut c_void, n: usize) -> *mut c_void {
    let q = libc::realloc(p, n.max(1));
    if q.is_null() {
        libc::abort();
    }
    q
}

/// Copy bytes into a fresh NUL-terminated malloc'd C string (caller frees
/// with `free`).
pub unsafe fn c_strdup(b: &[u8]) -> *mut c_char {
    let p = xmalloc(b.len() + 1) as *mut u8;
    ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
    *p.add(b.len()) = 0;
    p as *mut c_char
}

/// `strdup` of a C string, NULL stays NULL.
pub unsafe fn c_strdup_ptr(p: *const c_char) -> *mut c_char {
    match cstr_opt(p) {
        Some(b) => c_strdup(b),
        None => ptr::null_mut(),
    }
}

/// `free` (NULL is fine).
#[inline]
pub unsafe fn cfree<T>(p: *mut T) {
    libc::free(p as *mut c_void)
}

/// A malloc'd NULL-terminated vector of malloc'd strings (caller frees
/// elements and array).  `None` for an empty input when `null_if_empty`.
pub unsafe fn c_strvec(items: &[Vec<u8>]) -> *mut *mut c_char {
    let v = xmalloc((items.len() + 1) * core::mem::size_of::<*mut c_char>()) as *mut *mut c_char;
    for (i, s) in items.iter().enumerate() {
        *v.add(i) = c_strdup(s);
    }
    *v.add(items.len()) = ptr::null_mut();
    v
}

/// Iterate a NULL-terminated `char **`.
pub unsafe fn strvec_iter<'a>(v: *const *mut c_char) -> impl Iterator<Item = *mut c_char> + 'a {
    let mut i = 0usize;
    core::iter::from_fn(move || {
        if v.is_null() {
            return None;
        }
        let p = *v.add(i);
        if p.is_null() {
            None
        } else {
            i += 1;
            Some(p)
        }
    })
}

// ---- stdio output (always through the FILE so it interleaves with the app)

/// Write bytes to a FILE (no flush).
pub unsafe fn fwrite_bytes(f: *mut FILE, b: &[u8]) {
    if !f.is_null() && !b.is_empty() {
        libc::fwrite(b.as_ptr() as *const c_void, 1, b.len(), f);
    }
}

pub unsafe fn fflush(f: *mut FILE) {
    if !f.is_null() {
        libc::fflush(f);
    }
}

/// Simple growable byte buffer with a few formatting conveniences.
pub trait ByteBufExt {
    fn push_str(&mut self, s: &str);
    fn push_int(&mut self, n: i64);
}
impl ByteBufExt for Vec<u8> {
    fn push_str(&mut self, s: &str) {
        self.extend_from_slice(s.as_bytes());
    }
    fn push_int(&mut self, n: i64) {
        self.extend_from_slice(n.to_string().as_bytes());
    }
}

/// Case-insensitive ASCII comparison of byte strings.
pub fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `getenv` as bytes.
pub unsafe fn getenv_bytes(name: &core::ffi::CStr) -> Option<&'static [u8]> {
    cstr_opt(libc::getenv(name.as_ptr()))
}
