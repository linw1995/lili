use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use lili_storage::{ApplicationPaths, JsonDocument, open};
use lili_storage::{
    models::NewPluginEvidence,
    repository::{load_plugin_evidence, save_plugin_evidence},
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    CodexAdapterDiagnostics, ForwardingAck, ForwardingCredentialRecord, ForwardingCredentials,
    ForwardingMessage, ForwardingProtocolError, MAX_FORWARDING_FRAME_BYTES, PlatformEndpoint,
};

const CREDENTIAL_FILE_NAME: &str = "forwarding.json";
const MAX_CREDENTIAL_FILE_BYTES: u64 = 16 * 1024;
const MAX_CODEX_EVIDENCE_BYTES: usize = 64 * 1024;
const CODEX_EVIDENCE_SIGNATURE_DOMAIN: &str = "codex-plugin-diagnostics";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub struct ForwardingConnection {
    stream: Box<dyn AsyncStream>,
}

impl ForwardingConnection {
    pub async fn read_payload(&mut self) -> Result<Vec<u8>, ForwardingTransportError> {
        let mut header = [0_u8; 4];
        self.stream.read_exact(&mut header).await?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 {
            return Err(ForwardingTransportError::EmptyFrame);
        }
        if length > MAX_FORWARDING_FRAME_BYTES {
            return Err(ForwardingTransportError::FrameTooLarge);
        }
        let mut payload = vec![0_u8; length];
        self.stream.read_exact(&mut payload).await?;
        Ok(payload)
    }

    pub async fn write_message(
        &mut self,
        message: &ForwardingMessage,
    ) -> Result<(), ForwardingTransportError> {
        self.write_frame(&message.to_frame()?).await
    }

    pub async fn write_acknowledgement(
        &mut self,
        acknowledgement: &ForwardingAck,
    ) -> Result<(), ForwardingTransportError> {
        self.write_frame(&acknowledgement.to_frame()?).await
    }

