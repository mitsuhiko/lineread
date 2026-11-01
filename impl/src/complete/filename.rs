//! Filename and username completion generators.
//! Owner: display & completion workstream.
use super::*;

struct FnState {
    dir: *mut libc::DIR,
    /// directory as typed (prefix of every result)
    users_dirname: Vec<u8>,
    /// the part after the last slash, as matched
    filename: Vec<u8>,
}

static FN_STATE: CVar<FnState> = CVar::new(FnState { dir: null_mut(), users_dirname: Vec::new(), filename: Vec::new() });

/// Expand a leading `~` (tilde library, with a fallback for `~` and
/// `~user` when the tilde library leaves the word alone).
pub fn tilde_expand_bytes(s: &[u8]) -> Vec<u8> {
    if s.first() != Some(&b'~') {
        return s.to_vec();
    }
    unsafe {
        let c = c_strdup(s);
        let r = crate::tilde::tilde_expand(c);
        cfree(c);
        if !r.is_null() {
            let v = cstr(r).to_vec();
            cfree(r);
            if v != s {
                return v;
            }
        }
    }
    // fallback
    let end = s.iter().position(|&c| c == b'/').unwrap_or(s.len());
    let user = &s[1..end];
    let home: Option<Vec<u8>> = unsafe {
        if user.is_empty() {
            crate::util::getenv_bytes(c"HOME").map(|h| h.to_vec())
        } else {
            let cu = c_strdup(user);
            let pw = libc::getpwnam(cu);
            cfree(cu);
            if pw.is_null() {
                None
            } else {
                Some(cstr((*pw).pw_dir).to_vec())
            }
        }
    };
    match home {
        Some(mut h) => {
            h.extend_from_slice(&s[end..]);
            h
        }
        None => s.to_vec(),
    }
}

unsafe fn close_dir() {
    let st = FN_STATE.get_mut();
    if !st.dir.is_null() {
        libc::closedir(st.dir);
        st.dir = null_mut();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_filename_completion_function(text: *const c_char, state: c_int) -> *mut c_char {
    if state == 0 {
        close_dir();
        let text = cstr(text).to_vec();
        let (mut dirname, mut filename, mut users_dirname);
        match text.iter().rposition(|&c| c == b'/') {
            Some(i) => {
                dirname = text[..=i].to_vec();
                filename = text[i + 1..].to_vec();
                users_dirname = dirname.clone();
            }
            None => {
                dirname = b".".to_vec();
                filename = text.clone();
                users_dirname = Vec::new();
            }
        }
        if dirname.first() == Some(&b'~') {
            dirname = tilde_expand_bytes(&dirname);
            if crate::vars::expand_tilde.get() != 0 {
                users_dirname = dirname.clone();
            }
        }
        if let Some(h) = rl_directory_rewrite_hook.get() {
            dirname = call_icpp(h, &dirname).1;
        } else if let Some(h) = rl_directory_completion_hook.get() {
            let (r, d) = call_icpp(h, &dirname);
            dirname = d;
            if r != 0 {
                users_dirname = dirname.clone();
            }
        } else if rl_completion_found_quote.get() != 0 {
            if let Some(f) = rl_filename_dequoting_function.get() {
                if let Some(d) = call_dequote(f, &dirname, rl_completion_quote_character.get()) {
                    if !users_dirname.is_empty() {
                        users_dirname = d.clone();
                    }
                    dirname = d;
                }
            }
        }
        if rl_completion_found_quote.get() != 0 {
            if let Some(f) = rl_filename_dequoting_function.get() {
                if let Some(d) = call_dequote(f, &filename, rl_completion_quote_character.get()) {
                    filename = d;
                }
            }
        }
        if !filename.is_empty() {
            if let Some(h) = rl_completion_rewrite_hook.get() {
                if let Some(d) = call_dequote(h, &filename, filename.len() as c_int) {
                    filename = d;
                }
            }
        }
        let cd = c_strdup(&dirname);
        let dir = libc::opendir(cd);
        cfree(cd);
        let st = FN_STATE.get_mut();
        st.dir = dir;
        if dirname != b"." && !users_dirname.is_empty() && users_dirname.last() != Some(&b'/') {
            users_dirname.push(b'/');
        }
        st.users_dirname = users_dirname;
        st.filename = filename;
        rl_filename_completion_desired.set(1);
    }
    let st = FN_STATE.get_mut();
    if st.dir.is_null() {
        return null_mut();
    }
    let hidden_ok = crate::vars::match_hidden_files.get() != 0;
    loop {
        let st = FN_STATE.get_mut();
        let ent = libc::readdir(st.dir);
        if ent.is_null() {
            close_dir();
            return null_mut();
        }
        let dname = core::ffi::CStr::from_ptr((*ent).d_name.as_ptr()).to_bytes().to_vec();
        let name = match rl_filename_rewrite_hook.get() {
            Some(h) => {
                let p = c_strdup(&dname);
                let r = h(p, dname.len() as c_int);
                let v = if r.is_null() { dname.clone() } else { cstr(r).to_vec() };
                if !r.is_null() && r != p {
                    cfree(r);
                }
                cfree(p);
                v
            }
            None => dname.clone(),
        };
        let st = FN_STATE.get_mut();
        let fl = &st.filename;
        let ok = if fl.is_empty() {
            !(name == b"." || name == b".." || (!hidden_ok && name.first() == Some(&b'.')))
        } else {
            let hidden_skip = !hidden_ok && name.first() == Some(&b'.') && fl.first() != Some(&b'.');
            !hidden_skip && prefix_matches(fl, &name)
        };
        if !ok {
            continue;
        }
        let mut r = st.users_dirname.clone();
        r.extend_from_slice(&name);
        return c_strdup(&r);
    }
}

struct UserState {
    prefix: Vec<u8>,
    tilde: bool,
}
static USER_STATE: CVar<UserState> = CVar::new(UserState { prefix: Vec::new(), tilde: false });

#[no_mangle]
pub unsafe extern "C" fn rl_username_completion_function(text: *const c_char, state: c_int) -> *mut c_char {
    if state == 0 {
        let t = cstr(text);
        let tilde = t.first() == Some(&b'~');
        let st = USER_STATE.get_mut();
        st.tilde = tilde;
        st.prefix = if tilde { t[1..].to_vec() } else { t.to_vec() };
        libc::setpwent();
    }
    loop {
        let pw = libc::getpwent();
        if pw.is_null() {
            libc::endpwent();
            return null_mut();
        }
        let name = cstr((*pw).pw_name);
        let st = USER_STATE.get_mut();
        if name.starts_with(&st.prefix) {
            let mut v = Vec::new();
            if st.tilde {
                v.push(b'~');
                rl_filename_completion_desired.set(1);
            }
            v.extend_from_slice(name);
            return c_strdup(&v);
        }
    }
}
