//! Managed scrcpy mirror lifecycle: launch (with the codec retry policy) and
//! orderly stop.
//!
//! `phone_connect` calls [`PhoneManager::launch_scrcpy`] when a caller asks for a
//! mirror; `phone_disconnect` calls [`PhoneManager::stop_managed_scrcpy`] to
//! terminate the sky-cua-owned process. Split from `manager/mod.rs` to keep that
//! file under the god-file threshold.
//!
//! The launch path resolves the binary, probes its version, builds the launch
//! spec from the resolved selection, and drives the H.265 -> H.264 -> default
//! codec retry policy. Each spawn is followed by a short liveness check: scrcpy
//! that exits immediately is treated as a codec-retryable failure so the next
//! codec is tried; one that stays up is a launch. The live child is held by the
//! caller so disconnect can kill it, and `kill_on_drop` guards against a daemon
//! crash orphaning the mirror window.

use std::process::Stdio;
use std::time::Duration;

use sky_cua_platform::model::{DiagnosticEntry, PixelSize};

use super::{PhoneManager, ScrcpyRuntime, now_ms};
use crate::phone::scrcpy::{
    self, LaunchAttempt, ScrcpyCodec, ScrcpyLaunchSpec, ScrcpyProcess, ScrcpyResolution,
};

/// What the daemon needs to find and map a session's scrcpy desktop window.
///
/// Produced by [`PhoneManager::scrcpy_window_to_map`] for any session that has a
/// live mirror but no host-window mapping yet. The daemon matches the window by
/// `pid` first (robust against title collisions) and falls back to
/// `window_title`, then computes the content rect against `device_size`/
/// `rotation_degrees`.
pub(crate) struct ScrcpyWindowTarget {
    pub(crate) session_id: String,
    pub(crate) pid: Option<u32>,
    pub(crate) window_title: String,
    pub(crate) device_size: PixelSize,
    pub(crate) rotation_degrees: i32,
}

/// How long to wait after spawning scrcpy before deciding it stayed up. A mirror
/// that exits within this window (bad codec, device gone) is treated as a launch
/// failure for that codec rather than a live process.
const LIVENESS_WAIT: Duration = Duration::from_millis(700);

impl PhoneManager {
    /// Launch a managed scrcpy mirror for `serial`, driving the codec retry policy.
    ///
    /// Resolves the scrcpy binary (a `Missing` resolution is an immediate launch
    /// failure), probes its version, and tries codecs in order (H.265, H.264,
    /// default). For each codec it spawns scrcpy with all stdio nulled and
    /// `kill_on_drop`, waits a short liveness window, and inspects the child: an
    /// early exit is a codec-retryable failure; a still-running child is a launch.
    /// On success returns the managed [`ScrcpyProcess`], the live
    /// [`tokio::process::Child`] (held by the caller for orderly stop), and the
    /// probed version. On failure across every codec returns a structured
    /// [`DiagnosticEntry`]; the caller keeps the session and degrades to
    /// ADB/companion.
    pub(super) async fn launch_scrcpy(
        &self,
        serial: &str,
    ) -> Result<(ScrcpyProcess, tokio::process::Child, Option<String>), DiagnosticEntry> {
        let path = match scrcpy::resolve_scrcpy(&self.selection) {
            ScrcpyResolution::Found { path } => path,
            ScrcpyResolution::Missing { reason } => {
                return Err(scrcpy::launch_failed_diagnostic(reason));
            }
        };

        let version = scrcpy::probe_version(self.runner.as_ref(), &path).await;
        let mut spec = ScrcpyLaunchSpec::from_selection(&self.selection, serial);
        // Cap the mirror at a phone-scale size when `[phone] max_size` is unset,
        // using the host-derived default the daemon primed from the display
        // topology. Without this scrcpy mirrors at the device's full (often very
        // high) resolution and the window fills the desktop.
        spec.apply_default_max_size(self.scrcpy_host_size_default);

        // The shared `launch_with_retry` policy takes a synchronous closure, but the
        // real spawn + liveness check is async, so the codec order and
        // stop-on-success semantics are driven inline here. The surviving child is
        // captured in `live` and taken back out once a codec wins.
        let mut live: Option<(ScrcpyCodec, tokio::process::Child)> = None;
        let attempt_result = self.spawn_across_codecs(&path, &spec, &mut live).await;

        match attempt_result {
            LaunchAttempt::Launched { codec, pid } => {
                // The surviving child matches the launched codec.
                let (_, child) = live.expect("launched attempt must hold a live child");
                let pid = pid.or_else(|| child.id());
                let process = match pid {
                    Some(pid) => ScrcpyProcess::managed(pid, serial, codec),
                    // No PID exposed: still managed, but record it without one. This
                    // is rare (the OS dropped the id between spawn and check); the
                    // child handle still drives the kill on disconnect.
                    None => ScrcpyProcess::managed(0, serial, codec),
                };
                Ok((process, child, version))
            }
            LaunchAttempt::Failed { .. } => Err(scrcpy::launch_failed_diagnostic(
                "scrcpy exited immediately for every codec (h265, h264, default)",
            )),
        }
    }

