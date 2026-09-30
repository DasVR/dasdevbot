//! Current-user SIDs and owner-only DACLs. Windows only.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    EqualSid, GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL,
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// The TOKEN_USER of a process. The SID points into this buffer.
pub(crate) struct UserSid {
    // u64 keeps the TOKEN_USER pointer field aligned.
    buf: Vec<u64>,
}

impl UserSid {
    fn psid(&self) -> PSID {
        // SAFETY: buf was filled by GetTokenInformation(TokenUser), so it
        // starts with a TOKEN_USER whose Sid points inside buf.
        unsafe { (*(self.buf.as_ptr() as *const TOKEN_USER)).User.Sid }
    }

    pub(crate) fn equals(&self, other: &UserSid) -> bool {
        // SAFETY: both SIDs are valid for the life of their buffers.
        unsafe { EqualSid(self.psid(), other.psid()).is_ok() }
    }
}

/// The user this process runs as.
pub(crate) fn current_user() -> io::Result<UserSid> {
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no close.
    token_user_of(unsafe { GetCurrentProcess() })
}

/// The user a process runs as, by pid.
pub(crate) fn process_user(pid: u32) -> io::Result<UserSid> {
    // SAFETY: OpenProcess has no memory preconditions. The handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(io::Error::other)?;
    let user = token_user_of(process);
    // SAFETY: process is a handle this function owns.
    let _ = unsafe { CloseHandle(process) };
    user
}

fn token_user_of(process: HANDLE) -> io::Result<UserSid> {
    let mut token = HANDLE::default();
    // SAFETY: token is a valid out pointer. The handle is closed below.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::other)?;
    let user = token_user(token);
    // SAFETY: token is a handle this function owns.
    let _ = unsafe { CloseHandle(token) };
    user
}

fn token_user(token: HANDLE) -> io::Result<UserSid> {
    let mut len = 0u32;
    // SAFETY: a size query with no buffer. It fails with the needed length.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: buf holds at least len bytes and outlives the call.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
    }
    .map_err(io::Error::other)?;
    Ok(UserSid { buf })
}

/// A self-relative security descriptor from LocalAlloc.
pub(crate) struct Descriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is immutable after creation and freed once, on drop.
unsafe impl Send for Descriptor {}
unsafe impl Sync for Descriptor {}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: the pointer came from LocalAlloc and is freed only here.
        unsafe { LocalFree(Some(HLOCAL(self.0 .0))) };
    }
}

impl Descriptor {
    /// SECURITY_ATTRIBUTES pointing at this descriptor. Not inheritable.
    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0 .0,
            bInheritHandle: false.into(),
        }
    }

    fn dacl(&self) -> io::Result<*mut ACL> {
        let mut present = false.into();
        let mut defaulted = false.into();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: the descriptor is valid and the out pointers are locals.
        unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) }
            .map_err(io::Error::other)?;
        if !present.as_bool() || dacl.is_null() {
            return Err(io::Error::other("owner-only descriptor has no dacl"));
        }
        Ok(dacl)
    }
}

/// Protected DACL granting GENERIC_ALL to `user` only.
pub(crate) fn owner_only(user: &UserSid) -> io::Result<Descriptor> {
    let mut text = PWSTR::null();
    // SAFETY: the SID is valid. text is LocalFree'd below.
    unsafe { ConvertSidToStringSidW(user.psid(), &mut text) }.map_err(io::Error::other)?;
    // SAFETY: text is a NUL-terminated string from ConvertSidToStringSidW.
    let sid = unsafe { text.to_string() };
    // SAFETY: text came from LocalAlloc and is not used after this.
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    let sid = sid.map_err(io::Error::other)?;
    let sddl = wide(format!("D:P(A;;GA;;;{sid})").as_ref());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: sddl is NUL-terminated and outlives the call. The result is
    // owned by Descriptor, which LocalFrees it.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(io::Error::other)?;
    Ok(Descriptor(descriptor))
}

/// Replace the file DACL with the current user only, dropping inherited ACEs.
pub(crate) fn protect_file(path: &Path) -> io::Result<()> {
    let descriptor = owner_only(&current_user()?)?;
    let dacl = descriptor.dacl()?;
    let name = wide(path.as_os_str());
    // SAFETY: name is NUL-terminated and dacl lives inside descriptor, both
    // alive for the call.
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        )
    };
    status.ok().map_err(io::Error::other)
}

pub(crate) fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}