    async fn write_frame(&mut self, frame: &[u8]) -> Result<(), ForwardingTransportError> {
        self.stream.write_all(frame).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardingCredentialStore {
    path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexPluginEvidenceStore {
    paths: ApplicationPaths,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticatedCodexPluginEvidence {
    diagnostics: CodexAdapterDiagnostics,
    authentication: String,
}

impl CodexPluginEvidenceStore {
    pub fn for_application(paths: ApplicationPaths) -> Self {
        Self { paths }
    }

    pub fn load(
        &self,
        credentials: &ForwardingCredentials,
    ) -> Result<CodexAdapterDiagnostics, ForwardingTransportError> {
        let mut database = open(&self.paths)
            .map_err(|error| ForwardingTransportError::EvidenceStorage(error.to_string()))?;
        let row = load_plugin_evidence(database.connection())
            .map_err(|error| ForwardingTransportError::EvidenceStorage(error.to_string()))?
            .ok_or_else(|| {
                ForwardingTransportError::EvidenceStorage("evidence record is missing".to_owned())
            })?;
        let evidence: AuthenticatedCodexPluginEvidence =
            serde_json::from_str(row.evidence_json.as_str())
                .map_err(|_| ForwardingTransportError::MalformedEvidenceFile)?;
        let diagnostics_payload = serde_json::to_vec(&evidence.diagnostics)
            .map_err(|_| ForwardingTransportError::MalformedEvidenceFile)?;
        credentials
            .verify_evidence(
                CODEX_EVIDENCE_SIGNATURE_DOMAIN,
                &diagnostics_payload,
                &evidence.authentication,
            )
            .map_err(|_| ForwardingTransportError::UnauthenticatedEvidenceFile)?;
        Ok(evidence.diagnostics)
    }

    pub fn save(
        &self,
        diagnostics: &CodexAdapterDiagnostics,
        credentials: &ForwardingCredentials,
    ) -> Result<(), ForwardingTransportError> {
        let diagnostics_payload = serde_json::to_vec(diagnostics)
            .map_err(|_| ForwardingTransportError::MalformedEvidenceFile)?;
        let authentication = credentials
            .authenticate_evidence(CODEX_EVIDENCE_SIGNATURE_DOMAIN, &diagnostics_payload)?;
        let evidence = AuthenticatedCodexPluginEvidence {
            diagnostics: diagnostics.clone(),
            authentication,
        };
        let payload = serde_json::to_string(&evidence)
            .map_err(|_| ForwardingTransportError::MalformedEvidenceFile)?;
        if payload.len() > MAX_CODEX_EVIDENCE_BYTES {
            return Err(ForwardingTransportError::EvidenceFileTooLarge);
        }
        let evidence_json = JsonDocument::parse(payload)
            .map_err(|_| ForwardingTransportError::MalformedEvidenceFile)?;
        let mut database = open(&self.paths)
            .map_err(|error| ForwardingTransportError::EvidenceStorage(error.to_string()))?;
        save_plugin_evidence(
            database.connection(),
            &NewPluginEvidence {
                id: 1,
                evidence_json: &evidence_json,
                updated_at_ms: unix_time_ms(),
            },
        )
        .map_err(|error| ForwardingTransportError::EvidenceStorage(error.to_string()))?;
        Ok(())
    }
}

fn unix_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(i64::MAX as u128) as i64
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportFault {
    None,
    AfterListenerBind,
    BeforeCredentialReplace,
}

impl ForwardingCredentialStore {
    pub fn for_runtime_dir(runtime_dir: &Path) -> Self {
        Self {
            path: runtime_dir.join(CREDENTIAL_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<ForwardingCredentialRecord, ForwardingTransportError> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ForwardingTransportError::UnsafeCredentialFile);
        }
        validate_private_file(&self.path, &metadata)?;
        if metadata.len() > MAX_CREDENTIAL_FILE_BYTES {
            return Err(ForwardingTransportError::CredentialFileTooLarge);
        }
        let mut payload = Vec::with_capacity(metadata.len() as usize);
        File::open(&self.path)?
            .take(MAX_CREDENTIAL_FILE_BYTES + 1)
            .read_to_end(&mut payload)?;
        if payload.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
            return Err(ForwardingTransportError::CredentialFileTooLarge);
        }
        let record: ForwardingCredentialRecord = serde_json::from_slice(&payload)
            .map_err(|_| ForwardingTransportError::MalformedCredentialFile)?;
        record.credentials()?;
        Ok(record)
    }

    fn save_inner(
        &self,
        record: &ForwardingCredentialRecord,
        fault: TransportFault,
    ) -> Result<(), ForwardingTransportError> {
        record.credentials()?;
        let mut payload = serde_json::to_vec(record)
            .map_err(|_| ForwardingTransportError::MalformedCredentialFile)?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
            return Err(ForwardingTransportError::CredentialFileTooLarge);
        }

        let directory = self
            .path
            .parent()
            .expect("credential path must have a parent");
        ensure_private_runtime_dir(directory)?;
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary =
            directory.join(format!(".forwarding-{}-{sequence}.tmp", std::process::id()));
        let mut guard = TemporaryFileGuard::new(temporary.clone());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(windows)]
        crate::windows_acl::enforce_owner_only(&temporary, false)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        if fault == TransportFault::BeforeCredentialReplace {
            return Err(std::io::Error::other("injected credential replacement failure").into());
        }
        crate::replace_file_atomically(&temporary, &self.path)?;
        guard.commit();
        sync_directory(directory)?;
        Ok(())
    }
}

pub struct BoundForwardingEndpoint {
    listener: PlatformListener,
    credentials: ForwardingCredentials,
    credential_store: ForwardingCredentialStore,
}

impl BoundForwardingEndpoint {
    pub fn bind(runtime_dir: &Path) -> Result<Self, ForwardingTransportError> {
        Self::bind_inner(runtime_dir, TransportFault::None, |_| Ok(()), || Ok(()))
    }

    pub fn bind_with_credentials_rotation<F, R>(
        runtime_dir: &Path,
        pre_publish: F,
        rollback: R,
    ) -> Result<Self, ForwardingTransportError>
    where
        F: FnOnce(&ForwardingCredentials) -> Result<(), ForwardingTransportError>,
        R: FnOnce() -> Result<(), ForwardingTransportError>,
    {
        Self::bind_inner(runtime_dir, TransportFault::None, pre_publish, rollback)
    }

