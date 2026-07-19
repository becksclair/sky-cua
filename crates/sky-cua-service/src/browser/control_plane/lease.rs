use super::operation::{GroupId, Principal};

pub(crate) const IDLE_LEASE_MS: u64 = 30 * 60 * 1_000;
pub(crate) const DISCONNECT_GRACE_MS: u64 = 10 * 60 * 1_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LeaseProof {
    pub(crate) lease_id: String,
    pub(crate) group_id: GroupId,
    pub(crate) fence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LeaseState {
    Active,
    OrphanedGrace { grace_until_ms: u64 },
    ExpiryPending,
    Suspended,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LeaseSnapshot {
    pub(crate) lease_id: String,
    pub(crate) principal: Principal,
    pub(crate) group_id: GroupId,
    pub(crate) fence: u64,
    pub(crate) expires_at_ms: u64,
    pub(crate) state: LeaseState,
}

impl LeaseSnapshot {
    pub(crate) fn proof(&self) -> LeaseProof {
        LeaseProof {
            lease_id: self.lease_id.clone(),
            group_id: self.group_id.clone(),
            fence: self.fence,
        }
    }

    pub(crate) fn renew(&mut self, now_ms: u64) {
        self.expires_at_ms = now_ms.saturating_add(IDLE_LEASE_MS);
        self.state = LeaseState::Active;
    }

    pub(crate) fn disconnect(&mut self, now_ms: u64) {
        let grace_until_ms = self
            .expires_at_ms
            .min(now_ms.saturating_add(DISCONNECT_GRACE_MS));
        self.state = LeaseState::OrphanedGrace { grace_until_ms };
    }

    pub(crate) fn blocks_admission(&self, now_ms: u64) -> bool {
        !matches!(self.state, LeaseState::Active) || now_ms >= self.expires_at_ms
    }
}
