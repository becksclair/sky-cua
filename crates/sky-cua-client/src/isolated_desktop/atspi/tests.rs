use std::ffi::OsStr;

use super::*;

#[derive(Debug)]
struct FakeOps {
    launcher_owner: Option<u32>,
    registry_owner: Option<u32>,
    activation_succeeds: bool,
    spawned_launcher: usize,
    spawned_registry: usize,
    terminated: Vec<u32>,
    terminated_records: Vec<u32>,
    validations: Vec<(u32, Option<String>)>,
    probed: bool,
    fail_probe: bool,
    fail_registry_owner_query: bool,
}

impl FakeOps {
    fn new(activation_succeeds: bool) -> Self {
        Self {
            launcher_owner: None,
            registry_owner: None,
            activation_succeeds,
            spawned_launcher: 0,
            spawned_registry: 0,
            terminated: Vec::new(),
            terminated_records: Vec::new(),
            validations: Vec::new(),
            probed: false,
            fail_probe: false,
            fail_registry_owner_query: false,
        }
    }
}

impl AtspiOps for FakeOps {
    fn owner_pid(&mut self, _bus_address: &str, name: &str) -> Result<Option<u32>> {
        if name == REGISTRY_NAME && self.fail_registry_owner_query {
            bail!("simulated registry owner query failure")
        }
        Ok(match name {
            A11Y_BUS_NAME => self.launcher_owner,
            REGISTRY_NAME => self.registry_owner,
            _ => None,
        })
    }

    fn accessibility_bus_address(&mut self, _session_bus_address: &str) -> Result<String> {
        Ok("unix:path=/tmp/a11y-private".to_string())
    }

    fn activate_registry(&mut self, _accessibility_bus_address: &str) -> Result<()> {
        if self.activation_succeeds {
            self.registry_owner = Some(202);
            Ok(())
        } else {
            bail!("simulated unit failed")
        }
    }

    fn probe_registry(&mut self, _accessibility_bus_address: &str) -> Result<()> {
        self.probed = true;
        if self.fail_probe {
            bail!("simulated registry probe failure")
        }
        Ok(())
    }

    fn spawn_launcher(&mut self, _env: &IsolatedAtspiEnv) -> Result<SpawnedProcess> {
        self.spawned_launcher += 1;
        self.launcher_owner = Some(101);
        Ok(SpawnedProcess {
            pid: 101,
            child: None,
        })
    }

    fn spawn_registry(
        &mut self,
        _env: &IsolatedAtspiEnv,
        _accessibility_bus_address: &str,
    ) -> Result<SpawnedProcess> {
        self.spawned_registry += 1;
        self.registry_owner = Some(303);
        Ok(SpawnedProcess {
            pid: 303,
            child: None,
        })
    }

    fn validate_process(
        &mut self,
        pid: u32,
        _executable: &Path,
        _env: &IsolatedAtspiEnv,
        accessibility_bus_address: Option<&str>,
    ) -> Result<u64> {
        self.validations
            .push((pid, accessibility_bus_address.map(ToString::to_string)));
        Ok(u64::from(pid) * 10)
    }

    fn terminate_process(&mut self, pid: u32) -> Result<()> {
        self.terminated_records.push(pid);
        Ok(())
    }

    fn terminate_spawned(&mut self, process: &mut SpawnedProcess) {
        self.terminated.push(process.pid);
    }

    fn sleep(&mut self, _duration: Duration) {}
}

fn test_env() -> IsolatedAtspiEnv {
    IsolatedAtspiEnv {
        display: ":131".to_string(),
        session_bus_address: "unix:path=/tmp/session-private".to_string(),
        xauthority: "/tmp/xpra-Xauthority".to_string(),
    }
}

fn test_state(registry_direct: bool) -> AtspiSessionState {
    AtspiSessionState {
        version: STATE_VERSION,
        display: ":131".to_string(),
        xauthority: "/tmp/xpra-Xauthority".to_string(),
        session_bus_address: "unix:path=/tmp/session-private".to_string(),
        accessibility_bus_address: "unix:path=/tmp/a11y-private".to_string(),
        launcher: ProcessIdentity {
            pid: 101,
            start_ticks: 1010,
        },
        registry: ProcessIdentity {
            pid: 202,
            start_ticks: 2020,
        },
        registry_direct,
    }
}