    fn bind_inner<F, R>(
        runtime_dir: &Path,
        fault: TransportFault,
        pre_publish: F,
        rollback: R,
    ) -> Result<Self, ForwardingTransportError>
    where
        F: FnOnce(&ForwardingCredentials) -> Result<(), ForwardingTransportError>,
        R: FnOnce() -> Result<(), ForwardingTransportError>,
    {
        ensure_private_runtime_dir(runtime_dir)?;
        let credentials = ForwardingCredentials::generate()?;
        let listener = PlatformListener::bind(runtime_dir, credentials.instance_id())?;
        if fault == TransportFault::AfterListenerBind {
            return Err(std::io::Error::other("injected listener restart failure").into());
        }
        let credential_store = ForwardingCredentialStore::for_runtime_dir(runtime_dir);
        let record = ForwardingCredentialRecord::new(&credentials, listener.endpoint().clone());
        pre_publish(&credentials)?;
        if let Err(error) = credential_store.save_inner(&record, fault) {
            rollback()?;
            return Err(error);
        }
        Ok(Self {
            listener,
            credentials,
            credential_store,
        })
    }

    #[cfg(all(test, unix))]
    fn bind_with_fault(
        runtime_dir: &Path,
        fault: TransportFault,
    ) -> Result<Self, ForwardingTransportError> {
        Self::bind_inner(runtime_dir, fault, |_| Ok(()), || Ok(()))
    }

    pub fn credentials(&self) -> ForwardingCredentials {
        self.credentials.clone()
    }

    pub fn endpoint(&self) -> &PlatformEndpoint {
        self.listener.endpoint()
    }

    pub fn credential_store(&self) -> &ForwardingCredentialStore {
        &self.credential_store
    }

    pub async fn accept(&self) -> Result<ForwardingConnection, ForwardingTransportError> {
        self.listener.accept().await
    }
}

pub async fn deliver_forwarding_message(
    record: &ForwardingCredentialRecord,
    message: &ForwardingMessage,
) -> Result<ForwardingAck, ForwardingTransportError> {
    let credentials = record.credentials()?;
    if credentials.instance_id() != message_instance_id(message) {
        return Err(ForwardingTransportError::CredentialMismatch);
    }
    let mut connection = connect(record.endpoint()).await?;
    connection.write_message(message).await?;
    let payload = connection.read_payload().await?;
    let acknowledgement = ForwardingAck::from_payload(&payload)?;
    acknowledgement.validate_for(message)?;
    Ok(acknowledgement)
}

fn message_instance_id(message: &ForwardingMessage) -> &str {
    message.instance_id()
}

async fn connect(
    endpoint: &PlatformEndpoint,
) -> Result<ForwardingConnection, ForwardingTransportError> {
    endpoint.validate()?;
    platform_connect(endpoint).await
}

fn ensure_private_runtime_dir(directory: &Path) -> Result<(), ForwardingTransportError> {
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ForwardingTransportError::UnsafeRuntimeDirectory);
    }
    configure_private_directory(directory, &metadata)?;
    Ok(())
}

struct TemporaryFileGuard {
    path: PathBuf,
    committed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn configure_private_directory(
    directory: &Path,
    metadata: &fs::Metadata,
) -> Result<(), ForwardingTransportError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let current_uid = rustix::process::geteuid().as_raw();
    if metadata.uid() != current_uid {
        return Err(ForwardingTransportError::WrongOwner);
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn configure_private_directory(
    directory: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), ForwardingTransportError> {
    crate::windows_acl::enforce_owner_only(directory, true)?;
    Ok(())
}

#[cfg(unix)]
fn validate_private_file(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), ForwardingTransportError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(ForwardingTransportError::WrongOwner);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_private_file(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), ForwardingTransportError> {
    crate::windows_acl::validate_owner_only(path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), std::io::Error> {
    File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(unix)]
fn unix_socket_path(runtime_dir: &Path) -> PathBuf {
    use std::fmt::Write as _;

    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(runtime_dir.to_string_lossy().as_bytes());
    digest.update(rustix::process::geteuid().as_raw().to_be_bytes());
    let mut name = String::from("lili-");
    for byte in digest.finalize() {
        write!(&mut name, "{byte:02x}").expect("writing a socket name cannot fail");
    }
    name.push_str(".sock");
    PathBuf::from("/tmp").join(name)
}

#[cfg(unix)]
struct PlatformListener {
    listener: tokio::net::UnixListener,
    endpoint: PlatformEndpoint,
    expected_uid: u32,
}

#[cfg(unix)]
impl PlatformListener {
    fn bind(runtime_dir: &Path, _instance_id: &str) -> Result<Self, ForwardingTransportError> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

        let path = unix_socket_path(runtime_dir);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.file_type().is_socket()
                    || metadata.uid() != rustix::process::geteuid().as_raw()
                {
                    return Err(ForwardingTransportError::UnsafeEndpoint);
                }
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(ForwardingTransportError::EndpointInUse);
                }
                fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            endpoint: PlatformEndpoint::unix_socket(path)?,
            expected_uid: rustix::process::geteuid().as_raw(),
        })
    }

    fn endpoint(&self) -> &PlatformEndpoint {
        &self.endpoint
    }

    async fn accept(&self) -> Result<ForwardingConnection, ForwardingTransportError> {
        let (stream, _) = self.listener.accept().await?;
        validate_peer_owner(self.expected_uid, stream.peer_cred()?.uid())?;
        Ok(ForwardingConnection {
            stream: Box::new(stream),
        })
    }
}

