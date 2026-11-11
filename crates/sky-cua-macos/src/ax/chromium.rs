//! Activation of dormant Chromium and Electron accessibility trees.
//!
//! Chromium-based applications build their web-content accessibility tree
//! only once an assistive technology announces itself. On macOS that signal
//! is an attribute write on the application element: `AXManualAccessibility`
//! (Electron's documented switch) or `AXEnhancedUserInterface` (what
//! VoiceOver sets, honoured by Chromium). Until then a window reports only
//! its native frame and an empty `AXWebArea`.
//!
//! `AXEnhancedUserInterface` has side effects in some native applications
//! (window moves animate), so the write is limited to processes whose bundle
//! ships a Chromium engine, and made at most once per process that accepted
//! it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use sky_cua_platform::process::darwin::ProcessGeneration;

use super::element::{AxElement, cf_string};
use super::tree::WalkStats;

/// A walk that read at most this many elements is treated as a dormant
/// Chromium window (a woken Chromium window has hundreds).
pub(crate) const SHALLOW_TREE_ELEMENTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Engine {
    Electron,
    Chromium,
}

/// `/X.app/Contents/MacOS/X` -> `/X.app/Contents/Frameworks`.
pub(crate) fn frameworks_dir(executable: &Path) -> Option<PathBuf> {
    let contents = executable.parent()?.parent()?;
    (contents.file_name()? == "Contents").then(|| contents.join("Frameworks"))
}

/// Classifies a bundle by the engine framework it ships: Electron ships
/// `Electron Framework.framework`; Chromium browsers ship
/// `<Product> Framework.framework` with a `Helpers` directory of renderer
/// helper apps.
pub(crate) fn engine_from_frameworks(names: &[(String, bool)]) -> Option<Engine> {
    if names
        .iter()
        .any(|(name, _)| name == "Electron Framework.framework")
    {
        return Some(Engine::Electron);
    }
    names
        .iter()
        .any(|(name, has_helpers)| name.ends_with(" Framework.framework") && *has_helpers)
        .then_some(Engine::Chromium)
}

/// The engine of the bundle owning `executable`, cached per path.
pub(crate) fn engine(executable: &str) -> Option<Engine> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Engine>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(cached) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(executable).copied())
    {
        return cached;
    }
    let detected = frameworks_dir(Path::new(executable)).and_then(|dir| {
        let entries = std::fs::read_dir(&dir).ok()?;
        let names: Vec<(String, bool)> = entries
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let has_helpers = entry.path().join("Helpers").is_dir();
                (name, has_helpers)
            })
            .collect();
        engine_from_frameworks(&names)
    });
    if let Ok(mut cache) = cache.lock() {
        cache.insert(executable.to_string(), detected);
    }
    detected
}

/// Whether a walk looks like an unbuilt Chromium tree.
pub(crate) fn looks_dormant(stats: &WalkStats) -> bool {
    stats.read <= SHALLOW_TREE_ELEMENTS || stats.empty_web_areas > 0
}

/// Processes that accepted an activation write, plus any whose activation is
/// in progress. A process that accepted neither write is released so a later
/// read retries it: an application busy at first contact rejects both.
///
/// Keyed by exact process generation, not pid: a later application that is
/// handed a recorded pid has its own dormant tree and must still be woken.
#[derive(Debug, Default)]
pub(crate) struct ActivationAttempts(HashSet<ProcessGeneration>);

impl ActivationAttempts {
    /// Claims the activation of `process`; false when it already accepted
    /// one or one is in progress.
    pub(crate) fn claim(&mut self, process: ProcessGeneration) -> bool {
        self.0.insert(process)
    }

    /// Records the writes `process` accepted, releasing the claim when it
    /// accepted none.
    pub(crate) fn settle(&mut self, process: &ProcessGeneration, accepted: &[&str]) {
        if accepted.is_empty() {
            self.0.remove(process);
        }
    }
}

/// Writes both activation attributes on the application element. Returns
/// which writes the application accepted.
pub(crate) fn activate(app: &AxElement) -> Vec<&'static str> {
    ["AXManualAccessibility", "AXEnhancedUserInterface"]
        .into_iter()
        .filter(|attribute| app.set_bool(&cf_string(attribute), true).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[(&str, bool)]) -> Vec<(String, bool)> {
        entries
            .iter()
            .map(|(name, helpers)| ((*name).to_string(), *helpers))
            .collect()
    }

    #[test]
    fn frameworks_dir_comes_from_the_bundle_executable() {
        assert_eq!(
            frameworks_dir(Path::new("/Applications/Slack.app/Contents/MacOS/Slack")),
            Some(PathBuf::from("/Applications/Slack.app/Contents/Frameworks"))
        );
        assert_eq!(frameworks_dir(Path::new("/usr/bin/true")), None);
    }

    #[test]
    fn engines_are_recognised_by_their_framework() {
        assert_eq!(
            engine_from_frameworks(&names(&[
                ("Electron Framework.framework", false),
                ("Slack Helper.app", false)
            ])),
            Some(Engine::Electron)
        );
        assert_eq!(
            engine_from_frameworks(&names(&[(
                "Brave Browser Nightly Framework.framework",
                true
            )])),
            Some(Engine::Chromium)
        );
        assert_eq!(
            engine_from_frameworks(&names(&[("Sparkle.framework", false)])),
            None
        );
        assert_eq!(
            engine_from_frameworks(&names(&[("Mantle Framework.framework", false)])),
            None
        );
    }

    fn process(pid: u32, started_at_micros: u64) -> ProcessGeneration {
        ProcessGeneration {
            pid,
            started_at_micros,
        }
    }

    #[test]
    fn a_rejected_activation_is_retried_and_an_accepted_one_is_not() {
        let mut attempts = ActivationAttempts::default();
        let first = process(7, 1_000);
        assert!(attempts.claim(first));
        assert!(
            !attempts.claim(first),
            "an activation in progress is not repeated"
        );
        attempts.settle(&first, &[]);
        assert!(
            attempts.claim(first),
            "a process that accepted nothing is retried"
        );
        attempts.settle(&first, &["AXManualAccessibility"]);
        assert!(
            !attempts.claim(first),
            "an accepted activation is made once"
        );
        assert!(attempts.claim(process(8, 1_000)), "claims are per process");
        assert!(
            attempts.claim(process(7, 2_000)),
            "a later process holding a recycled pid is still activated"
        );
    }

    #[test]
    fn dormancy_is_a_shallow_walk_or_an_empty_web_area() {
        let shallow = WalkStats {
            read: 6,
            ..WalkStats::default()
        };
        assert!(looks_dormant(&shallow));
        let woken = WalkStats {
            read: 400,
            ..WalkStats::default()
        };
        assert!(!looks_dormant(&woken));
        let empty_web = WalkStats {
            read: 120,
            empty_web_areas: 1,
            ..WalkStats::default()
        };
        assert!(looks_dormant(&empty_web));
    }
}
