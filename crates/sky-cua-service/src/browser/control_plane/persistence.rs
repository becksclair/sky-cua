use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use serde::{Deserialize, Serialize};
use sky_cua_platform::model::BrowserControlEventKind;

use super::{
    group::{GroupAdmission, GroupRegistry, GroupSnapshot},
    introspection::{EventContext, EventRecorder},
    lease::{LeaseSnapshot, LeaseState},
    operation::{BrowserInstanceId, GroupId, Principal, TabKey},
};

pub(super) const RECOVERY_JOURNAL_VERSION: u32 = 1;
pub(super) const RECOVERY_JOURNAL_FILE: &str = "browser-control-recovery-v1.json";
pub(super) const MAX_JOURNAL_BYTES: u64 = 256 * 1024;
const MAX_GROUPS: usize = 128;
const MAX_MEMBERS_PER_GROUP: usize = 256;
const MAX_ID_BYTES: usize = 1024;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

/// Authority-free restart record. It deliberately excludes lease IDs,
/// expirations, operation identities and payloads, and handoff offers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryGroupHint {
    pub(crate) group_id: GroupId,
    pub(crate) browser_instance_id: BrowserInstanceId,
    pub(crate) principal: Principal,
    pub(crate) members: BTreeSet<TabKey>,
    pub(crate) membership_revision: u64,
    pub(crate) prior_fence: u64,
    pub(crate) unresolved_mutation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryJournal {
    pub(crate) version: u32,
    pub(crate) groups: Vec<RecoveryGroupHint>,
}