    /// Establish a managed scrcpy mirror for `serial`, adopting an already-running
    /// window when the daemon primed one and otherwise launching fresh, and fold the
    /// resulting live-mirror capability into `profile.scrcpy`.
    ///
    /// Shared by the fresh-session connect path and the idempotent-reconnect path so
    /// both adopt-or-launch through one code path. On success returns the new
    /// [`ScrcpyRuntime`] and its window title (`profile.scrcpy` is flipped to the
    /// active capability); on failure returns the structured launch diagnostic and
    /// leaves `profile.scrcpy` untouched (it stays idle) so the caller degrades to
    /// ADB/companion. Returns no runtime, title, or diagnostic only when an adopt
    /// path produced a window with no diagnostic — adoption never fails here.
    ///
    /// Adoption: the daemon primes a candidate (a window whose deterministic
    /// `sky-cua-phone-<safe-serial>` title it found) before connect, so a previous
    /// run's mirror — or one the operator left up — is reused instead of stacking a
    /// second mirror. An adopted window carries no child we own (`child: None`);
    /// ownership is `Adopted`, so the liveness watchdog never polls it and disconnect
    /// never kills it.
    pub(super) async fn establish_scrcpy_mirror(
        &self,
        serial: &str,
        profile: &mut sky_cua_platform::model::PhoneCapabilityProfile,
    ) -> (
        Option<ScrcpyRuntime>,
        Option<String>,
        Option<DiagnosticEntry>,
    ) {
        if let Some(candidate) = self.adoption_candidate_for(serial) {
            let process =
                scrcpy::ScrcpyProcess::adopted(candidate.pid, &candidate.window_title, serial);
            profile.scrcpy = scrcpy::active_capabilities(None, &process, false);
            let window_title = process.window_title.clone();
            let runtime = ScrcpyRuntime {
                process,
                child: None,
                mapping: None,
                mapping_attempts_exhausted: false,
            };
            return (Some(runtime), Some(window_title), None);
        }
        match self.launch_scrcpy(serial).await {
            Ok((process, child, version)) => {
                profile.scrcpy = scrcpy::active_capabilities(version, &process, false);
                let window_title = process.window_title.clone();
                let runtime = ScrcpyRuntime {
                    process,
                    child: Some(child),
                    mapping: None,
                    mapping_attempts_exhausted: false,
                };
                (Some(runtime), Some(window_title), None)
            }
            Err(diagnostic) => (None, None, Some(diagnostic)),
        }
    }