#[test]
fn child_environment_enables_private_x11_accessibility() {
    let env = test_env();
    let mut command = Command::new("at-spi-test");
    configure_child(&mut command, &env, None);
    let values = command
        .get_envs()
        .map(|(key, value)| (key.to_owned(), value.map(OsStr::to_owned)))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        values.get(OsStr::new("DISPLAY")).unwrap().as_deref(),
        Some(OsStr::new(":131"))
    );
    assert_eq!(
        values.get(OsStr::new("NO_AT_BRIDGE")).unwrap().as_deref(),
        Some(OsStr::new("0"))
    );
    assert_eq!(
        values
            .get(OsStr::new("ACCESSIBILITY_ENABLED"))
            .unwrap()
            .as_deref(),
        Some(OsStr::new("1"))
    );
    assert_eq!(values.get(OsStr::new("WAYLAND_DISPLAY")), Some(&None));
    assert_eq!(values.get(OsStr::new("AT_SPI_BUS_ADDRESS")), Some(&None));
}

fn process_environment(
    env: &IsolatedAtspiEnv,
    accessibility_bus_address: Option<&str>,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::from([
        ("DISPLAY".to_string(), env.display.clone()),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            env.session_bus_address.clone(),
        ),
        ("XAUTHORITY".to_string(), env.xauthority.clone()),
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
        ("NO_AT_BRIDGE".to_string(), "0".to_string()),
        ("ACCESSIBILITY_ENABLED".to_string(), "1".to_string()),
    ]);
    if let Some(address) = accessibility_bus_address {
        values.insert("AT_SPI_BUS_ADDRESS".to_string(), address.to_string());
    }
    values
}

#[test]
fn process_environment_requires_x11_session_type() {
    let env = test_env();
    let mut values = process_environment(&env, None);
    validate_process_environment(&values, &env, None, 42).expect("x11 environment is valid");

    values.insert("XDG_SESSION_TYPE".to_string(), "wayland".to_string());
    validate_process_environment(&values, &env, None, 42)
        .expect_err("wayland owner must not be authorized");
}

#[test]
fn process_environment_requires_exact_accessibility_bus_contract() {
    let env = test_env();
    let address = "unix:path=/tmp/a11y-private";
    let mut values = process_environment(&env, None);
    validate_process_environment(&values, &env, None, 42)
        .expect("launcher must not carry an accessibility bus address");

    values.insert("AT_SPI_BUS_ADDRESS".to_string(), address.to_string());
    validate_process_environment(&values, &env, None, 42)
        .expect_err("launcher must not carry AT_SPI_BUS_ADDRESS");
    validate_process_environment(&values, &env, Some(address), 42)
        .expect("registry must carry its exact accessibility bus address");

    values.insert(
        "AT_SPI_BUS_ADDRESS".to_string(),
        "unix:path=/tmp/other-a11y".to_string(),
    );
    validate_process_environment(&values, &env, Some(address), 42)
        .expect_err("registry address must match exactly");
}

#[test]
fn activation_failure_uses_direct_registry_fallback() {
    let mut ops = FakeOps::new(false);
    let state = bootstrap_with(&mut ops, &test_env(), None).expect("fallback should bootstrap");

    assert_eq!(ops.spawned_launcher, 1);
    assert_eq!(ops.spawned_registry, 1);
    assert!(ops.probed);
    assert!(state.registry_direct);
    assert_eq!(state.registry.pid, 303);
}

#[test]
fn activation_success_does_not_spawn_registry_directly() {
    let mut ops = FakeOps::new(true);
    let state = bootstrap_with(&mut ops, &test_env(), None).expect("activation should bootstrap");

    assert_eq!(ops.spawned_launcher, 1);
    assert_eq!(ops.spawned_registry, 0);
    assert!(ops.probed);
    assert!(!state.registry_direct);
    assert_eq!(state.registry.pid, 202);
}

#[test]
fn failed_bootstrap_terminates_only_children_from_that_attempt() {
    let mut ops = FakeOps::new(false);
    ops.fail_probe = true;

    bootstrap_with(&mut ops, &test_env(), None).expect_err("probe failure must fail closed");

    assert_eq!(ops.terminated, vec![303, 101]);
}

#[test]
fn persistence_failure_terminates_newly_validated_owners() {
    let mut ops = FakeOps::new(false);
    let mut previous = test_state(true);
    previous.registry = ProcessIdentity {
        pid: 999,
        start_ticks: 9990,
    };

    ensure_with(&mut ops, &test_env(), Some(&previous), |_| {
        bail!("simulated persistence failure")
    })
    .expect_err("persistence failure must fail the ensure operation");

    assert_eq!(ops.terminated_records, vec![303, 101]);
}

#[test]
fn exact_direct_registry_reuse_validates_its_private_bus_before_acceptance() {
    let mut ops = FakeOps::new(false);
    ops.launcher_owner = Some(101);
    ops.registry_owner = Some(202);
    let previous = test_state(true);

    let state = ensure_with(&mut ops, &test_env(), Some(&previous), |_| Ok(()))
        .expect("exact direct registry reuse should succeed");

    assert!(state.registry_direct);
    assert_eq!(ops.spawned_launcher, 0);
    assert_eq!(ops.spawned_registry, 0);
    assert_eq!(
        ops.validations,
        vec![
            (101, None),
            (202, Some("unix:path=/tmp/a11y-private".to_string()))
        ]
    );
}