#[derive(Debug)]
pub(super) struct JournalLoadFailure {
    pub(super) code: &'static str,
    pub(super) detail: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalWire {
    version: u32,
    groups: Vec<GroupWire>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupWire {
    group_id: String,
    browser_instance_id: String,
    principal: PrincipalWire,
    members: Vec<TabWire>,
    membership_revision: u64,
    prior_fence: u64,
    unresolved_mutation: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalWire {
    id: String,
    uid: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TabWire {
    browser_instance_id: String,
    tab_id: String,
}

impl RecoveryJournal {
    pub(crate) fn empty() -> Self {
        Self {
            version: RECOVERY_JOURNAL_VERSION,
            groups: Vec::new(),
        }
    }

    pub(super) fn capture(groups: &GroupRegistry, unresolved_mutations: &HashSet<GroupId>) -> Self {
        let mut hints = groups
            .all()
            .filter(|group| !matches!(group.admission, GroupAdmission::Released))
            .map(|group| RecoveryGroupHint {
                group_id: group.group_id.clone(),
                browser_instance_id: group.browser_instance_id.clone(),
                principal: group.lease.principal.clone(),
                members: group.members.clone(),
                membership_revision: group.membership_revision,
                prior_fence: group.lease.fence,
                unresolved_mutation: unresolved_mutations.contains(&group.group_id)
                    || matches!(
                        group.admission,
                        GroupAdmission::SettlementPending | GroupAdmission::RecoveryRequired
                    ),
            })
            .collect::<Vec<_>>();
        hints.sort_by(|left, right| left.group_id.cmp(&right.group_id));
        Self {
            version: RECOVERY_JOURNAL_VERSION,
            groups: hints,
        }
    }

    pub(crate) fn restore_suspended(&self) -> GroupRegistry {
        let mut registry = GroupRegistry::default();
        for hint in &self.groups {
            let fence = hint.prior_fence.saturating_add(1);
            registry.insert_recovered(GroupSnapshot {
                group_id: hint.group_id.clone(),
                browser_instance_id: hint.browser_instance_id.clone(),
                members: hint.members.clone(),
                membership_revision: hint.membership_revision,
                lease: LeaseSnapshot {
                    lease_id: format!("recovery-{}-{fence}", hint.group_id),
                    principal: hint.principal.clone(),
                    group_id: hint.group_id.clone(),
                    fence,
                    expires_at_ms: 0,
                    state: LeaseState::Suspended,
                },
                admission: if hint.unresolved_mutation {
                    GroupAdmission::RecoveryRequired
                } else {
                    GroupAdmission::Suspended
                },
            });
        }
        registry
    }

    fn to_wire(&self) -> JournalWire {
        JournalWire {
            version: self.version,
            groups: self
                .groups
                .iter()
                .map(|group| GroupWire {
                    group_id: group.group_id.0.clone(),
                    browser_instance_id: group.browser_instance_id.0.clone(),
                    principal: PrincipalWire {
                        id: group.principal.id.clone(),
                        uid: group.principal.uid,
                    },
                    members: group
                        .members
                        .iter()
                        .map(|tab| TabWire {
                            browser_instance_id: tab.browser_instance_id.0.clone(),
                            tab_id: tab.tab_id.clone(),
                        })
                        .collect(),
                    membership_revision: group.membership_revision,
                    prior_fence: group.prior_fence,
                    unresolved_mutation: group.unresolved_mutation,
                })
                .collect(),
        }
    }

    fn from_wire(wire: JournalWire) -> Result<Self, JournalLoadFailure> {
        if wire.version != RECOVERY_JOURNAL_VERSION {
            return Err(failure(
                "recovery_journal_unknown_version",
                format!("unsupported recovery journal version {}", wire.version),
            ));
        }
        if wire.groups.len() > MAX_GROUPS {
            return Err(failure(
                "recovery_journal_count_exceeded",
                "recovery journal group count exceeds the configured bound",
            ));
        }
        let mut groups = Vec::with_capacity(wire.groups.len());
        let mut group_ids = HashSet::with_capacity(wire.groups.len());
        let mut claimed_tabs = HashSet::new();
        for group in wire.groups {
            validate_id("group_id", &group.group_id)?;
            validate_id("browser_instance_id", &group.browser_instance_id)?;
            validate_id("principal.id", &group.principal.id)?;
            if group.prior_fence == u64::MAX {
                return Err(failure(
                    "recovery_journal_malformed",
                    "recovery journal fence cannot be advanced",
                ));
            }
            if !group_ids.insert(group.group_id.clone()) {
                return Err(failure(
                    "recovery_journal_malformed",
                    "recovery journal contains duplicate group identity",
                ));
            }
            if group.members.len() > MAX_MEMBERS_PER_GROUP {
                return Err(failure(
                    "recovery_journal_count_exceeded",
                    "recovery journal member count exceeds the configured bound",
                ));
            }
            let browser_instance_id = BrowserInstanceId(group.browser_instance_id);
            let mut members = BTreeSet::new();
            for tab in group.members {
                validate_id("member.browser_instance_id", &tab.browser_instance_id)?;
                validate_id("member.tab_id", &tab.tab_id)?;
                if tab.browser_instance_id != browser_instance_id.0 {
                    return Err(failure(
                        "recovery_journal_malformed",
                        "recovery journal member belongs to a different browser identity",
                    ));
                }
                let tab = TabKey::new(tab.browser_instance_id.as_str(), tab.tab_id);
                if !members.insert(tab.clone()) {
                    return Err(failure(
                        "recovery_journal_malformed",
                        "recovery journal contains duplicate tab identity",
                    ));
                }
                if !claimed_tabs.insert(tab) {
                    return Err(failure(
                        "recovery_journal_malformed",
                        "recovery journal assigns one tab to multiple groups",
                    ));
                }
            }
            groups.push(RecoveryGroupHint {
                group_id: GroupId(group.group_id),
                browser_instance_id,
                principal: Principal::new(group.principal.id, group.principal.uid),
                members,
                membership_revision: group.membership_revision,
                prior_fence: group.prior_fence,
                unresolved_mutation: group.unresolved_mutation,
            });
        }
        Ok(Self {
            version: wire.version,
            groups,
        })
    }
}

pub(super) fn load(path: &Path) -> Result<RecoveryJournal, JournalLoadFailure> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RecoveryJournal::empty());
        }
        Err(error) => return Err(io_failure("recovery_journal_read_failed", error)),
    };
    let metadata = file
        .metadata()
        .map_err(|error| io_failure("recovery_journal_read_failed", error))?;
    if metadata.len() > MAX_JOURNAL_BYTES {
        return Err(failure(
            "recovery_journal_oversized",
            "recovery journal exceeds the configured byte bound",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_failure("recovery_journal_read_failed", error))?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(failure(
            "recovery_journal_oversized",
            "recovery journal exceeds the configured byte bound",
        ));
    }
    let wire = serde_json::from_slice(&bytes).map_err(|error| {
        failure(
            "recovery_journal_malformed",
            format!("recovery journal JSON is invalid: {error}"),
        )
    })?;
    RecoveryJournal::from_wire(wire)
}

#[derive(Clone)]
pub(super) struct JournalWriter {
    shared: Arc<WriterShared>,
}

struct WriterShared {
    state: Mutex<WriterState>,
    wake: Condvar,
    flushed: Condvar,
}

struct WriterState {
    pending: Option<(u64, RecoveryJournal)>,
    requested: u64,
    completed: u64,
}

impl JournalWriter {
    pub(super) fn spawn(path: PathBuf, events: EventRecorder) -> Self {
        let shared = Arc::new(WriterShared {
            state: Mutex::new(WriterState {
                pending: None,
                requested: 0,
                completed: 0,
            }),
            wake: Condvar::new(),
            flushed: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("browser-recovery-journal".to_owned())
            .spawn(move || writer_loop(path, events, worker))
            .expect("failed to spawn browser recovery journal writer");
        Self { shared }
    }

    pub(super) fn enqueue(&self, journal: RecoveryJournal) {
        let mut state = self.shared.state.lock().expect("journal writer poisoned");
        state.requested = state.requested.saturating_add(1);
        let sequence = state.requested;
        state.pending = Some((sequence, journal));
        self.shared.wake.notify_one();
    }

    #[cfg(test)]
    pub(super) fn flush(&self) {
        let mut state = self.shared.state.lock().expect("journal writer poisoned");
        let target = state.requested;
        while state.completed < target {
            state = self
                .shared
                .flushed
                .wait(state)
                .expect("journal writer poisoned");
        }
    }
}

fn writer_loop(path: PathBuf, events: EventRecorder, shared: Arc<WriterShared>) {
    loop {
        let (sequence, journal) = {
            let mut state = shared.state.lock().expect("journal writer poisoned");
            while state.pending.is_none() {
                state = shared.wake.wait(state).expect("journal writer poisoned");
            }
            state.pending.take().expect("pending journal exists")
        };
        if let Err(error) = write_atomic(&path, &journal) {
            events.record(
                BrowserControlEventKind::Recovery {
                    state: format!("journal_write_failed:{}", error.kind()),
                },
                EventContext::default(),
            );
        }
        let mut state = shared.state.lock().expect("journal writer poisoned");
        state.completed = state.completed.max(sequence);
        shared.flushed.notify_all();
    }
}

pub(super) fn write_atomic(path: &Path, journal: &RecoveryJournal) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "journal path has no parent"))?;
    ensure_private_dir(parent)?;
    if journal.groups.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => sync_directory(parent)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        return Ok(());
    }
    if journal.groups.len() > MAX_GROUPS
        || journal
            .groups
            .iter()
            .any(|group| group.members.len() > MAX_MEMBERS_PER_GROUP)
    {
        clear_journal(path, parent)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery journal count bound exceeded",
        ));
    }
    let bytes = serde_json::to_vec(&journal.to_wire())?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        clear_journal(path, parent)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery journal byte bound exceeded",
        ));
    }
    let temp = temp_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        set_private_file(path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn ensure_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("journal");
    path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn clear_journal(path: &Path, parent: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_directory(parent),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_id(field: &str, value: &str) -> Result<(), JournalLoadFailure> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(failure(
            "recovery_journal_malformed",
            format!("recovery journal {field} is empty or too long"),
        ));
    }
    Ok(())
}

