use super::{validate_domain, Router};
use anyhow::{bail, Context, Result};
use base64::Engine;
use dockon_core::SystemAction;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostsChange {
    pub installation: String,
    pub domains: Vec<String>,
    pub expected_hash: String,
    pub expires: u64,
}
pub(super) struct Prepared {
    action: SystemAction,
    hosts: Option<HostsChange>,
    certificate: Option<Vec<u8>>,
    expires: u64,
}
#[derive(Serialize)]
pub struct PermissionPlan {
    pub token: String,
    pub action: SystemAction,
    pub target: String,
    pub before: String,
    pub after: String,
    pub fingerprint: Option<String>,
    pub warning: String,
    pub supported: bool,
}
fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn hosts_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(system_directory()?.join("drivers/etc/hosts"))
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("/etc/hosts"))
    }
}
#[cfg(windows)]
fn system_directory() -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let size = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if size == 0 || size >= buffer.len() {
        bail!("Windows system directory is unavailable");
    }
    Ok(PathBuf::from(String::from_utf16(&buffer[..size])?))
}

pub fn render_hosts(original: &[u8], installation: &str, domains: &[String]) -> Result<Vec<u8>> {
    uuid::Uuid::parse_str(installation)?;
    if original.len() > 1024 * 1024 || domains.len() > 100 {
        bail!("Hosts data exceeds limits");
    }
    let text = std::str::from_utf8(original)
        .context("Hosts file must be UTF-8; no conversion is performed")?;
    let begin = format!("# dockon {installation} begin");
    let end = format!("# dockon {installation} end");
    let mut inside = false;
    let mut seen = false;
    let mut output = String::new();
    for line in text.split_inclusive('\n') {
        let value = line.trim_end_matches(['\r', '\n']);
        if value == begin {
            if inside || seen {
                bail!("Ambiguous Dockon hosts block");
            }
            inside = true;
            seen = true;
            continue;
        }
        if value == end {
            if !inside {
                bail!("Malformed Dockon hosts block");
            }
            inside = false;
            continue;
        }
        if !inside {
            output.push_str(line);
        }
    }
    if inside {
        bail!("Unclosed Dockon hosts block; repair it explicitly before proceeding");
    }
    let mut requested = BTreeSet::new();
    for domain in domains {
        validate_domain(domain)?;
        if !requested.insert(domain) {
            bail!("Duplicate hosts domain");
        }
    }
    for line in output.lines() {
        let mut fields = line
            .split('#')
            .next()
            .unwrap_or_default()
            .split_whitespace();
        let Some(address) = fields.next() else {
            continue;
        };
        for name in fields {
            if requested
                .iter()
                .any(|domain| domain.eq_ignore_ascii_case(name))
                && address != "127.0.0.1"
            {
                bail!("Domain {name} already maps to {address}; existing hosts entries will not be overwritten");
            }
        }
    }
    if domains.is_empty() {
        return Ok(output.into_bytes());
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(newline);
    }
    output.push_str(&begin);
    output.push_str(newline);
    for domain in domains {
        output.push_str(&format!("127.0.0.1 {domain}{newline}"));
    }
    output.push_str(&end);
    output.push_str(newline);
    Ok(output.into_bytes())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("Permission input exceeds size limit");
    }
    Ok(bytes)
}
fn own_block(bytes: &[u8], installation: &str) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut inside = false;
    let mut block = String::new();
    for line in text.lines() {
        if line == format!("# dockon {installation} begin") {
            inside = true;
        }
        if inside {
            block.push_str(line);
            block.push('\n');
        }
        if line == format!("# dockon {installation} end") {
            inside = false;
        }
    }
    block
}
fn certificate(path: &Path) -> Result<Vec<u8>> {
    let bytes = read_bounded(path, 65536)?;
    let text = std::str::from_utf8(&bytes)?;
    let content = text
        .trim()
        .strip_prefix("-----BEGIN CERTIFICATE-----")
        .and_then(|v| v.strip_suffix("-----END CERTIFICATE-----"))
        .context("Expected a single public PEM certificate")?;
    let content = content
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    Ok(base64::engine::general_purpose::STANDARD.decode(content)?)
}
impl Router {
    pub async fn permission_plan(&self, action: SystemAction) -> Result<PermissionPlan> {
        let expires = now()? + 300;
        let token = uuid::Uuid::new_v4().to_string();
        let (plan, prepared) = match action {
            SystemAction::HostsUpdate | SystemAction::HostsRemove => {
                let state = self.inner.lock().await;
                let settings = state
                    .settings
                    .as_ref()
                    .context("Apply router settings before preparing hosts changes")?;
                let domains = if action == SystemAction::HostsRemove {
                    vec![]
                } else {
                    settings
                        .routes
                        .iter()
                        .filter(|r| !r.domain.ends_with(".localhost"))
                        .map(|r| r.domain.clone())
                        .collect()
                };
                let path = hosts_path()?;
                let bytes = read_bounded(&path, 1024 * 1024)?;
                let installation = std::fs::read_to_string(self.root.join("installation.id"))?;
                let after = render_hosts(&bytes, &installation, &domains)?;
                let change = HostsChange {
                    installation,
                    domains,
                    expected_hash: digest(&bytes),
                    expires,
                };
                (PermissionPlan{token:token.clone(),action,target:path.to_string_lossy().into(),before:own_block(&bytes,&change.installation),after:own_block(&after,&change.installation),fingerprint:None,warning:"Requires an operating-system administrator approval. Only this installation's marked block is replaced. A backup is created beside hosts. Existing conflicting mappings are rejected.".into(),supported:cfg!(windows)||cfg!(target_os="macos")||Path::new("/usr/bin/pkexec").is_file()},Prepared{action,hosts:Some(change),certificate:None,expires})
            }
            SystemAction::TrustCa | SystemAction::UntrustCa => {
                let der = certificate(&self.certificate())?;
                let fingerprint = digest(&der);
                (PermissionPlan{token:token.clone(),action,target:"Current user root certificate trust store".into(),before:if cfg!(windows){format!("Certificate present: {}",platform_trusted(&der)?)}else{"Trust status is not implemented on this platform".into()},after:if action==SystemAction::TrustCa{"Trust this exact Dockon root CA"}else{"Remove this exact Dockon root CA from user trust"}.into(),fingerprint:Some(fingerprint),warning:"A trusted root CA can issue certificates accepted for any domain in this user account. Only approve a CA you control. Firefox and other independent browser stores may need separate configuration. No machine-wide root store is changed.".into(),supported:cfg!(windows)},Prepared{action,hosts:None,certificate:Some(der),expires})
            }
        };
        let mut pending = self.permissions.lock().await;
        pending.retain(|_, v| v.expires > now().unwrap_or(u64::MAX));
        if pending.len() >= 16 {
            bail!("Too many outstanding system permission plans");
        }
        pending.insert(token, prepared);
        Ok(plan)
    }
    pub async fn permission_apply(&self, token: &str) -> Result<()> {
        let prepared = self
            .permissions
            .lock()
            .await
            .remove(token)
            .context("System permission token is unknown or already used")?;
        if now()? > prepared.expires {
            bail!("System permission plan expired; preview it again");
        }
        match prepared.action {
            SystemAction::HostsUpdate | SystemAction::HostsRemove => {
                let change = prepared.hosts.context("Missing hosts change")?;
                if digest(&read_bounded(&hosts_path()?, 1024 * 1024)?) != change.expected_hash {
                    bail!("Hosts file changed since preview; preview it again");
                }
                let directory = self.directory().join("permissions");
                std::fs::create_dir_all(&directory)?;
                let path = directory.join(format!("{}.json", uuid::Uuid::new_v4()));
                let bytes = serde_json::to_vec(&change)?;
                std::fs::write(&path, &bytes)?;
                let hash = digest(&bytes);
                let helper = std::env::current_exe()?
                    .parent()
                    .context("Application directory unavailable")?
                    .join(if cfg!(windows) {
                        "dockon-hosts-helper.exe"
                    } else {
                        "dockon-hosts-helper"
                    });
                if !helper.is_file() {
                    bail!("The scoped dockon-hosts-helper executable is missing");
                }
                tokio::task::spawn_blocking(move || elevate(&helper, &path, &hash)).await??;
                let updated = read_bounded(&hosts_path()?, 1024 * 1024)?;
                if render_hosts(&updated, &change.installation, &change.domains)? != updated {
                    bail!("Hosts change could not be verified");
                }
            }
            SystemAction::TrustCa | SystemAction::UntrustCa => {
                let der = prepared.certificate.context("Missing public certificate")?;
                if certificate(&self.certificate())? != der {
                    bail!("Root CA changed since preview; preview it again");
                }
                let trust = prepared.action == SystemAction::TrustCa;
                platform_set_trust(&der, trust)?;
                if platform_trusted(&der)? != trust {
                    bail!("Certificate trust outcome could not be verified");
                }
            }
        }
        Ok(())
    }
}