#[test]
fn exact_owner_reuse_skips_redundant_persistence() {
    let mut ops = FakeOps::new(false);
    ops.launcher_owner = Some(101);
    ops.registry_owner = Some(202);
    let previous = test_state(true);
    let mut persist_called = false;

    let state = ensure_with(&mut ops, &test_env(), Some(&previous), |_| {
        persist_called = true;
        bail!("unchanged state must not be rewritten")
    })
    .expect("exact owner reuse should not depend on rewriting unchanged state");

    assert_eq!(state, previous);
    assert!(!persist_called);
    assert!(ops.terminated_records.is_empty());
}

#[test]
fn registry_teardown_failure_does_not_skip_launcher_teardown() {
    let mut ops = FakeOps::new(true);
    ops.launcher_owner = Some(101);
    ops.fail_registry_owner_query = true;

    let error = terminate_recorded_state_with(&mut ops, &test_state(false))
        .expect_err("registry query failure must be reported");

    assert!(error.to_string().contains("registry teardown failed"));
    assert_eq!(ops.terminated_records, vec![101]);
}

#[test]
fn teardown_state_is_removed_only_after_success() {
    let directory = std::env::temp_dir().join(format!(
        "sky-cua-atspi-teardown-test-{}-{}",
        std::process::id(),
        NEXT_STATE_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).expect("test directory should exist");

    let completed = directory.join("completed.json");
    fs::write(&completed, b"completed").expect("completed state should exist");
    finish_termination(&completed, Ok(()));
    assert!(!completed.exists());

    let retryable = directory.join("retryable.json");
    fs::write(&retryable, b"retryable").expect("retryable state should exist");
    finish_termination(&retryable, Err(anyhow!("simulated teardown failure")));
    assert!(retryable.exists());

    fs::remove_file(retryable).expect("retryable state should be removed");
    fs::remove_dir(directory).expect("test directory should be removed");
}

#[test]
fn changed_owner_generation_is_never_authorized_for_termination() {
    let expected = ProcessIdentity {
        pid: 42,
        start_ticks: 9001,
    };
    assert!(recorded_owner_matches(&expected, 42, 9001));
    assert!(!recorded_owner_matches(&expected, 43, 9001));
    assert!(!recorded_owner_matches(&expected, 42, 9002));
}

#[test]
fn direct_registry_identity_survives_only_exact_session_reuse() {
    let mut previous = AtspiSessionState {
        version: STATE_VERSION,
        display: ":131".to_string(),
        xauthority: "/tmp/xpra-Xauthority".to_string(),
        session_bus_address: "unix:path=/tmp/session".to_string(),
        accessibility_bus_address: "unix:path=/tmp/a11y".to_string(),
        launcher: ProcessIdentity {
            pid: 10,
            start_ticks: 11,
        },
        registry: ProcessIdentity {
            pid: 12,
            start_ticks: 13,
        },
        registry_direct: true,
    };
    let mut current = previous.clone();
    current.registry_direct = false;
    assert!(preserves_direct_registry(&previous, &current));

    current.registry.start_ticks += 1;
    assert!(!preserves_direct_registry(&previous, &current));
    current.registry.start_ticks -= 1;
    previous.registry_direct = false;
    assert!(!preserves_direct_registry(&previous, &current));
}

#[test]
fn state_round_trips_as_owner_only_json() {
    let directory = std::env::temp_dir().join(format!(
        "sky-cua-atspi-state-test-{}-{}",
        std::process::id(),
        NEXT_STATE_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let path = directory.join("state.json");
    let state = AtspiSessionState {
        version: STATE_VERSION,
        display: ":131".to_string(),
        xauthority: "/tmp/xpra-Xauthority".to_string(),
        session_bus_address: "unix:path=/tmp/session".to_string(),
        accessibility_bus_address: "unix:path=/tmp/a11y".to_string(),
        launcher: ProcessIdentity {
            pid: 10,
            start_ticks: 11,
        },
        registry: ProcessIdentity {
            pid: 12,
            start_ticks: 13,
        },
        registry_direct: true,
    };

    persist_state(&path, &state).expect("state should persist");
    assert_eq!(read_state(&path).expect("state should read"), Some(state));
    let mode = fs::metadata(&path)
        .expect("state metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    fs::remove_file(path).expect("state should be removed");
    fs::remove_dir(directory).expect("state directory should be removed");
}
