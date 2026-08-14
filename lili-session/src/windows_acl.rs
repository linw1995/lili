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
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            EXPLICIT_ACCESS_W, GetNamedSecurityInfoW, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
            SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, GetTokenInformation, NO_INHERITANCE,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        PSID, SE_DACL_PROTECTED, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::SystemServices::ACCESS_ALLOWED_ACE_TYPE,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(crate) fn enforce_owner_only(path: &Path, container: bool) -> io::Result<()> {
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

pub(crate) fn validate_owner_only(path: &Path) -> io::Result<()> {
    let current_user = CurrentUser::read()?;
    let expected_owner = current_user.sid()?;
    let wide_path = wide_path(path);
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: wide_path is NUL-terminated and all output pointers refer to writable storage.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalAllocation(descriptor);
    if owner.is_null() || dacl.is_null() || unsafe { EqualSid(owner, expected_owner) } == 0 {
        return Err(unsafe_acl_error());
    }

    let mut control = 0;
    let mut revision = 0;
    // SAFETY: descriptor is a valid security descriptor returned by GetNamedSecurityInfoW.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Err(unsafe_acl_error());
    }

    let mut information = ACL_SIZE_INFORMATION::default();
    // SAFETY: dacl is owned by descriptor and information is writable for its full size.
    if unsafe {
        GetAclInformation(
            dacl,
            (&raw mut information).cast::<c_void>(),
            u32::try_from(mem::size_of_val(&information)).expect("ACL information fits in u32"),
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if information.AceCount != 1 {
        return Err(unsafe_acl_error());
    }

    let mut raw_ace: *mut c_void = null_mut();
    // SAFETY: dacl contains one ACE and raw_ace points to writable pointer storage.
    if unsafe { GetAce(dacl, 0, &mut raw_ace) } == 0 || raw_ace.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetAce returned a non-null pointer to the first ACE.
    let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
    let ace_sid = (&raw const ace.SidStart).cast_mut().cast::<c_void>();
    if u32::from(ace.Header.AceType) != ACCESS_ALLOWED_ACE_TYPE
        || ace.Mask != FILE_ALL_ACCESS
        || unsafe { EqualSid(ace_sid, expected_owner) } == 0
    {
        return Err(unsafe_acl_error());
    }
    Ok(())
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
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
        if required < size_of::<TOKEN_USER>() as u32 {
            return Err(io::Error::last_os_error());
        }
        let words = (required as usize).div_ceil(size_of::<usize>());
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

struct LocalAllocation(*mut core::ffi::c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: Windows security APIs return these buffers through LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn owner_only_validation_rejects_a_null_dacl() {
        let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-private-acl-{}-{sequence}.tmp",
            std::process::id()
        ));
        fs::write(&path, b"private").unwrap();
        enforce_owner_only(&path, false).unwrap();
        validate_owner_only(&path).unwrap();

        let mut wide_path = wide_path(&path);
        // SAFETY: wide_path is NUL-terminated and a null DACL is a documented permissive ACL.
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null(),
                null(),
            )
        };
        assert_eq!(status, 0);
        assert_eq!(
            validate_owner_only(&path).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(path).unwrap();
    }
}
