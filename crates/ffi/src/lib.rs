#![allow(clippy::missing_safety_doc)]
//! See the public header and the native integration guide for pointer contracts.
use anyhow::{Result, ensure};
use skald_vcs_client::Repository;
use skald_vcs_core::Settings;
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    ptr,
};

pub const SKALD_OK: i32 = 0;
pub const SKALD_ERROR: i32 = 1;
pub const SKALD_INVALID_ARGUMENT: i32 = 2;
pub const SKALD_PANIC: i32 = 3;
pub const SKALD_CONFLICT: i32 = 4;

pub struct SkaldRepo {
    _private: (),
}

#[derive(Debug)]
struct Argument;
impl std::fmt::Display for Argument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid pointer, flag, or UTF-8 argument")
    }
}
impl std::error::Error for Argument {}

thread_local! { static ERROR: RefCell<CString> = RefCell::new(CString::default()); }

fn set_error(message: String) {
    ERROR.with(|last| last.replace(CString::new(message.replace('\0', "\\0")).unwrap_or_default()));
}

fn call(action: impl FnOnce() -> Result<i32>) -> i32 {
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(code)) => {
            set_error(if code == SKALD_CONFLICT {
                "Merge conflicts require resolution.".into()
            } else {
                String::new()
            });
            code
        }
        Ok(Err(error)) => {
            let code = if error.is::<Argument>() {
                SKALD_INVALID_ARGUMENT
            } else {
                SKALD_ERROR
            };
            set_error(format!("{error:#}"));
            code
        }
        Err(_) => {
            set_error("Skald encountered an internal panic.".into());
            SKALD_PANIC
        }
    }
}

unsafe fn repo<'a>(handle: *const SkaldRepo) -> Result<&'a Repository> {
    unsafe { handle.cast::<Repository>().as_ref() }.ok_or_else(|| Argument.into())
}

unsafe fn string<'a>(value: *const c_char) -> Result<&'a str> {
    ensure!(!value.is_null(), Argument);
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|_| Argument.into())
}

unsafe fn optional<'a>(value: *const c_char) -> Result<Option<&'a str>> {
    if value.is_null() {
        Ok(None)
    } else {
        unsafe { string(value).map(Some) }
    }
}

unsafe fn output<T>(out: *mut *mut T) -> Result<()> {
    ensure!(!out.is_null(), Argument);
    unsafe {
        out.write(ptr::null_mut());
    }
    Ok(())
}

unsafe fn put(out: *mut *mut c_char, value: String) -> Result<()> {
    unsafe {
        out.write(CString::new(value)?.into_raw());
    }
    Ok(())
}

fn flag(value: u8) -> Result<bool> {
    ensure!(value <= 1, Argument);
    Ok(value == 1)
}