#[cfg(unix)]
fn validate_peer_owner(expected_uid: u32, actual_uid: u32) -> Result<(), ForwardingTransportError> {
    if expected_uid != actual_uid {
        return Err(ForwardingTransportError::WrongPeerOwner);
    }
    Ok(())
}

#[cfg(unix)]
impl Drop for PlatformListener {
    fn drop(&mut self) {
        if let Some(path) = self.endpoint.unix_path() {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(unix)]
async fn platform_connect(
    endpoint: &PlatformEndpoint,
) -> Result<ForwardingConnection, ForwardingTransportError> {
    let path = endpoint
        .unix_path()
        .ok_or(ForwardingTransportError::UnsupportedEndpoint)?;
    let stream = tokio::net::UnixStream::connect(path).await?;
    Ok(ForwardingConnection {
        stream: Box::new(stream),
    })
}

#[cfg(windows)]
struct PlatformListener {
    endpoint: PlatformEndpoint,
    next: tokio::sync::Mutex<Option<tokio::net::windows::named_pipe::NamedPipeServer>>,
}

#[cfg(windows)]
impl PlatformListener {
    fn bind(_runtime_dir: &Path, instance_id: &str) -> Result<Self, ForwardingTransportError> {
        let name = format!(r"\\.\pipe\lili-{instance_id}");
        let server = create_private_named_pipe(&name, true)?;
        Ok(Self {
            endpoint: PlatformEndpoint::windows_named_pipe(name)?,
            next: tokio::sync::Mutex::new(Some(server)),
        })
    }

    fn endpoint(&self) -> &PlatformEndpoint {
        &self.endpoint
    }

    async fn accept(&self) -> Result<ForwardingConnection, ForwardingTransportError> {
        let name = self
            .endpoint
            .named_pipe()
            .ok_or(ForwardingTransportError::UnsupportedEndpoint)?;
        let server = {
            let mut next = self.next.lock().await;
            let server = next
                .take()
                .ok_or(ForwardingTransportError::EndpointUnavailable)?;
            *next = Some(create_private_named_pipe(name, false)?);
            server
        };
        server.connect().await?;
        Ok(ForwardingConnection {
            stream: Box::new(server),
        })
    }
}

#[cfg(windows)]
fn create_private_named_pipe(
    name: &str,
    first: bool,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer, ForwardingTransportError> {
    use std::{ffi::c_void, mem, ptr};

    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
    };

    let sddl = "D:P(A;;GA;;;OW)\0".encode_utf16().collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // The descriptor is allocated by Windows and remains alive through pipe creation.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .reject_remote_clients(true)
        .first_pipe_instance(first);
    // SECURITY_ATTRIBUTES points to the live descriptor above and is not retained by CreateNamedPipeW.
    let result = unsafe {
        options.create_with_security_attributes_raw(
            name,
            &mut attributes as *mut SECURITY_ATTRIBUTES as *mut c_void,
        )
    };
    // LocalFree is the required release function for the converted descriptor.
    unsafe {
        LocalFree(descriptor);
    }
    Ok(result?)
}

#[cfg(windows)]
async fn platform_connect(
    endpoint: &PlatformEndpoint,
) -> Result<ForwardingConnection, ForwardingTransportError> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let name = endpoint
        .named_pipe()
        .ok_or(ForwardingTransportError::UnsupportedEndpoint)?;
    let stream = ClientOptions::new().open(name)?;
    Ok(ForwardingConnection {
        stream: Box::new(stream),
    })
}

#[cfg(windows)]
pub fn private_forwarding_endpoint_is_live(endpoint: &PlatformEndpoint) -> bool {
    use std::{ffi::c_void, mem, os::windows::io::AsRawHandle, ptr};

    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, IsWellKnownSid,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, WinCreatorOwnerRightsSid,
        },
        Storage::FileSystem::FILE_ALL_ACCESS,
        System::SystemServices::ACCESS_ALLOWED_ACE_TYPE,
    };

    let Some(name) = endpoint.named_pipe() else {
        return false;
    };
    let pipe = match fs::OpenOptions::new().read(true).write(true).open(name) {
        Ok(pipe) => pipe,
        Err(_error) => {
            #[cfg(test)]
            eprintln!("named pipe security check could not open endpoint: {_error}");
            return false;
        }
    };
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    #[cfg(test)]
    eprintln!(
        "named pipe security query status={status} owner={} dacl={} descriptor={}",
        !owner.is_null(),
        !dacl.is_null(),
        !descriptor.is_null()
    );
    if status != 0 || owner.is_null() || dacl.is_null() || descriptor.is_null() {
        return false;
    }
    let mut information = ACL_SIZE_INFORMATION::default();
    let acl_read = unsafe {
        GetAclInformation(
            dacl,
            (&raw mut information).cast::<c_void>(),
            u32::try_from(mem::size_of_val(&information)).expect("ACL information fits in u32"),
            AclSizeInformation,
        )
    };
    let mut raw_ace: *mut c_void = ptr::null_mut();
    let ace_read = acl_read != 0
        && information.AceCount == 1
        && unsafe { GetAce(dacl, 0, &mut raw_ace) } != 0
        && !raw_ace.is_null();
    #[cfg(test)]
    eprintln!(
        "named pipe security ACL read={} aceCount={} aceAvailable={}",
        acl_read != 0,
        information.AceCount,
        !raw_ace.is_null()
    );
    let private = ace_read
        && {
            let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
            let sid = (&raw const ace.SidStart).cast_mut().cast::<c_void>();
            let owner_rights = unsafe { IsWellKnownSid(sid, WinCreatorOwnerRightsSid) } != 0;
            #[cfg(test)]
            eprintln!(
                "named pipe security ACL read={} aceCount={} aceType={} mask={:#x} ownerRights={owner_rights}",
                acl_read != 0,
                information.AceCount,
                ace.Header.AceType,
                ace.Mask
            );
            u32::from(ace.Header.AceType) == ACCESS_ALLOWED_ACE_TYPE
            // CreateNamedPipe maps SDDL `GA` to the file object's concrete full-access mask.
            && ace.Mask == FILE_ALL_ACCESS
            // SDDL `OW` is the Owner Rights well-known SID, which represents the
            // object's owner but is distinct from the owner's account SID.
            && owner_rights
        };
    unsafe {
        LocalFree(descriptor);
    }
    private
}

