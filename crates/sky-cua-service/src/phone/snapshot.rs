//! Phone snapshot ids and the per-manager snapshot registry.
//!
//! Every capture (`phone_screenshot`, `phone_observe`) mints a
//! `phone_snapshot_id` and registers a [`PhoneSnapshotRecord`] describing the
//! frame: backend, session, serial, device size, orientation, coordinate
//! mapping id, and capture timestamp. Later coordinate actions (`phone_tap`,
//! `phone_swipe`) reference that id, and the manager resolves it through
//! [`PhoneSnapshotRegistry::resolve`], which rejects stale ids, ids minted for a
//! different session/serial, and ids the registry has already evicted.
//!
//! The registry is bounded: it retains at most `capacity` records and evicts the
//! oldest when full, so a long session cannot grow snapshot memory without
//! limit. Resolution is also TTL-gated: a snapshot older than `ttl_ms` is
//! rejected as stale even if still resident, because the device may have rotated
//! or resized since capture.
//!
//! This module owns no device I/O. The ADB/companion lanes call [`mint`] +
//! [`PhoneSnapshotRegistry::register`] after a successful capture and
//! [`PhoneSnapshotRegistry::resolve`] before dispatching a coordinate action.

use std::collections::VecDeque;

use sky_cua_platform::model::{PhoneBackendKind, PhoneCoordinateMapping, PixelSize};

/// Default number of snapshots retained per registry. A handful covers
/// observe-then-act loops with room for a couple of stale frames; beyond that
/// older frames are evicted.
pub(super) const DEFAULT_SNAPSHOT_CAPACITY: usize = 16;

/// A registered capture. `snapshot_id` is the handle returned to the agent; the
/// rest is the metadata coordinate actions validate against before mapping.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PhoneSnapshotRecord {
    pub snapshot_id: String,
    pub session_id: String,
    pub serial: String,
    pub backend: PhoneBackendKind,
    pub device_size: PixelSize,
    /// Pixel dimensions of the image the model actually saw (the delivered,
    /// possibly downscaled, model image). A coordinate action names this plane,
    /// not `device_size`, so `phone_tap`/`phone_swipe` must translate through it
    /// rather than assuming a 1:1 screenshot.
    pub screenshot_size: PixelSize,
    pub rotation_degrees: i32,
    pub mapping_id: String,
    pub captured_at_ms: u64,
}

/// Why a `phone_snapshot_id` could not be resolved into an actionable record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SnapshotError {
    /// No record with this id is resident (never minted, or already evicted).
    Unknown,
    /// The record exists but belongs to a different session.
    SessionMismatch { expected: String, found: String },
    /// The record exists but was captured against a different serial.
    SerialMismatch { expected: String, found: String },
    /// The record is older than the registry TTL; the device state it described
    /// can no longer be trusted for coordinate mapping.
    Stale { age_ms: u64, ttl_ms: u64 },
    /// The device rotated after capture: the record's captured size has the
    /// profile's current size with width/height swapped, so a coordinate from the
    /// old orientation would land in the wrong place.
    OrientationMismatch {
        captured: PixelSize,
        current: PixelSize,
    },
    /// The device resolution changed after capture (and it is not a clean
    /// orientation swap), so the snapshot's coordinate mapping no longer matches
    /// the live display.
    ResolutionMismatch {
        captured: PixelSize,
        current: PixelSize,
    },
}

impl SnapshotError {
    /// Stable diagnostic code for structured responses.
    pub(super) fn code(&self) -> &'static str {
        match self {
            SnapshotError::Unknown => "PhoneSnapshotUnknown",
            SnapshotError::SessionMismatch { .. } => "PhoneSnapshotSessionMismatch",
            SnapshotError::SerialMismatch { .. } => "PhoneSnapshotSerialMismatch",
            SnapshotError::Stale { .. } => "PhoneSnapshotStale",
            SnapshotError::OrientationMismatch { .. } => "PhoneSnapshotOrientationMismatch",
            SnapshotError::ResolutionMismatch { .. } => "PhoneSnapshotResolutionMismatch",
        }
    }
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Unknown => write!(f, "unknown or evicted phone snapshot id"),
            SnapshotError::SessionMismatch { expected, found } => {
                write!(
                    f,
                    "snapshot belongs to session {found}, expected {expected}"
                )
            }
            SnapshotError::SerialMismatch { expected, found } => {
                write!(
                    f,
                    "snapshot was captured against {found}, expected {expected}"
                )
            }
            SnapshotError::Stale { age_ms, ttl_ms } => {
                write!(f, "snapshot is {age_ms}ms old (ttl {ttl_ms}ms)")
            }
            SnapshotError::OrientationMismatch { captured, current } => {
                write!(
                    f,
                    "device rotated since capture: snapshot was {}x{}, display is now {}x{}",
                    captured.width, captured.height, current.width, current.height
                )
            }
            SnapshotError::ResolutionMismatch { captured, current } => {
                write!(
                    f,
                    "device resolution changed since capture: snapshot was {}x{}, display is now {}x{}",
                    captured.width, captured.height, current.width, current.height
                )
            }
        }
    }
}