#[unsafe(no_mangle)]
pub extern "C" fn skald_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn skald_last_error() -> *const c_char {
    ERROR.with(|error| error.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_repo_free(handle: *mut SkaldRepo) {
    let _ = call(|| {
        if !handle.is_null() {
            unsafe {
                drop(Box::from_raw(handle.cast::<Repository>()));
            }
        }
        Ok(SKALD_OK)
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_string_free(value: *mut c_char) {
    let _ = call(|| {
        if !value.is_null() {
            unsafe {
                drop(CString::from_raw(value));
            }
        }
        Ok(SKALD_OK)
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_init(path: *const c_char, out: *mut *mut SkaldRepo) -> i32 {
    call(|| unsafe {
        output(out)?;
        let repo = Repository::init(string(path)?, Settings::default())?;
        out.write(Box::into_raw(Box::new(repo)).cast());
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_open(path: *const c_char, out: *mut *mut SkaldRepo) -> i32 {
    call(|| unsafe {
        output(out)?;
        let repo = Repository::open(string(path)?)?;
        out.write(Box::into_raw(Box::new(repo)).cast());
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_clone(
    path: *const c_char,
    url: *const c_char,
    token: *const c_char,
    out: *mut *mut SkaldRepo,
) -> i32 {
    call(|| unsafe {
        output(out)?;
        let repo = Repository::clone_remote(
            Path::new(string(path)?),
            string(url)?,
            optional(token)?.unwrap_or(""),
        )?;
        out.write(Box::into_raw(Box::new(repo)).cast());
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_stage(handle: *const SkaldRepo, path: *const c_char) -> i32 {
    call(|| unsafe {
        repo(handle)?.stage(&[string(path)?.to_owned()])?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_unstage(handle: *const SkaldRepo, path: *const c_char) -> i32 {
    call(|| unsafe {
        repo(handle)?.unstage(&[string(path)?.to_owned()])?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_commit(
    handle: *const SkaldRepo,
    author: *const c_char,
    message: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out)?;
        put(
            out,
            repo(handle)?
                .commit(string(author)?, string(message)?)?
                .to_string(),
        )?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_status(handle: *const SkaldRepo, out_json: *mut *mut c_char) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        put(out_json, serde_json::to_string(&repo(handle)?.status()?)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_history(
    handle: *const SkaldRepo,
    revision: *const c_char,
    limit: u32,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        put(
            out_json,
            serde_json::to_string(&repo(handle)?.history(optional(revision)?, limit as usize)?)?,
        )?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_branches(
    handle: *const SkaldRepo,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        put(out_json, serde_json::to_string(&repo(handle)?.branches()?)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_branch(
    handle: *const SkaldRepo,
    name: *const c_char,
    from: *const c_char,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.branch(string(name)?, optional(from)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_branch_delete(
    handle: *const SkaldRepo,
    name: *const c_char,
    force: u8,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.delete_branch(string(name)?, flag(force)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_checkout(
    handle: *const SkaldRepo,
    target: *const c_char,
    detach: u8,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.checkout(string(target)?, flag(detach)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_merge(
    handle: *const SkaldRepo,
    target: *const c_char,
    author: *const c_char,
    message: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        let result = repo(handle)?.merge(string(target)?, string(author)?, string(message)?)?;
        put(out_json, serde_json::to_string(&result)?)?;
        Ok(if result.conflicts.is_empty() {
            SKALD_OK
        } else {
            SKALD_CONFLICT
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_merge_abort(handle: *const SkaldRepo) -> i32 {
    call(|| unsafe {
        repo(handle)?.abort_merge()?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_remote(
    handle: *const SkaldRepo,
    name: *const c_char,
    url: *const c_char,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.remote(string(name)?, optional(url)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_fetch(
    handle: *const SkaldRepo,
    remote: *const c_char,
    token: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        let refs = repo(handle)?.fetch(string(remote)?, optional(token)?.unwrap_or(""))?;
        put(out_json, serde_json::to_string(&refs)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_push(
    handle: *const SkaldRepo,
    remote: *const c_char,
    token: *const c_char,
    name: *const c_char,
    tags: u8,
    delete: u8,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.push(
            string(remote)?,
            optional(token)?.unwrap_or(""),
            optional(name)?,
            flag(tags)?,
            flag(delete)?,
        )?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_pull(
    handle: *const SkaldRepo,
    remote: *const c_char,
    token: *const c_char,
    branch: *const c_char,
    author: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        let result = repo(handle)?.pull(
            string(remote)?,
            optional(token)?.unwrap_or(""),
            optional(branch)?,
            string(author)?,
        )?;
        put(out_json, serde_json::to_string(&result)?)?;
        Ok(if result.conflicts.is_empty() {
            SKALD_OK
        } else {
            SKALD_CONFLICT
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_verify(handle: *const SkaldRepo) -> i32 {
    call(|| unsafe {
        repo(handle)?.verify()?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_restore(
    handle: *const SkaldRepo,
    revision: *const c_char,
    path: *const c_char,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.restore(string(revision)?, &[string(path)?.to_owned()])?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_diff(
    handle: *const SkaldRepo,
    from: *const c_char,
    to: *const c_char,
    staged: u8,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        let diff = repo(handle)?.diff(optional(from)?, optional(to)?, flag(staged)?)?;
        put(out_json, serde_json::to_string(&diff)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_tags(handle: *const SkaldRepo, out_json: *mut *mut c_char) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        put(out_json, serde_json::to_string(&repo(handle)?.tags()?)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_tag(
    handle: *const SkaldRepo,
    name: *const c_char,
    from: *const c_char,
    delete: u8,
) -> i32 {
    call(|| unsafe {
        repo(handle)?.tag(string(name)?, optional(from)?, flag(delete)?)?;
        Ok(SKALD_OK)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skald_remotes(
    handle: *const SkaldRepo,
    out_json: *mut *mut c_char,
) -> i32 {
    call(|| unsafe {
        output(out_json)?;
        put(out_json, serde_json::to_string(&repo(handle)?.remotes()?)?)?;
        Ok(SKALD_OK)
    })
}