// The elevated helper has one target, obtained from the OS. No supplied path,
// command, arbitrary file content or certificate operation can be elevated.
pub fn apply_hosts_plan(path: &Path, expected_plan_hash: &str) -> Result<()> {
    let raw = read_bounded(path, 65536)?;
    if digest(&raw) != expected_plan_hash {
        bail!("Hosts plan integrity check failed");
    }
    let change: HostsChange = serde_json::from_slice(&raw)?;
    if now()? > change.expires || change.expires > now()? + 300 {
        bail!("Hosts plan has expired or has an invalid lifetime");
    }
    let target = hosts_path()?;
    apply_hosts_at(&target, &change)
}
fn apply_hosts_at(target: &Path, change: &HostsChange) -> Result<()> {
    if std::fs::symlink_metadata(target)?.file_type().is_symlink() {
        bail!("Symlinked hosts files are unsupported");
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(target)?;
    file.try_lock_exclusive()?;
    let mut original = vec![];
    (&mut file)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut original)?;
    if digest(&original) != change.expected_hash {
        bail!("Hosts file changed since preview");
    }
    let updated = render_hosts(&original, &change.installation, &change.domains)?;
    let backup = target.with_file_name(format!(
        "hosts.dockon-{}-{}.backup",
        change.installation,
        uuid::Uuid::new_v4()
    ));
    let mut backup_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(backup)?;
    backup_file.write_all(&original)?;
    backup_file.sync_all()?;
    let write = (|| -> Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&updated)?;
        file.set_len(updated.len() as u64)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&original)?;
        file.set_len(original.len() as u64)?;
        file.sync_all()?;
        return Err(error.context("Hosts write failed; original content restored"));
    }
    Ok(())
}