/// Mint a unique `phone_snapshot_id` for a freshly captured frame. The id is an
/// opaque, collision-resistant handle (the mapping/session/serial it describes
/// live in the registered record, not in the string). Uniqueness comes from the
/// platform crate's canonical UUID minter, matching how desktop snapshots are
/// identified; the sanitized serial and timestamp prefix keep ids readable in
/// logs without leaking more than `phone_list_devices` already exposes.
pub(super) fn mint(serial: &str, captured_at_ms: u64) -> String {
    let sanitized: String = serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!(
        "phone-{sanitized}-{captured_at_ms}-{}",
        sky_cua_platform::snapshot::new_snapshot_id()
    )
}

/// Build a record from a capture's metadata and its coordinate mapping. Keeps
/// the snapshot id, device geometry, and mapping id consistent in one place so
/// the ADB/companion lanes cannot register a partially-populated record.
pub(super) fn record_from_mapping(
    snapshot_id: &str,
    backend: PhoneBackendKind,
    device_size: PixelSize,
    mapping: &PhoneCoordinateMapping,
) -> PhoneSnapshotRecord {
    PhoneSnapshotRecord {
        snapshot_id: snapshot_id.to_string(),
        session_id: mapping.session_id.clone(),
        serial: mapping.serial.clone(),
        backend,
        device_size,
        screenshot_size: PixelSize {
            width: mapping.screenshot_rect.width.round() as u32,
            height: mapping.screenshot_rect.height.round() as u32,
        },
        rotation_degrees: mapping.rotation_degrees,
        mapping_id: mapping.mapping_id.clone(),
        captured_at_ms: mapping.captured_at_ms,
    }
}

/// Bounded, TTL-gated snapshot store. One per session is the intended granularity
/// (the manager keys registries by session), but the registry still validates
/// session/serial on resolution so a shared registry is also safe.
#[derive(Debug)]
pub(super) struct PhoneSnapshotRegistry {
    records: VecDeque<PhoneSnapshotRecord>,
    capacity: usize,
    ttl_ms: u64,
}

impl PhoneSnapshotRegistry {
    /// Create a registry retaining at most `capacity` records (minimum 1) and
    /// rejecting records older than `ttl_ms` on resolution.
    pub(super) fn new(capacity: usize, ttl_ms: u64) -> Self {
        Self {
            records: VecDeque::new(),
            capacity: capacity.max(1),
            ttl_ms,
        }
    }

    /// Register a freshly captured snapshot, evicting the oldest record when the
    /// registry is at capacity. Returns the snapshot id for convenience.
    pub(super) fn register(&mut self, record: PhoneSnapshotRecord) -> String {
        let id = record.snapshot_id.clone();
        // A repeated id (shouldn't happen with uuids) replaces the prior entry.
        self.records.retain(|r| r.snapshot_id != id);
        self.records.push_back(record);
        while self.records.len() > self.capacity {
            self.records.pop_front();
        }
        id
    }

    /// Resolve a snapshot id into an actionable record, validating residency,
    /// session, serial, and freshness. `now_ms` is the wall clock the manager
    /// passes; staleness is `now_ms - captured_at_ms > ttl_ms`.
    pub(super) fn resolve(
        &self,
        snapshot_id: &str,
        session_id: &str,
        serial: &str,
        now_ms: u64,
    ) -> Result<&PhoneSnapshotRecord, SnapshotError> {
        let record = self
            .records
            .iter()
            .find(|r| r.snapshot_id == snapshot_id)
            .ok_or(SnapshotError::Unknown)?;
        if record.session_id != session_id {
            return Err(SnapshotError::SessionMismatch {
                expected: session_id.to_string(),
                found: record.session_id.clone(),
            });
        }
        if record.serial != serial {
            return Err(SnapshotError::SerialMismatch {
                expected: serial.to_string(),
                found: record.serial.clone(),
            });
        }
        let age_ms = now_ms.saturating_sub(record.captured_at_ms);
        if age_ms > self.ttl_ms {
            return Err(SnapshotError::Stale {
                age_ms,
                ttl_ms: self.ttl_ms,
            });
        }
        Ok(record)
    }