fn failure(code: &'static str, detail: impl Into<String>) -> JournalLoadFailure {
    JournalLoadFailure {
        code,
        detail: detail.into(),
    }
}

fn io_failure(code: &'static str, error: io::Error) -> JournalLoadFailure {
    failure(code, format!("recovery journal I/O failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "sky-cua-recovery-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ))
            .join(name)
    }

    fn journal() -> RecoveryJournal {
        let browser = BrowserInstanceId::from("browser-a");
        RecoveryJournal {
            version: RECOVERY_JOURNAL_VERSION,
            groups: vec![RecoveryGroupHint {
                group_id: GroupId::from("group-a"),
                browser_instance_id: browser.clone(),
                principal: Principal::new("principal-a", 1000),
                members: BTreeSet::from([TabKey::new(browser, "tab-a")]),
                membership_revision: 3,
                prior_fence: 7,
                unresolved_mutation: true,
            }],
        }
    }

    #[test]
    fn journal_round_trip_is_authority_free_and_atomic() {
        let path = temp_path(RECOVERY_JOURNAL_FILE);
        let expected = journal();
        write_atomic(&path, &expected).unwrap();
        assert_eq!(load(&path).unwrap(), expected);
        let text = fs::read_to_string(&path).unwrap();
        for forbidden in [
            "lease_id",
            "expires_at",
            "operation_id",
            "payload",
            "resume_token",
            "handoff",
        ] {
            assert!(!text.contains(forbidden), "serialized {forbidden}");
        }
        assert!(fs::read_dir(path.parent().unwrap()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        }));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn malformed_unknown_version_and_oversized_journals_fail_closed() {
        let path = temp_path(RECOVERY_JOURNAL_FILE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not-json").unwrap();
        assert_eq!(load(&path).unwrap_err().code, "recovery_journal_malformed");

        fs::write(&path, br#"{"version":99,"groups":[]}"#).unwrap();
        assert_eq!(
            load(&path).unwrap_err().code,
            "recovery_journal_unknown_version"
        );

        fs::write(&path, vec![b' '; MAX_JOURNAL_BYTES as usize + 1]).unwrap();
        assert_eq!(load(&path).unwrap_err().code, "recovery_journal_oversized");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn empty_journal_removes_existing_file() {
        let path = temp_path(RECOVERY_JOURNAL_FILE);
        write_atomic(&path, &journal()).unwrap();
        assert!(path.exists());
        write_atomic(&path, &RecoveryJournal::empty()).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