#[cfg(windows)]
fn elevate(helper: &Path, plan: &Path, hash: &str) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{Com::*, Threading::*},
        UI::Shell::*,
    };
    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().chain(Some(0)).collect()
    }
    let file = wide(helper.as_os_str());
    let verb = wide(std::ffi::OsStr::new("runas"));
    let args = wide(std::ffi::OsStr::new(&format!(
        "\"{}\" {hash}",
        plan.display()
    )));
    unsafe {
        let initialized = CoInitializeEx(
            std::ptr::null(),
            COINIT_APARTMENTTHREADED as u32 | COINIT_DISABLE_OLE1DDE as u32,
        );
        if initialized < 0 {
            bail!("Unable to initialize Windows permission dialog: {initialized}");
        }
        let result = (|| -> Result<()> {
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
                lpVerb: verb.as_ptr(),
                lpFile: file.as_ptr(),
                lpParameters: args.as_ptr(),
                nShow: 0,
                ..std::mem::zeroed()
            };
            if ShellExecuteExW(&mut info) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let wait = WaitForSingleObject(info.hProcess, 120000);
            let mut exit = 1;
            let read = GetExitCodeProcess(info.hProcess, &mut exit);
            CloseHandle(info.hProcess);
            if wait != 0 || read == 0 || exit != 0 {
                bail!("Hosts helper did not complete successfully; it may have been cancelled or the hosts file changed");
            }
            Ok(())
        })();
        CoUninitialize();
        result
    }
}
#[cfg(not(windows))]
fn elevate(helper: &Path, plan: &Path, hash: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        fn quote(value: &str) -> String {
            format!("'{}'", value.replace('\'', "'\\''"))
        }
        let shell = format!(
            "{} {} {}",
            quote(&helper.to_string_lossy()),
            quote(&plan.to_string_lossy()),
            quote(hash)
        );
        let script = format!(
            "do shell script {} with administrator privileges",
            serde_json::to_string(&shell)?
        );
        let mut command = std::process::Command::new("/usr/bin/osascript");
        command.args(["-e", &script]);
        command
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = {
        if !Path::new("/usr/bin/pkexec").is_file() {
            bail!("pkexec is unavailable; automated hosts changes are unsupported on this host");
        }
        let mut command = std::process::Command::new("/usr/bin/pkexec");
        command.arg(helper).arg(plan).arg(hash);
        command
    };
    let status = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        bail!("Hosts permission was denied or the scoped helper failed; preview again");
    }
    Ok(())
}

