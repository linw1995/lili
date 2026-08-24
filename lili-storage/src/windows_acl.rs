use std::{
    ffi::c_void,
    io, mem,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        Authorization::{
            EXPLICIT_ACCESS_W, GetNamedSecurityInfoW, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
            SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Security::{
        NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    },
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub fn enforce_owner_only(path: &Path, container: bool) -> io::Result<()> {
    validate_owner(path)?;
    let current_user = CurrentUser::read()?;
    let sid = current_user.sid()?;
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
    let mut wide_path = wide_path(path);
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

fn validate_owner(path: &Path) -> io::Result<()> {
    let current_user = CurrentUser::read()?;
    let expected_owner = current_user.sid()?;
    let wide_path = wide_path(path);
    let mut owner: PSID = null_mut();
    let mut descriptor: *mut c_void = null_mut();
    // SAFETY: wide_path is NUL-terminated and all output pointers refer to writable storage.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _descriptor = LocalAllocation(descriptor);
    if owner.is_null() || !same_sid(owner, expected_owner) {
        return Err(unsafe_acl_error());
    }
    Ok(())
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn same_sid(actual: PSID, expected: PSID) -> bool {
    !actual.is_null()
        && !expected.is_null()
        && unsafe { windows_sys::Win32::Security::EqualSid(actual, expected) } != 0
}

fn unsafe_acl_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "file owner or access control list is not private",
    )
}

struct CurrentUser {
    _token: TokenHandle,
    information: Vec<usize>,
}

impl CurrentUser {
    fn read() -> io::Result<Self> {
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
        if required < u32::try_from(mem::size_of::<TOKEN_USER>()).unwrap_or(u32::MAX) {
            return Err(io::Error::last_os_error());
        }
        let words = (required as usize).div_ceil(mem::size_of::<usize>());
        let mut information = vec![0_usize; words];
        // SAFETY: The aligned buffer is at least `required` bytes and remains alive below.
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                information.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            _token: token,
            information,
        })
    }

    fn sid(&self) -> io::Result<PSID> {
        // SAFETY: A successful TokenUser query initialized a TOKEN_USER at the buffer start.
        let sid = unsafe {
            self.information
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
        Ok(sid)
    }
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

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: Windows security APIs return these buffers through LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}
