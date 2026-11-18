use super::{validate_domain, Router};
use anyhow::{bail, Context, Result};
use base64::Engine;
use dockon_core::SystemAction;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
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
    Ok(render_hosts_block(original, installation, domains, false)?.0)
}

fn laragon_domain(line: &str) -> Option<&str> {
    let (entry, comment) = line.split_once('#')?;
    if !comment.trim().eq_ignore_ascii_case("laragon magic!") {
        return None;
    }
    let mut fields = entry.split_whitespace();
    if fields.next()? != "127.0.0.1" {
        return None;
    }
    let domain = fields.next()?;
    fields.next().is_none().then_some(domain)
}

fn suspended_laragon_prefix(installation: &str) -> String {
    format!("# dockon {installation} suspended laragon: ")
}

fn render_hosts_block(
    original: &[u8],
    installation: &str,
    domains: &[String],
    skip_conflicts: bool,
) -> Result<(Vec<u8>, Vec<String>)> {
    uuid::Uuid::parse_str(installation)?;
    if original.len() > 1024 * 1024 || domains.len() > 300 {
        bail!("Hosts data exceeds limits");
    }
    let text = std::str::from_utf8(original)
        .context("Hosts file must be UTF-8; no conversion is performed")?;
    let begin = format!("# dockon {installation} begin");
    let end = format!("# dockon {installation} end");
    let suspended = suspended_laragon_prefix(installation);
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
            let restored = line
                .strip_prefix(&suspended)
                .filter(|entry| laragon_domain(entry).is_some())
                .unwrap_or(line);
            output.push_str(restored);
        }
    }
    if inside {
        bail!("Unclosed Dockon hosts block; repair it explicitly before proceeding");
    }
    let mut requested = BTreeMap::new();
    for entry in domains {
        let (address, domain) = entry.split_once(' ').unwrap_or(("127.0.0.1", entry));
        let ip: std::net::Ipv4Addr = address.parse().context("Invalid local hosts address")?;
        if !ip.is_loopback() {
            bail!("Dockon hosts entries must use IPv4 loopback addresses");
        }
        validate_domain(domain)?;
        if requested.insert(domain, ip).is_some() {
            bail!("Duplicate hosts domain");
        }
    }
    let mut skipped = std::collections::BTreeSet::new();
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
            if let Some((domain, expected)) = requested
                .iter()
                .find(|(domain, _)| domain.eq_ignore_ascii_case(name))
            {
                if address.parse::<std::net::Ipv4Addr>().ok().as_ref() != Some(expected) {
                    if cfg!(windows)
                        && *expected == dockon_core::WINDOWS_WEBMAIL_IP
                        && laragon_domain(line).is_some_and(|host| host == name)
                    {
                        continue;
                    }
                    if !skip_conflicts {
                        bail!("Domain {name} already maps to {address}; existing hosts entries will not be overwritten");
                    }
                    skipped.insert((*domain).to_owned());
                }
            }
        }
    }
    requested.retain(|domain, _| !skipped.contains(*domain));
    let skipped = skipped.into_iter().collect();
    if requested.is_empty() {
        return Ok((output.into_bytes(), skipped));
    }
    if cfg!(windows) {
        let mut preserved = String::with_capacity(output.len());
        for line in output.split_inclusive('\n') {
            if laragon_domain(line).is_some_and(|name| {
                requested.iter().any(|(domain, ip)| {
                    domain.eq_ignore_ascii_case(name) && *ip == dockon_core::WINDOWS_WEBMAIL_IP
                })
            }) {
                preserved.push_str(&suspended);
            }
            preserved.push_str(line);
        }
        output = preserved;
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(newline);
    }
    output.push_str(&begin);
    output.push_str(newline);
    for (domain, address) in requested {
        output.push_str(&format!("{address} {domain}{newline}"));
    }
    output.push_str(&end);
    output.push_str(newline);
    Ok((output.into_bytes(), skipped))
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
fn hosts_preview(bytes: &[u8], installation: &str) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut inside = false;
    let mut block = String::new();
    let suspended = suspended_laragon_prefix(installation);
    for line in text.lines() {
        if line == format!("# dockon {installation} begin") {
            inside = true;
        }
        if inside || line.starts_with(&suspended) {
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
fn select_project_domain(
    settings: &dockon_core::RouterSettings,
    project: &dockon_core::Project,
    projects: &[dockon_core::Project],
    windows_hosts: Option<(&[u8], &str)>,
) -> Result<String> {
    let id = &project.manifest.id;
    let current = settings.routes.iter().find(|route| &route.project_id == id);
    let slug = project
        .manifest
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(id);
    let fallback = id.replace('_', "-");
    let suffix = &settings.domain_suffix;
    let candidates = current
        .into_iter()
        .map(|route| route.domain.clone())
        .chain([
            format!("{}.{suffix}", slug.to_ascii_lowercase()),
            format!("{fallback}.{suffix}"),
        ])
        .chain((1..=100).map(|index| {
            let label = format!("-dockon-{index}");
            let base = fallback
                .get(..fallback.len().min(63 - label.len()))
                .unwrap_or("project")
                .trim_end_matches('-');
            format!("{base}{label}.{suffix}")
        }));
    let route_domains = |domain: &str, mail: bool| {
        let mut names = vec![domain.to_owned()];
        if mail {
            names.extend([format!("mail.{domain}"), format!("webmail.{domain}")]);
        }
        names
    };
    let occupied: std::collections::BTreeSet<_> = settings
        .routes
        .iter()
        .filter(|route| &route.project_id != id)
        .flat_map(|route| {
            route_domains(
                &route.domain,
                projects
                    .iter()
                    .any(|entry| entry.manifest.id == route.project_id && entry.has_mailpit()),
            )
        })
        .collect();
    for candidate in candidates {
        let names = route_domains(&candidate, project.has_mailpit());
        if names
            .iter()
            .any(|name| validate_domain(name).is_err() || occupied.contains(name))
            || (cfg!(windows) && candidate.ends_with(".localhost"))
        {
            continue;
        }
        if let Some((bytes, installation)) = windows_hosts {
            let mut domains = vec![format!("{} {candidate}", dockon_core::WINDOWS_WEBMAIL_IP)];
            if project.has_mailpit() {
                domains.push(format!(
                    "{} webmail.{candidate}",
                    dockon_core::WINDOWS_WEBMAIL_IP
                ));
                domains.push(format!(
                    "{} mail.{candidate}",
                    dockon_core::project_smtp_ip(id)
                ));
            }
            if !render_hosts_block(bytes, installation, &domains, true)?
                .1
                .is_empty()
            {
                continue;
            }
        }
        return Ok(candidate);
    }
    bail!("No free local domain for project {id}")
}

impl Router {
    pub fn project_domain(
        &self,
        settings: &dockon_core::RouterSettings,
        project: &dockon_core::Project,
        projects: &[dockon_core::Project],
    ) -> Result<String> {
        let windows_hosts = if cfg!(windows) {
            Some((
                read_bounded(&hosts_path()?, 1024 * 1024)?,
                std::fs::read_to_string(self.root.join("installation.id"))?,
            ))
        } else {
            None
        };
        select_project_domain(
            settings,
            project,
            projects,
            windows_hosts
                .as_ref()
                .map(|(bytes, installation)| (bytes.as_slice(), installation.as_str())),
        )
    }

    pub fn hosts_pending(
        &self,
        settings: &dockon_core::RouterSettings,
        projects: &[dockon_core::Project],
    ) -> Result<bool> {
        let bytes = read_bounded(&hosts_path()?, 1024 * 1024)?;
        let installation = std::fs::read_to_string(self.root.join("installation.id"))?;
        let domains = super::hosts_entries(settings, projects);
        let (after, _) = render_hosts_block(&bytes, &installation, &domains, true)?;
        Ok(hosts_preview(&bytes, &installation) != hosts_preview(&after, &installation))
    }

    pub(super) fn certificate_trusted(&self) -> bool {
        certificate(&self.certificate())
            .and_then(|der| platform_trusted(&der))
            .unwrap_or(false)
    }
    pub async fn permission_plan(
        &self,
        action: SystemAction,
        projects: &[dockon_core::Project],
    ) -> Result<PermissionPlan> {
        let expires = now()? + 300;
        let token = uuid::Uuid::new_v4().to_string();
        let (plan, prepared) = match action {
            SystemAction::HostsUpdate | SystemAction::HostsRemove => {
                let state = self.inner.lock().await;
                let settings = state
                    .settings
                    .as_ref()
                    .context("Apply router settings before preparing hosts changes")?;
                let mut domains = if action == SystemAction::HostsRemove {
                    vec![]
                } else {
                    super::hosts_entries(settings, projects)
                };
                let path = hosts_path()?;
                let bytes = read_bounded(&path, 1024 * 1024)?;
                let installation = std::fs::read_to_string(self.root.join("installation.id"))?;
                let (after, skipped) = render_hosts_block(&bytes, &installation, &domains, true)?;
                domains.retain(|entry| {
                    let domain = entry
                        .split_once(' ')
                        .map_or(entry.as_str(), |(_, name)| name);
                    !skipped.iter().any(|name| name == domain)
                });
                let mut warning = "Requires operating-system administrator approval. Dockon updates its marked block and creates a hosts backup.".to_owned();
                if String::from_utf8_lossy(&after)
                    .contains(&suspended_laragon_prefix(&installation))
                {
                    warning.push_str(" Matching Laragon-generated loopback entries are suspended while Dockon manages those domains.");
                }
                if !skipped.is_empty() {
                    warning.push_str(&format!(
                        " Existing mappings preserved for: {}.",
                        skipped.join(", ")
                    ));
                }
                let change = HostsChange {
                    installation,
                    domains,
                    expected_hash: digest(&bytes),
                    expires,
                };
                (
                    PermissionPlan {
                        token: token.clone(),
                        action,
                        target: path.to_string_lossy().into(),
                        before: hosts_preview(&bytes, &change.installation),
                        after: hosts_preview(&after, &change.installation),
                        fingerprint: None,
                        warning,
                        supported: cfg!(windows)
                            || cfg!(target_os = "macos")
                            || Path::new("/usr/bin/pkexec").is_file(),
                    },
                    Prepared {
                        action,
                        hosts: Some(change),
                        certificate: None,
                        expires,
                    },
                )
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
    store_trusted(&store, der)
}
#[cfg(windows)]
fn store_trusted(store: &RootStore, der: &[u8]) -> Result<bool> {
    let context = find(store, der)?;
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
    let store = RootStore::open()?;
    store_set_trust(&store, der, trust)
}
#[cfg(windows)]
fn store_set_trust(store: &RootStore, der: &[u8], trust: bool) -> Result<()> {
    use windows_sys::Win32::Security::Cryptography::*;
    if trust {
        if store_trusted(store, der)? {
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
        let context = find(store, der)?;
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
    fn selects_and_keeps_an_available_project_domain_when_hosts_collide() {
        let installation = "d9218895-043c-4419-91a6-a0c1ea499737";
        let project: dockon_core::Project = serde_json::from_value(serde_json::json!({
            "manifest": {
                "schema_version": 1,
                "id": "shop",
                "name": "Shop",
                "root": "www/shop",
                "health": {"type": "http", "path": "/"},
                "services": [{"id": "mailpit", "executable": "mailpit"}]
            },
            "state": "RUNNING",
            "services": []
        }))
        .unwrap();
        let mut settings: dockon_core::RouterSettings = serde_json::from_value(serde_json::json!({
            "runtime_installation": "caddy",
            "http_port": 8080,
            "https_port": 8443,
            "routes": [{"project_id": "shop", "domain": "shop.test", "https": true}]
        }))
        .unwrap();
        let hosts = b"127.0.0.1 shop.test # manual\n127.0.0.1 mail.shop-dockon-1.test\n";
        let choose = |settings: &dockon_core::RouterSettings, hosts: &[u8]| {
            select_project_domain(
                settings,
                &project,
                std::slice::from_ref(&project),
                Some((hosts, installation)),
            )
            .unwrap()
        };
        assert_eq!(choose(&settings, hosts), "shop-dockon-2.test");
        settings.routes[0].domain = "shop-dockon-2.test".into();
        assert_eq!(choose(&settings, b""), "shop-dockon-2.test");
    }
    #[test]
    #[cfg(windows)]
    fn native_certificate_operations_use_an_isolated_memory_store() {
        use windows_sys::Win32::Security::Cryptography::*;
        // This native store is in memory, never the user's or machine's ROOT store.
        let handle = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_MEMORY,
                0,
                0,
                CERT_STORE_CREATE_NEW_FLAG,
                std::ptr::null(),
            )
        };
        assert!(!handle.is_null());
        let store = RootStore(handle);
        let der = include_bytes!("../testdata/public-ca.der");
        assert!(!store_trusted(&store, der).unwrap());
        store_set_trust(&store, der, true).unwrap();
        assert!(store_trusted(&store, der).unwrap());
        store_set_trust(&store, der, true).unwrap();
        store_set_trust(&store, der, false).unwrap();
        assert!(!store_trusted(&store, der).unwrap());
        store_set_trust(&store, der, false).unwrap();
    }
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
        let (updated, skipped) = render_hosts_block(
            original,
            id,
            &[
                "127.77.0.1 shared.internal".into(),
                "127.77.0.1 shop.test".into(),
            ],
            true,
        )
        .unwrap();
        assert_eq!(skipped, ["shared.internal"]);
        assert_eq!(
            updated,
            render_hosts(original, id, &["127.77.0.1 shop.test".into()]).unwrap()
        );
        assert!(render_hosts(original, id, &["shop.test\nmalicious".into()]).is_err());
        let mail = [
            "127.149.7.3 mail.shop.test".into(),
            "127.77.0.1 webmail.shop.test".into(),
        ];
        let mapped = render_hosts(original, id, &mail).unwrap();
        assert!(String::from_utf8_lossy(&mapped).contains("127.149.7.3 mail.shop.test"));
        assert_eq!(render_hosts(&mapped, id, &mail).unwrap(), mapped);
        assert!(render_hosts(original, id, &["192.168.1.2 mail.shop.test".into()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn hosts_update_suspends_only_matching_laragon_records_and_restores_them() {
        let id = "d9218895-043c-4419-91a6-a0c1ea499737";
        let original = b"127.0.0.1 Test.test  # laragon magic!\r\n127.0.0.1 other.test #laragon magic!\r\n127.0.0.1 manual.test # custom\r\n";
        let domains = [
            "127.77.0.1 test.test".into(),
            "127.77.0.1 test2.test".into(),
        ];
        let updated = render_hosts(original, id, &domains).unwrap();
        let preview = hosts_preview(&updated, id);
        assert!(preview.contains("suspended laragon: 127.0.0.1 Test.test"));
        assert!(preview.contains("127.77.0.1 test.test\n127.77.0.1 test2.test"));
        assert!(!preview.contains("other.test"));
        assert_eq!(render_hosts(&updated, id, &domains).unwrap(), updated);
        assert_eq!(render_hosts(&updated, id, &[]).unwrap(), original);
        assert!(render_hosts(original, id, &["127.77.0.1 manual.test".into()]).is_err());
    }
}