#[cfg(all(test, windows))]
#[tokio::test]
async fn owner_rights_named_pipe_is_private() {
    let runtime_dir = std::env::temp_dir().join(format!(
        "lili-forwarding-windows-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&runtime_dir);
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();

    assert!(private_forwarding_endpoint_is_live(endpoint.endpoint()));

    drop(endpoint);
    fs::remove_dir_all(runtime_dir).unwrap();
}

#[cfg(not(windows))]
pub fn private_forwarding_endpoint_is_live(_endpoint: &PlatformEndpoint) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
compile_error!("local forwarding transport is unsupported on this platform");

#[derive(Debug, thiserror::Error)]
pub enum ForwardingTransportError {
    #[error("forwarding I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("forwarding protocol failed: {0}")]
    Protocol(#[from] ForwardingProtocolError),
    #[error("forwarding runtime directory is unsafe")]
    UnsafeRuntimeDirectory,
    #[error("forwarding credential file is unsafe")]
    UnsafeCredentialFile,
    #[error("forwarding credential file exceeds 16 KiB")]
    CredentialFileTooLarge,
    #[error("forwarding credential file is malformed")]
    MalformedCredentialFile,
    #[error("Codex plugin evidence record exceeds 64 KiB")]
    EvidenceFileTooLarge,
    #[error("Codex plugin evidence record is malformed")]
    MalformedEvidenceFile,
    #[error("Codex plugin evidence record could not be authenticated")]
    UnauthenticatedEvidenceFile,
    #[error("Codex plugin evidence storage failed: {0}")]
    EvidenceStorage(String),
    #[error("forwarding endpoint is unsafe")]
    UnsafeEndpoint,
    #[error("forwarding endpoint is already in use")]
    EndpointInUse,
    #[error("forwarding endpoint is unavailable")]
    EndpointUnavailable,
    #[error("forwarding endpoint is unsupported on this platform")]
    UnsupportedEndpoint,
    #[error("forwarding object belongs to another user")]
    WrongOwner,
    #[error("forwarding peer belongs to another user")]
    WrongPeerOwner,
    #[error("forwarding message does not match the credential record")]
    CredentialMismatch,
    #[error("forwarding frame is empty")]
    EmptyFrame,
    #[error("forwarding frame exceeds the size limit")]
    FrameTooLarge,
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{
        ForwardingAckDisposition, ProviderCapabilitiesInputV1, ProviderInputV1,
        normalize_provider_input,
    };
    use diesel::prelude::*;

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("lili-forwarding-{}-{sequence}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn event() -> crate::NormalizedSessionEvent {
        normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-1".to_owned()),
            session_id: Some("session-1".to_owned()),
            turn_id: Some("turn-1".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn owner_only_unix_endpoint_delivers_one_acknowledged_frame() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new();
        let endpoint = BoundForwardingEndpoint::bind(&temp.0).unwrap();
        let store = ForwardingCredentialStore::for_runtime_dir(&temp.0);
        let record = store.load().unwrap();
        assert_eq!(
            fs::metadata(&temp.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(endpoint.endpoint().unix_path().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let message = record.credentials().unwrap().sign(event(), 1_000).unwrap();
        let server = async {
            let mut connection = endpoint.accept().await.unwrap();
            let payload = connection.read_payload().await.unwrap();
            let mut verifier = crate::ForwardingVerifier::new(endpoint.credentials());
            let verified = verifier.verify_payload(&payload, 1_001).unwrap();
            connection
                .write_acknowledgement(
                    &verified.acknowledgement(ForwardingAckDisposition::Accepted),
                )
                .await
                .unwrap();
        };
        let client = async {
            let acknowledgement = deliver_forwarding_message(&record, &message).await.unwrap();
            assert_eq!(
                acknowledgement.disposition(),
                ForwardingAckDisposition::Accepted
            );
        };
        tokio::join!(server, client);
    }

    #[tokio::test]
    async fn credential_record_rotates_and_is_retained_on_shutdown() {
        let temp = TempDir::new();
        let store = ForwardingCredentialStore::for_runtime_dir(&temp.0);
        let first_id = {
            let endpoint = BoundForwardingEndpoint::bind(&temp.0).unwrap();
            let id = store.load().unwrap().instance_id().to_owned();
            assert_eq!(id, endpoint.credentials().instance_id());
            id
        };
        assert_eq!(store.load().unwrap().instance_id(), first_id);
        let endpoint = BoundForwardingEndpoint::bind(&temp.0).unwrap();
        assert_ne!(first_id, endpoint.credentials().instance_id());
    }

    #[tokio::test]
    async fn credential_rotation_persists_evidence_before_publishing_the_record() {
        let temp = TempDir::new();
        let runtime_dir = temp.0.join("runtime");
        let bootstrap = BoundForwardingEndpoint::bind(&temp.0.join("bootstrap")).unwrap();
        let previous_credentials = bootstrap.credentials();
        let credential_store = ForwardingCredentialStore::for_runtime_dir(&runtime_dir);
        let previous_record =
            ForwardingCredentialRecord::new(&previous_credentials, bootstrap.endpoint().clone());
        credential_store
            .save_inner(&previous_record, TransportFault::None)
            .unwrap();
        let evidence_store = CodexPluginEvidenceStore::for_application(
            lili_storage::ApplicationPaths::from_root(temp.0.join("app")).unwrap(),
        );
        let diagnostics = CodexAdapterDiagnostics::default();
        evidence_store
            .save(&diagnostics, &previous_credentials)
            .unwrap();

        let endpoint = BoundForwardingEndpoint::bind_with_credentials_rotation(
            &runtime_dir,
            |credentials| {
                assert_eq!(
                    credential_store.load().unwrap().instance_id(),
                    previous_credentials.instance_id()
                );
                evidence_store.save(&diagnostics, credentials)
            },
            || evidence_store.save(&diagnostics, &previous_credentials),
        )
        .unwrap();

        let published = credential_store.load().unwrap().credentials().unwrap();
        assert_eq!(
            published.instance_id(),
            endpoint.credentials().instance_id()
        );
        assert_eq!(evidence_store.load(&published).unwrap(), diagnostics);
    }

    #[tokio::test]
    async fn failed_credential_publication_restores_previous_evidence() {
        let temp = TempDir::new();
        let runtime_dir = temp.0.join("runtime");
        let bootstrap = BoundForwardingEndpoint::bind(&temp.0.join("bootstrap")).unwrap();
        let previous_credentials = bootstrap.credentials();
        let credential_store = ForwardingCredentialStore::for_runtime_dir(&runtime_dir);
        let previous_record =
            ForwardingCredentialRecord::new(&previous_credentials, bootstrap.endpoint().clone());
        credential_store
            .save_inner(&previous_record, TransportFault::None)
            .unwrap();
        let evidence_store = CodexPluginEvidenceStore::for_application(
            lili_storage::ApplicationPaths::from_root(temp.0.join("app")).unwrap(),
        );
        let diagnostics = CodexAdapterDiagnostics::default();
        evidence_store
            .save(&diagnostics, &previous_credentials)
            .unwrap();

        let result = BoundForwardingEndpoint::bind_inner(
            &runtime_dir,
            TransportFault::BeforeCredentialReplace,
            |credentials| evidence_store.save(&diagnostics, credentials),
            || evidence_store.save(&diagnostics, &previous_credentials),
        );

        assert!(result.is_err());
        let retained = credential_store.load().unwrap().credentials().unwrap();
        assert_eq!(retained.instance_id(), previous_credentials.instance_id());
        assert_eq!(evidence_store.load(&retained).unwrap(), diagnostics);
    }

    #[test]
    fn codex_plugin_evidence_round_trips_through_an_owner_only_database() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new();
        let paths = lili_storage::ApplicationPaths::from_root(temp.0.join("app")).unwrap();
        let store = CodexPluginEvidenceStore::for_application(paths.clone());
        let credentials = ForwardingCredentials::generate().unwrap();
        let diagnostics = CodexAdapterDiagnostics::default();
        store.save(&diagnostics, &credentials).unwrap();
        assert_eq!(store.load(&credentials).unwrap(), diagnostics);
        assert_eq!(
            fs::metadata(paths.database_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let mut database = lili_storage::open(&paths).unwrap();
        let evidence = lili_storage::repository::load_plugin_evidence(database.connection())
            .unwrap()
            .unwrap();
        let mut edited: serde_json::Value =
            serde_json::from_str(evidence.evidence_json.as_str()).unwrap();
        edited["diagnostics"]["plugin"]["desktopVersion"] = serde_json::json!("9.9.9");
        let edited =
            lili_storage::JsonDocument::parse(serde_json::to_string(&edited).unwrap()).unwrap();
        diesel::update(lili_storage::schema::plugin_evidence::table.find(1))
            .set(lili_storage::schema::plugin_evidence::evidence_json.eq(edited))
            .execute(database.connection())
            .unwrap();
        drop(database);
        assert!(matches!(
            store.load(&credentials),
            Err(ForwardingTransportError::UnauthenticatedEvidenceFile)
        ));
        store.save(&diagnostics, &credentials).unwrap();
        let unrelated = ForwardingCredentials::generate().unwrap();
        assert!(matches!(
            store.load(&unrelated),
            Err(ForwardingTransportError::UnauthenticatedEvidenceFile)
        ));
    }

    #[tokio::test]
    async fn failure_injection_during_credential_rotation_preserves_previous_record() {
        let temp = TempDir::new();
        let endpoint = BoundForwardingEndpoint::bind(&temp.0).unwrap();
        let store = ForwardingCredentialStore::for_runtime_dir(&temp.0);
        let previous = store.load().unwrap();
        let replacement = ForwardingCredentials::generate().unwrap();
        let replacement =
            ForwardingCredentialRecord::new(&replacement, endpoint.endpoint().clone());

        assert!(
            store
                .save_inner(&replacement, TransportFault::BeforeCredentialReplace)
                .is_err()
        );
        assert_eq!(store.load().unwrap().instance_id(), previous.instance_id());
        assert!(
            fs::read_dir(&temp.0)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
        );
    }

    #[tokio::test]
    async fn failure_injection_during_socket_restart_cleans_partial_listener() {
        let temp = TempDir::new();
        assert!(
            BoundForwardingEndpoint::bind_with_fault(&temp.0, TransportFault::AfterListenerBind)
                .is_err()
        );
        assert!(
            fs::read_dir(&temp.0)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".sock"))
        );

        let endpoint = BoundForwardingEndpoint::bind(&temp.0).unwrap();
        assert!(endpoint.endpoint().unix_path().unwrap().exists());
    }

    #[tokio::test]
    async fn unix_listener_rejects_unsafe_and_live_nodes_but_reclaims_stale_sockets() {
        use std::os::unix::{fs::symlink, net::UnixListener};

        let temp = TempDir::new();
        let socket_path = unix_socket_path(&temp.0);

        fs::write(&socket_path, b"not a socket").unwrap();
        assert!(matches!(
            PlatformListener::bind(&temp.0, "instance"),
            Err(ForwardingTransportError::UnsafeEndpoint)
        ));
        fs::remove_file(&socket_path).unwrap();

        let symlink_target = temp.0.join("target");
        fs::write(&symlink_target, b"target").unwrap();
        symlink(&symlink_target, &socket_path).unwrap();
        assert!(matches!(
            PlatformListener::bind(&temp.0, "instance"),
            Err(ForwardingTransportError::UnsafeEndpoint)
        ));
        fs::remove_file(&socket_path).unwrap();

        let stale = UnixListener::bind(&socket_path).unwrap();
        drop(stale);
        let live = PlatformListener::bind(&temp.0, "instance").unwrap();
        assert!(matches!(
            PlatformListener::bind(&temp.0, "instance"),
            Err(ForwardingTransportError::EndpointInUse)
        ));
        drop(live);
        assert!(!socket_path.exists());
    }

    #[test]
    fn unix_socket_path_is_bounded_for_long_runtime_roots() {
        let runtime_dir = PathBuf::from("/").join("runtime".repeat(256));
        let socket_path = unix_socket_path(&runtime_dir);

        assert_eq!(socket_path.parent(), Some(Path::new("/tmp")));
        assert!(socket_path.to_string_lossy().len() < 100);
    }

    #[test]
    fn mismatched_peer_owner_is_rejected() {
        assert!(validate_peer_owner(1_000, 1_000).is_ok());
        assert!(matches!(
            validate_peer_owner(1_000, 1_001),
            Err(ForwardingTransportError::WrongPeerOwner)
        ));
    }

    #[tokio::test]
    async fn partial_and_oversized_frames_are_rejected_before_payload_processing() {
        use tokio::io::AsyncWriteExt;

        let temp = TempDir::new();
        let endpoint = BoundForwardingEndpoint::bind(&temp.0).unwrap();
        let path = endpoint.endpoint().unix_path().unwrap().to_owned();
        let (accepted_sender, accepted_receiver) = tokio::sync::oneshot::channel();

        let partial_server = async {
            let mut connection = endpoint.accept().await.unwrap();
            accepted_sender.send(()).unwrap();
            assert!(matches!(
                connection.read_payload().await,
                Err(ForwardingTransportError::Io(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof
            ));
        };
        let partial_client = async {
            let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
            accepted_receiver.await.unwrap();
            stream.write_all(&8_u32.to_be_bytes()).await.unwrap();
            stream.write_all(b"half").await.unwrap();
            stream.shutdown().await.unwrap();
        };
        tokio::join!(partial_server, partial_client);

        let (accepted_sender, accepted_receiver) = tokio::sync::oneshot::channel();
        let oversized_server = async {
            let mut connection = endpoint.accept().await.unwrap();
            accepted_sender.send(()).unwrap();
            assert!(matches!(
                connection.read_payload().await,
                Err(ForwardingTransportError::FrameTooLarge)
            ));
        };
        let oversized_client = async {
            let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
            accepted_receiver.await.unwrap();
            stream
                .write_all(&((MAX_FORWARDING_FRAME_BYTES + 1) as u32).to_be_bytes())
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        };
        tokio::join!(oversized_server, oversized_client);
    }
}