    /// The most recently registered record, if any. Used when a caller opts into
    /// "act on the latest snapshot" instead of naming an id. Not yet wired into
    /// the manager's coordinate path (which always names a snapshot id today).
    #[cfg_attr(not(test), expect(dead_code))]
    pub(super) fn latest(&self) -> Option<&PhoneSnapshotRecord> {
        self.records.back()
    }

    /// Number of resident records. Test/diagnostic accessor.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::{CoordinateSpace, RectF};

    fn size(w: u32, h: u32) -> PixelSize {
        PixelSize {
            width: w,
            height: h,
        }
    }

    fn record(id: &str, session: &str, serial: &str, captured_at_ms: u64) -> PhoneSnapshotRecord {
        PhoneSnapshotRecord {
            snapshot_id: id.to_string(),
            session_id: session.to_string(),
            serial: serial.to_string(),
            backend: PhoneBackendKind::Adb,
            device_size: size(1080, 2400),
            screenshot_size: size(1080, 2400),
            rotation_degrees: 0,
            mapping_id: format!("map-{id}"),
            captured_at_ms,
        }
    }

    #[test]
    fn mint_is_unique_and_carries_sanitized_serial() {
        let a = mint("172.16.255.58:38781", 1000);
        let b = mint("172.16.255.58:38781", 1000);
        assert_ne!(a, b, "ids must be unique even for same serial/time");
        assert!(a.contains("172-16-255-58-38781"));
        assert!(!a.contains(':'));
    }