    /// Re-establish a session's managed scrcpy mirror on an idempotent reconnect
    /// whose request asked for scrcpy but whose previous mirror is gone (torn down
    /// by [`poll_scrcpy_liveness`] after a crash or operator close).
    ///
    /// [`rebuild_session`] runs first and re-detects the profile/capabilities, but it
    /// does not touch the scrcpy runtime, so a `phone_connect{start_scrcpy:true}`
    /// (or `backend == Scrcpy`) against an existing session whose mirror died would
    /// otherwise report `scrcpy.active=false` and silently never relaunch. This
    /// adopts-or-launches through [`establish_scrcpy_mirror`], then re-folds the
    /// live-mirror capability back into the session's cached profile, capabilities,
    /// action list, and public view — mirroring the post-launch recompute the
    /// fresh-session path performs — and installs the new runtime. On a relaunch
    /// failure it records the structured diagnostic on the session (surfaced by
    /// `phone_companion_status`) and leaves scrcpy honestly inactive.
    ///
    /// A no-op when the session is gone or already owns a live mirror; the caller
    /// gates entry on `request_wants_scrcpy && entry.scrcpy.is_none()`.
    ///
    /// [`rebuild_session`]: PhoneManager::rebuild_session
    /// [`poll_scrcpy_liveness`]: PhoneManager::poll_scrcpy_liveness
    /// [`establish_scrcpy_mirror`]: PhoneManager::establish_scrcpy_mirror
    pub(super) async fn relaunch_scrcpy_on_reconnect(&mut self, session_id: &str) {
        let Some(serial) = self
            .sessions
            .get(session_id)
            .map(|entry| entry.session.serial.clone())
        else {
            return;
        };

        // Start from the freshly rebuilt cached profile so the relaunch folds onto
        // the same profile rebuild_session just produced.
        let Some(mut profile) = self
            .profiles
            .get(session_id)
            .map(|cached| cached.profile.clone())
        else {
            return;
        };

        let (runtime, window_title, diagnostic) =
            self.establish_scrcpy_mirror(&serial, &mut profile).await;

        // Recompute capabilities/affordances after the relaunch so they reflect the
        // live mirror (`profile.scrcpy.active`) when one came up, mirroring the
        // fresh-session path's post-launch recompute.
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        let now = now_ms();
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
            entry.session.managed_process = runtime.is_some();
            if window_title.is_some() {
                entry.session.window_title = window_title;
            }
            entry.scrcpy = runtime;
            // Surface a relaunch failure the same way connect does: append it to the
            // session's diagnostics so `phone_companion_status` can report that the
            // mirror could not be re-established.
            if let Some(diagnostic) = diagnostic {
                entry.companion_diagnostics.push(diagnostic);
            }
        }
        self.profiles.insert(
            session_id.to_string(),
            super::CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );
    }

    /// Spawn scrcpy across the codec retry order, keeping the first child that
    /// survives the liveness window. Returns the policy's [`LaunchAttempt`]; on a
    /// `Launched` result `live` holds the surviving `(codec, child)`.
    ///
    /// This mirrors [`scrcpy::launch_with_retry`]'s ordering and stop-on-success
    /// semantics, but inline so the async spawn can run per codec (the shared
    /// policy helper takes a synchronous closure and is unit-tested separately).
    async fn spawn_across_codecs(
        &self,
        path: &str,
        spec: &ScrcpyLaunchSpec,
        live: &mut Option<(ScrcpyCodec, tokio::process::Child)>,
    ) -> LaunchAttempt {
        let mut last = LaunchAttempt::Failed {
            codec: ScrcpyCodec::Default,
            retryable: false,
        };
        for codec in scrcpy::CODEC_RETRY_ORDER {
            match self.spawn_one(path, spec, codec).await {
                LaunchOutcome::Live(child) => {
                    *live = Some((codec, child));
                    return LaunchAttempt::Launched {
                        codec,
                        pid: live.as_ref().and_then(|(_, child)| child.id()),
                    };
                }
                LaunchOutcome::Exited => {
                    last = LaunchAttempt::Failed {
                        codec,
                        retryable: true,
                    };
                }
                LaunchOutcome::SpawnError => {
                    // A spawn error (binary unusable) is not codec-attributable;
                    // retrying other codecs cannot help, so stop immediately.
                    return LaunchAttempt::Failed {
                        codec,
                        retryable: false,
                    };
                }
            }
        }
        last
    }

    /// Spawn scrcpy once at `codec`, with all stdio nulled and `kill_on_drop`, then
    /// wait the liveness window and classify the outcome.
    async fn spawn_one(
        &self,
        path: &str,
        spec: &ScrcpyLaunchSpec,
        codec: ScrcpyCodec,
    ) -> LaunchOutcome {
        let mut command = tokio::process::Command::new(path);
        command
            .args(spec.argv(codec))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => return LaunchOutcome::SpawnError,
        };

        tokio::time::sleep(LIVENESS_WAIT).await;

        match child.try_wait() {
            // Already exited within the window: treat as a codec-retryable failure.
            Ok(Some(_)) => LaunchOutcome::Exited,
            // Still running: a live mirror. `kill_on_drop` keeps the dropped child
            // from being orphaned if we never reach an orderly stop.
            Ok(None) => LaunchOutcome::Live(child),
            // Could not poll the child (rare): be conservative and treat it as a
            // failure for this codec rather than claiming a live mirror.
            Err(_) => LaunchOutcome::Exited,
        }
    }

    /// Stop a sky-cua-managed scrcpy mirror as part of `phone_disconnect`.
    ///
    /// Only managed, still-running processes are stopped (`can_be_stopped_by_us`);
    /// adopted/external windows are left alone. Returns a structured diagnostic so
    /// disconnect is honest about whether it actually terminated a managed mirror.
    pub(super) async fn stop_managed_scrcpy(mut runtime: ScrcpyRuntime) -> Vec<DiagnosticEntry> {
        if !runtime.process.can_be_stopped_by_us() {
            return Vec::new();
        }
        // Kill the live child, then mark the ownership record stopped. `kill` is
        // idempotent enough here: a child that already exited yields an error we
        // do not need to surface, since the outcome (no live mirror) is the same.
        // Only a managed mirror carries a child we own; `can_be_stopped_by_us`
        // above already guaranteed managed ownership, so `child` is `Some` here.
        if let Some(child) = runtime.child.as_mut() {
            let _ = child.kill().await;
        }
        runtime.process.mark_stopped();
        vec![DiagnosticEntry {
            code: "PhoneScrcpyStopped".to_string(),
            message: "stopped the managed scrcpy mirror for this session".to_string(),
            details: runtime
                .process
                .pid
                .map(|pid| format!("pid={pid}, window_title={}", runtime.process.window_title)),
        }]
    }

    // ===================================================================
    // Host-window mapping (the host-visible cursor overlay plane)
    // ===================================================================

    /// The session whose managed scrcpy mirror is live but not yet host-mapped,
    /// returning everything the daemon needs to locate its desktop window and
    /// compute the content-rect mapping. Returns `None` when no session is
    /// awaiting mapping (no mirror, already mapped, or no device size known).
    ///
    /// Only sessions with `scrcpy.active && !host_window_mapped` and a stored
    /// runtime mapping of `None` are eligible, so the daemon does not re-map an
    /// already-mapped window. The device size comes from the cached profile's
    /// detected display geometry; rotation is the profile's live
    /// `display_rotation_degrees` quarter (0/90/180/270) when probed, falling
    /// back to the orientation label (portrait -> 0, landscape -> 90, default 0)
    /// only when no live rotation is available.
    pub(crate) fn scrcpy_window_to_map(&self) -> Option<ScrcpyWindowTarget> {
        self.sessions.iter().find_map(|(session_id, entry)| {
            let runtime = entry.scrcpy.as_ref()?;
            if runtime.mapping.is_some() {
                return None;
            }
            // A mirror whose bounded retry round already ran and failed is not
            // re-offered, so the daemon does not re-run its ~2s poll on every
            // subsequent phone request for a window that never registers. The
            // marker is re-armed only when a fresh runtime is built on
            // reconnect/refresh.
            if runtime.mapping_attempts_exhausted {
                return None;
            }
            let scrcpy = &entry.session.capability_profile.scrcpy;
            if !scrcpy.active || scrcpy.host_window_mapped {
                return None;
            }
            let device_size = entry.session.capability_profile.display_size.clone()?;
            let rotation_degrees = resolve_rotation_degrees(&entry.session.capability_profile);
            Some(ScrcpyWindowTarget {
                session_id: session_id.clone(),
                pid: runtime.process.pid,
                window_title: runtime.process.window_title.clone(),
                device_size,
                rotation_degrees,
            })
        })
    }

    /// The session whose scrcpy mirror is already host-mapped, returning the same
    /// locate-the-window target so the daemon can re-query the live window rect and
    /// recompute the mapping if it drifted (the operator resized the scrcpy window,
    /// or the host display scale changed). Returns `None` when no mapped session
    /// exists.
    ///
    /// This is the counterpart to [`scrcpy_window_to_map`]: that offers only the
    /// not-yet-mapped session (the initial map), while this offers the mapped one
    /// (the re-map drift check). The daemon runs the re-query on the per-request
    /// window-work path it already takes — no new polling loop — and feeds the
    /// fresh window rect back through [`set_scrcpy_window_mapping`], which is
    /// idempotent: an unchanged rect recomputes the same content rect and returns
    /// without rebuilding, so a stable window is a cheap no-op.
    ///
    /// [`scrcpy_window_to_map`]: Self::scrcpy_window_to_map
    /// [`set_scrcpy_window_mapping`]: Self::set_scrcpy_window_mapping
    pub(crate) fn scrcpy_window_to_remap(&self) -> Option<ScrcpyWindowTarget> {
        self.sessions.iter().find_map(|(session_id, entry)| {
            let runtime = entry.scrcpy.as_ref()?;
            // Only an already-mapped mirror is a re-map candidate; the initial map
            // is `scrcpy_window_to_map`'s job.
            runtime.mapping.as_ref()?;
            let scrcpy = &entry.session.capability_profile.scrcpy;
            if !scrcpy.active || !scrcpy.host_window_mapped {
                return None;
            }
            let device_size = entry.session.capability_profile.display_size.clone()?;
            let rotation_degrees = resolve_rotation_degrees(&entry.session.capability_profile);
            Some(ScrcpyWindowTarget {
                session_id: session_id.clone(),
                pid: runtime.process.pid,
                window_title: runtime.process.window_title.clone(),
                device_size,
                rotation_degrees,
            })
        })
    }

    /// Clear a stale host-window mapping for a session whose scrcpy window was
    /// previously mapped but can no longer be found or no longer exposes bounds.
    /// The mirror may still be running, especially for adopted windows we do not
    /// own, so keep `scrcpy.active` tied to the process liveness and only drop the
    /// host-window mapping/overlay plane.
    pub(crate) fn clear_scrcpy_window_mapping(&mut self, session_id: &str) -> bool {
        let mut profile = {
            let Some(entry) = self.sessions.get_mut(session_id) else {
                return false;
            };
            let Some(runtime) = entry.scrcpy.as_mut() else {
                return false;
            };
            if runtime.mapping.is_none() {
                return false;
            }
            runtime.mapping = None;
            runtime.mapping_attempts_exhausted = true;

            let mut profile = entry.session.capability_profile.clone();
            let version = profile.scrcpy.version.clone();
            profile.scrcpy = scrcpy::active_capabilities(version, &runtime.process, false);
            profile
        };
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
        }
        self.profiles.insert(
            session_id.to_string(),
            super::CachedProfile {
                profile,
                detected_at_ms: now_ms(),
            },
        );
        true
    }

    /// Compute and record the host-window content-rect mapping for a session from
    /// the discovered desktop window rect, and flip its scrcpy capability to
    /// `host_window_mapped=true`.
    ///
    /// The daemon supplies the raw host window `bounds` plus the `device_size`/
    /// `rotation_degrees` it read from the same [`ScrcpyWindowTarget`]; the
    /// content-rect math (letterboxing, rotation, fractional host scale) stays
    /// encapsulated in the phone module here. Stores the content rect on the
    /// runtime, rebuilds `profile.scrcpy` with the mapping on (preserving the
    /// probed version/codec via the running process), recomputes cursor
    /// capabilities and the action affordance list, and refreshes the cached
    /// profile + the public session view. Returns whether a mapping is current (a
    /// degenerate window or device rect leaves the session unmapped and honest). A
    /// no-op when the session has no live managed mirror.
    ///
    /// Idempotent: recomputing the same content rect against an already-mapped
    /// session (the window did not move/resize and the host scale is unchanged)
    /// returns `true` without touching the profile cache, so the daemon's
    /// resize/scale re-map check can call this on every request without churning
    /// the cached profile. Only a changed content rect rebuilds.
    pub(crate) fn set_scrcpy_window_mapping(
        &mut self,
        session_id: &str,
        host_window: &sky_cua_platform::model::RectF,
        device_size: PixelSize,
        rotation_degrees: i32,
    ) -> bool {
        let Some(content) = scrcpy::content_rect(host_window, device_size, rotation_degrees) else {
            return false;
        };

        let Some(entry) = self.sessions.get_mut(session_id) else {
            return false;
        };
        let Some(runtime) = entry.scrcpy.as_mut() else {
            return false;
        };
        // Already mapped to the same content rect: the window rect and host scale
        // are unchanged, so there is nothing to recompute. Returning early avoids
        // re-inserting an identical cached profile (no profile churn) and keeps the
        // re-map check on the per-request path cheap.
        if runtime.mapping.as_ref() == Some(&content) {
            return true;
        }
        let version = entry.session.capability_profile.scrcpy.version.clone();
        runtime.mapping = Some(content);
        let scrcpy_caps = scrcpy::active_capabilities(version, &runtime.process, true);

        let mut profile = entry.session.capability_profile.clone();
        profile.scrcpy = scrcpy_caps;
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        let now = now_ms();
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
        }
        self.profiles.insert(
            session_id.to_string(),
            super::CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );
        true
    }

    /// The public [`PhoneSession`] view for a session id, so the daemon can
    /// re-read it after applying a host-window mapping.
    pub(crate) fn session_view(
        &self,
        session_id: &str,
    ) -> Option<sky_cua_platform::model::PhoneSession> {
        self.sessions
            .get(session_id)
            .map(|entry| entry.session.clone())
    }

    /// Mark a session's managed scrcpy mirror as having exhausted the daemon's
    /// bounded window-mapping retry round, so [`scrcpy_window_to_map`] stops
    /// offering it until a fresh runtime is built on reconnect/refresh. A no-op
    /// when the session has no live managed mirror or it has since been mapped.
    ///
    /// [`scrcpy_window_to_map`]: Self::scrcpy_window_to_map
    pub(crate) fn mark_scrcpy_mapping_exhausted(&mut self, session_id: &str) {
        if let Some(entry) = self.sessions.get_mut(session_id)
            && let Some(runtime) = entry.scrcpy.as_mut()
            && runtime.mapping.is_none()
        {
            runtime.mapping_attempts_exhausted = true;
        }
    }

    /// The id of a session that owns a live, host-mapped scrcpy mirror, if any.
    ///
    /// An honest query of the scrcpy host-window mapping surface: a `Some` means a
    /// mirror's desktop window is mapped and the mirror is sized/located against
    /// it. The phone-to-desktop cursor bridge that consumed this in production was
    /// removed (the agent cursor is now drawn on the device by the companion
    /// overlay), so the accessor is currently only exercised by the scrcpy-mapping
    /// lifecycle tests; the `expect(dead_code)` keeps non-test builds clean while
    /// preserving the tested mapping-surface contract.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn host_overlay_session(&self) -> Option<String> {
        self.sessions.iter().find_map(|(session_id, entry)| {
            let runtime = entry.scrcpy.as_ref()?;
            runtime.mapping.as_ref().map(|_| session_id.clone())
        })
    }

    /// Poll every session's managed scrcpy child for a mid-session exit (crash or
    /// the operator closing the mirror window) and downgrade any that died.
    ///
    /// After the post-spawn liveness check, nothing else re-polls the child, so a
    /// mirror that dies mid-session would otherwise keep `scrcpy.active`/
    /// `host_window_mapped` true and the daemon would keep pushing host-cursor
    /// draws onto a dead window. For each session whose runtime is a still-tracked
    /// managed process that `try_wait()` reports as exited, this marks the process
    /// crashed, rebuilds the cached profile + session capabilities from the
    /// downgraded scrcpy capability (mirroring [`set_scrcpy_window_mapping`]),
    /// tears down the dead runtime (so disconnect never kills an exited child and
    /// the window is no longer offered for mapping), and records the session.
    ///
    /// Returns `(session_id, was_host_mapped)` for each crash detected, so the
    /// daemon's watchdog can hide the host overlay for a mirror that had a live
    /// host-window mapping when it died.
    pub(crate) fn poll_scrcpy_liveness(&mut self) -> Vec<(String, bool)> {
        // Discover the dead managed mirrors under a short immutable pass, then
        // mutate by session id, so no `&mut` borrow of `self.sessions` is held
        // across the profile/capability rebuild that also touches `self.profiles`.
        let mut dead: Vec<(String, bool)> = Vec::new();
        for (session_id, entry) in self.sessions.iter_mut() {
            let Some(runtime) = entry.scrcpy.as_mut() else {
                continue;
            };
            if !runtime.process.can_be_stopped_by_us() {
                // Only live, sky-cua-managed processes are tracked children we own
                // and may poll/teardown; adopted/external windows are left alone.
                continue;
            }
            // Only a managed mirror carries a child we own and may poll. An adopted
            // window has `child: None` (its process belongs to whoever launched it),
            // so it must never be `try_wait`ed or treated as crashed; the ownership
            // gate above already excludes it, and the `Some` guard makes that
            // explicit. `try_wait` is non-blocking: `Ok(Some(_))` is an exit (crash
            // or the operator closing the mirror), `Ok(None)` is still running,
            // `Err(_)` is an un-pollable child we conservatively leave alone.
            let Some(child) = runtime.child.as_mut() else {
                continue;
            };
            if let Ok(Some(_status)) = child.try_wait() {
                dead.push((session_id.clone(), runtime.mapping.is_some()));
            }
        }

        for (session_id, _was_host_mapped) in &dead {
            self.downgrade_crashed_scrcpy(session_id);
        }
        dead
    }

    /// Mark a session's managed scrcpy process crashed and rebuild its cached
    /// profile + session capabilities from the downgraded scrcpy capability, then
    /// tear down the dead runtime. Mirrors the profile/capability rebuild in
    /// [`set_scrcpy_window_mapping`]. A no-op when the session or its runtime is
    /// already gone.
    fn downgrade_crashed_scrcpy(&mut self, session_id: &str) {
        let Some(entry) = self.sessions.get_mut(session_id) else {
            return;
        };
        let Some(runtime) = entry.scrcpy.as_mut() else {
            return;
        };
        // `mark_crashed` flips the process liveness to `Crashed` and returns the
        // downgraded `scrcpy::crashed_capabilities(..)` shape (inactive, unmapped,
        // with a structured reason). Clear the mapping so no host plane survives.
        let scrcpy_caps = runtime.process.mark_crashed();
        runtime.mapping = None;

        let mut profile = entry.session.capability_profile.clone();
        profile.scrcpy = scrcpy_caps;
        let capabilities = self.backend_capabilities(&profile);
        super::routing::populate_actions(&mut profile, &capabilities);

        let now = now_ms();
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capability_profile = profile.clone();
            entry.session.capabilities = capabilities;
            // Tear down the dead runtime: the child has already exited, so there is
            // nothing left to stop, and dropping it keeps disconnect from trying to
            // kill an exited child and stops the daemon offering the window for
            // mapping or treating it as a live host-overlay plane.
            entry.scrcpy = None;
        }
        self.profiles.insert(
            session_id.to_string(),
            super::CachedProfile {
                profile,
                detected_at_ms: now,
            },
        );
    }

    /// Map a device-pixel point into host-desktop pixels through a session's
    /// stored scrcpy content-rect mapping, returned as `(x, y)`. Returns `None`
    /// when the session is not host-mapped.
    ///
    /// Test-only: the production phone-to-desktop cursor bridge was removed (the
    /// agent cursor is drawn on the device by the companion overlay), but the
    /// scrcpy window content-rect mapping it relied on is still computed and
    /// re-checked for drift. This thin accessor lets the re-map tests assert the
    /// mapping recomputed after a window resize without resurrecting the bridge.
    #[cfg(test)]
    pub(super) fn scrcpy_device_to_host_for_tests(
        &self,
        session_id: &str,
        device_x: f64,
        device_y: f64,
    ) -> Option<(f64, f64)> {
        let runtime = self.sessions.get(session_id)?.scrcpy.as_ref()?;
        let mapping = runtime.mapping.as_ref()?;
        Some(mapping.device_to_host(device_x, device_y))
    }
}