#[cfg(windows)]
struct RootStore(windows_sys::Win32::Security::Cryptography::HCERTSTORE);
#[cfg(windows)]
impl RootStore {
    fn open() -> Result<Self> {
        use windows_sys::Win32::Security::Cryptography::*;
        let name = [82u16, 79, 79, 84, 0];
        let store = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_SYSTEM_W,
                0,
                0,
                CERT_SYSTEM_STORE_CURRENT_USER | CERT_STORE_OPEN_EXISTING_FLAG,
                name.as_ptr().cast(),
            )
        };
        if store.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(store))
    }
}
#[cfg(windows)]
impl Drop for RootStore {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Security::Cryptography::CertCloseStore(self.0, 0);
        }
    }
}
#[cfg(windows)]
fn find(
    store: &RootStore,
    der: &[u8],
) -> Result<*mut windows_sys::Win32::Security::Cryptography::CERT_CONTEXT> {
    use windows_sys::Win32::Security::Cryptography::*;
    let mut hash = sha1::Sha1::digest(der).to_vec();
    let blob = CRYPT_INTEGER_BLOB {
        cbData: hash.len() as u32,
        pbData: hash.as_mut_ptr(),
    };
    let context = unsafe {
        CertFindCertificateInStore(
            store.0,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SHA1_HASH,
            (&blob as *const CRYPT_INTEGER_BLOB).cast(),
            std::ptr::null(),
        )
    };
    if context.is_null()
        && unsafe { windows_sys::Win32::Foundation::GetLastError() }
            != windows_sys::Win32::Foundation::CRYPT_E_NOT_FOUND as u32
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(context)
}
#[cfg(windows)]
fn platform_trusted(der: &[u8]) -> Result<bool> {
    let store = RootStore::open()?;
    let context = find(&store, der)?;
    if context.is_null() {
        return Ok(false);
    }
    let exact = unsafe {
        std::slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize)
            == der
    };
    unsafe {
        windows_sys::Win32::Security::Cryptography::CertFreeCertificateContext(context);
    }
    Ok(exact)
}
#[cfg(windows)]
fn platform_set_trust(der: &[u8], trust: bool) -> Result<()> {
    use windows_sys::Win32::Security::Cryptography::*;
    let store = RootStore::open()?;
    if trust {
        if platform_trusted(der)? {
            return Ok(());
        }
        if unsafe {
            CertAddEncodedCertificateToStore(
                store.0,
                X509_ASN_ENCODING,
                der.as_ptr(),
                der.len() as u32,
                CERT_STORE_ADD_NEW,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    } else {
        let context = find(&store, der)?;
        if !context.is_null() {
            let exact = unsafe {
                std::slice::from_raw_parts(
                    (*context).pbCertEncoded,
                    (*context).cbCertEncoded as usize,
                ) == der
            };
            if !exact {
                unsafe {
                    CertFreeCertificateContext(context);
                }
                bail!("Certificate search returned a different certificate; refusing removal");
            }
            if unsafe { CertDeleteCertificateFromStore(context) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn platform_trusted(_der: &[u8]) -> Result<bool> {
    bail!("User trust-store integration is not yet implemented on this platform")
}
#[cfg(not(windows))]
fn platform_set_trust(_der: &[u8], _trust: bool) -> Result<()> {
    bail!("User trust-store integration is not yet implemented on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_write_backs_up_and_rejects_a_stale_preview() {
        let directory =
            std::env::temp_dir().join(format!("dockon-hosts-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let target = directory.join("hosts");
        let original = b"127.0.0.1 localhost\n# untouched\n";
        std::fs::write(&target, original).unwrap();
        let mut change = HostsChange {
            installation: uuid::Uuid::new_v4().to_string(),
            domains: vec!["private.test".into()],
            expected_hash: digest(original),
            expires: now().unwrap() + 300,
        };
        apply_hosts_at(&target, &change).unwrap();
        let updated = std::fs::read(&target).unwrap();
        assert!(updated.starts_with(original));
        assert!(apply_hosts_at(&target, &change).is_err());
        let backup = std::fs::read_dir(&directory)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "backup"))
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), original);
        change.expected_hash = digest(&updated);
        change.domains.clear();
        apply_hosts_at(&target, &change).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), original);
    }
    #[test]
    fn hosts_changes_preserve_other_entries_and_reject_conflicts() {
        let id = "d9218895-043c-4419-91a6-a0c1ea499737";
        let original = b"# keep\r\n127.0.0.1 localhost\r\n10.1.2.3 shared.internal\r\n";
        let updated = render_hosts(original, id, &["shop.test".into()]).unwrap();
        assert!(updated.starts_with(original));
        assert_eq!(render_hosts(&updated, id, &[]).unwrap(), original);
        assert_eq!(
            render_hosts(&updated, id, &["shop.test".into()]).unwrap(),
            updated
        );
        assert!(render_hosts(original, id, &["shared.internal".into()]).is_err());
        assert!(render_hosts(original, id, &["shop.test\nmalicious".into()]).is_err());
    }
}