    #[test]
    fn record_from_mapping_copies_session_serial_and_geometry() {
        let mapping = PhoneCoordinateMapping {
            mapping_id: "map-1".to_string(),
            session_id: "sess-1".to_string(),
            serial: "emulator-5554".to_string(),
            device_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1080.0,
                height: 2400.0,
                space: CoordinateSpace::StreamPixels,
            },
            screenshot_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1080.0,
                height: 2400.0,
                space: CoordinateSpace::StreamPixels,
            },
            host_window_rect: None,
            host_content_rect: None,
            rotation_degrees: 90,
            captured_at_ms: 555,
        };
        let rec = record_from_mapping(
            "snap-1",
            PhoneBackendKind::Companion,
            size(1080, 2400),
            &mapping,
        );
        assert_eq!(rec.session_id, "sess-1");
        assert_eq!(rec.serial, "emulator-5554");
        assert_eq!(rec.mapping_id, "map-1");
        assert_eq!(rec.rotation_degrees, 90);
        assert_eq!(rec.captured_at_ms, 555);
        assert_eq!(rec.backend, PhoneBackendKind::Companion);
        assert_eq!(rec.screenshot_size, size(1080, 2400));
    }

    #[test]
    fn record_from_mapping_records_downscaled_screenshot_size() {
        // The delivered model image is smaller than the device: the record must
        // carry the delivered (screenshot) plane, not the device plane, so a
        // later coordinate action scales through the right ratio.
        let mapping = PhoneCoordinateMapping {
            mapping_id: "map-2".to_string(),
            session_id: "sess-1".to_string(),
            serial: "emulator-5554".to_string(),
            device_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1080.0,
                height: 2400.0,
                space: CoordinateSpace::StreamPixels,
            },
            screenshot_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 540.0,
                height: 1200.0,
                space: CoordinateSpace::StreamPixels,
            },
            host_window_rect: None,
            host_content_rect: None,
            rotation_degrees: 0,
            captured_at_ms: 555,
        };
        let rec = record_from_mapping("snap-2", PhoneBackendKind::Adb, size(1080, 2400), &mapping);
        assert_eq!(rec.device_size, size(1080, 2400));
        assert_eq!(rec.screenshot_size, size(540, 1200));
    }

    #[test]
    fn resolves_a_fresh_matching_snapshot() {
        let mut reg = PhoneSnapshotRegistry::new(8, 30_000);
        reg.register(record("snap-1", "sess-1", "serial-1", 1_000));
        let resolved = reg
            .resolve("snap-1", "sess-1", "serial-1", 5_000)
            .expect("fresh");
        assert_eq!(resolved.snapshot_id, "snap-1");
    }

    #[test]
    fn unknown_snapshot_rejected() {
        let reg = PhoneSnapshotRegistry::new(8, 30_000);
        let err = reg
            .resolve("nope", "sess-1", "serial-1", 1_000)
            .expect_err("unknown");
        assert_eq!(err, SnapshotError::Unknown);
        assert_eq!(err.code(), "PhoneSnapshotUnknown");
    }

    #[test]
    fn session_mismatch_rejected() {
        let mut reg = PhoneSnapshotRegistry::new(8, 30_000);
        reg.register(record("snap-1", "sess-1", "serial-1", 1_000));
        let err = reg
            .resolve("snap-1", "sess-OTHER", "serial-1", 2_000)
            .expect_err("session mismatch");
        assert!(matches!(err, SnapshotError::SessionMismatch { .. }));
        assert_eq!(err.code(), "PhoneSnapshotSessionMismatch");
    }

    #[test]
    fn serial_mismatch_rejected() {
        let mut reg = PhoneSnapshotRegistry::new(8, 30_000);
        reg.register(record("snap-1", "sess-1", "serial-1", 1_000));
        let err = reg
            .resolve("snap-1", "sess-1", "serial-OTHER", 2_000)
            .expect_err("serial mismatch");
        assert!(matches!(err, SnapshotError::SerialMismatch { .. }));
        assert_eq!(err.code(), "PhoneSnapshotSerialMismatch");
    }

    #[test]
    fn stale_snapshot_rejected_past_ttl() {
        let mut reg = PhoneSnapshotRegistry::new(8, 10_000);
        reg.register(record("snap-1", "sess-1", "serial-1", 1_000));
        // age = 12_000 > ttl 10_000.
        let err = reg
            .resolve("snap-1", "sess-1", "serial-1", 13_000)
            .expect_err("stale");
        assert!(matches!(
            err,
            SnapshotError::Stale {
                age_ms: 12_000,
                ttl_ms: 10_000
            }
        ));
        assert_eq!(err.code(), "PhoneSnapshotStale");
    }

    #[test]
    fn orientation_and_resolution_mismatch_codes_are_stable() {
        // The orientation/resolution mismatch variants carry stable diagnostic
        // codes so callers route on the code, not the prose.
        let orientation = SnapshotError::OrientationMismatch {
            captured: size(1080, 2400),
            current: size(2400, 1080),
        };
        assert_eq!(orientation.code(), "PhoneSnapshotOrientationMismatch");
        assert!(orientation.to_string().contains("1080x2400"));
        assert!(orientation.to_string().contains("2400x1080"));

        let resolution = SnapshotError::ResolutionMismatch {
            captured: size(1080, 2400),
            current: size(1440, 3120),
        };
        assert_eq!(resolution.code(), "PhoneSnapshotResolutionMismatch");
        assert!(resolution.to_string().contains("1440x3120"));
    }

    #[test]
    fn boundary_age_equal_to_ttl_is_still_valid() {
        let mut reg = PhoneSnapshotRegistry::new(8, 10_000);
        reg.register(record("snap-1", "sess-1", "serial-1", 1_000));
        // age = 10_000 == ttl, not > ttl, so still resolvable.
        let resolved = reg
            .resolve("snap-1", "sess-1", "serial-1", 11_000)
            .expect("exactly at ttl");
        assert_eq!(resolved.snapshot_id, "snap-1");
    }

    #[test]
    fn capacity_bound_evicts_oldest() {
        let mut reg = PhoneSnapshotRegistry::new(2, 60_000);
        reg.register(record("a", "s", "serial", 1));
        reg.register(record("b", "s", "serial", 2));
        reg.register(record("c", "s", "serial", 3));
        assert_eq!(reg.len(), 2);
        // Oldest ("a") evicted.
        assert_eq!(
            reg.resolve("a", "s", "serial", 4).expect_err("evicted"),
            SnapshotError::Unknown
        );
        assert!(reg.resolve("b", "s", "serial", 4).is_ok());
        assert!(reg.resolve("c", "s", "serial", 4).is_ok());
        assert_eq!(reg.latest().expect("latest").snapshot_id, "c");
    }

    #[test]
    fn capacity_floor_is_one() {
        let mut reg = PhoneSnapshotRegistry::new(0, 60_000);
        reg.register(record("a", "s", "serial", 1));
        reg.register(record("b", "s", "serial", 2));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.latest().expect("latest").snapshot_id, "b");
    }

    #[test]
    fn re_registering_same_id_replaces_without_growth() {
        let mut reg = PhoneSnapshotRegistry::new(8, 60_000);
        reg.register(record("a", "s", "serial", 1));
        reg.register(record("a", "s", "serial", 9));
        assert_eq!(reg.len(), 1);
        let resolved = reg.resolve("a", "s", "serial", 10).expect("ok");
        assert_eq!(resolved.captured_at_ms, 9);
    }

    #[test]
    fn latest_is_none_when_empty() {
        let reg = PhoneSnapshotRegistry::new(8, 60_000);
        assert!(reg.latest().is_none());
    }
}