/// Rotation in degrees for the host content-rect math.
///
/// Prefers the profile's live `display_rotation_degrees`, the exact quarter
/// (0/90/180/270) the `dumpsys` rotation probe reported, so 180/270 survive into
/// the content-rect mapping instead of collapsing back into the orientation
/// label's two states. Falls back to the label-derived quarter only when no live
/// rotation was probed (`display_rotation_degrees` is `None`), keeping the
/// portrait/landscape behavior unchanged for ADB-only or older capture paths.
fn resolve_rotation_degrees(profile: &sky_cua_platform::model::PhoneCapabilityProfile) -> i32 {
    profile
        .display_rotation_degrees
        .unwrap_or_else(|| rotation_from_orientation(profile.orientation.as_deref()))
}

/// Rotation in degrees for the host content-rect math, derived from the detected
/// orientation label: landscape -> 90, everything else (portrait/unknown) -> 0.
/// Used only as the fallback when no live `display_rotation_degrees` quarter was
/// probed.
fn rotation_from_orientation(orientation: Option<&str>) -> i32 {
    match orientation {
        Some(label) if label.eq_ignore_ascii_case("landscape") => 90,
        _ => 0,
    }
}

/// Classification of a single scrcpy spawn attempt at one codec.
enum LaunchOutcome {
    /// scrcpy spawned and was still running after the liveness window.
    Live(tokio::process::Child),
    /// scrcpy exited within the liveness window (or could not be polled).
    Exited,
    /// The spawn itself failed (binary not executable). Not codec-attributable.
    SpawnError,
}
