use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        Authorization::{
            EXPLICIT_ACCESS_W, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW, SetNamedSecurityInfoW,
            TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(crate) fn enforce_owner_only(path: &Path, container: bool) -> io::Result<()> {
    let mut token: HANDLE = null_mut();
    // SAFETY: The process pseudo-handle is valid and token points to writable storage.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = TokenHandle(token);
    let mut required = 0;
    // SAFETY: A null buffer with zero length is the documented size query.
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
    }
    if required < size_of::<TOKEN_USER>() as u32 {
        return Err(io::Error::last_os_error());
    }
    let words = (required as usize).div_ceil(size_of::<usize>());
    let mut token_information = vec![0_usize; words];
    // SAFETY: The aligned buffer is at least `required` bytes and remains alive below.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            token_information.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: A successful TokenUser query initializes a TOKEN_USER at the buffer start.
    let sid = unsafe {
        token_information
            .as_ptr()
            .cast::<TOKEN_USER>()
            .read()
            .User
            .Sid
    };
    if sid.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process token omitted its user SID",
        ));
    }
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: if container {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            NO_INHERITANCE
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid.cast(),
        },
    };
    let mut acl = null_mut();
    // SAFETY: entry and acl are valid for the duration of the call.
    let status = unsafe { SetEntriesInAclW(1, &entry, null(), &mut acl) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let acl = LocalAllocation(acl.cast());
    let mut wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: wide_path is NUL-terminated, the SID and ACL remain alive, and unused pointers are null.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.0.cast(),
            null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: OpenProcessToken returned this owned handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct LocalAllocation(*mut core::ffi::c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: SetEntriesInAclW allocated this buffer with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}
