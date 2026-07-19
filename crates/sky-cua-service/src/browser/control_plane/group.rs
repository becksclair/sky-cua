use std::collections::{BTreeSet, HashMap};

use sky_cua_platform::model::{BrowserControlGroupSummary, BrowserLeaseState, BrowserTabKey};

use super::{
    lease::{LeaseProof, LeaseSnapshot, LeaseState},
    operation::{BrowserInstanceId, GroupId, OperationId, Principal, TabKey},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HandoffOffer {
    pub(crate) target: Principal,
    pub(crate) membership_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GroupAdmission {
    Open,
    HandoffPending(HandoffOffer),
    ExpiryPending,
    SettlementPending,
    RecoveryRequired,
    Suspended,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GroupSnapshot {
    pub(crate) group_id: GroupId,
    pub(crate) browser_instance_id: BrowserInstanceId,
    pub(crate) members: BTreeSet<TabKey>,
    pub(crate) membership_revision: u64,
    pub(crate) lease: LeaseSnapshot,
    pub(crate) admission: GroupAdmission,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GroupError {
    UnknownGroup,
    WrongBrowserInstance,
    WrongPrincipal,
    StaleFence,
    StaleMembershipRevision,
    AdmissionClosed,
    DifferentUid,
    SettlementRequired,
    InFlight,
    NoHandoffOffer,
    WrongHandoffTarget,
    RecoveryIdentityMismatch,
}

#[derive(Default)]
pub(crate) struct GroupRegistry {
    groups: HashMap<GroupId, GroupSnapshot>,
    pending_settlements: HashMap<GroupId, BTreeSet<OperationId>>,
    unknown_settlements: HashMap<GroupId, BTreeSet<OperationId>>,
    deferred_admission: HashMap<GroupId, GroupAdmission>,
    next_lease: u64,
}

impl GroupRegistry {
    pub(crate) fn introspection_summaries(
        &self,
        limit: usize,
        member_limit: usize,
        in_flight: impl Fn(&GroupId) -> usize,
    ) -> (Vec<BrowserControlGroupSummary>, u32) {
        let mut groups = self.groups.values().collect::<Vec<_>>();
        groups.sort_by(|left, right| left.group_id.0.cmp(&right.group_id.0));
        let omitted = groups.len().saturating_sub(limit);
        let summaries = groups
            .into_iter()
            .take(limit)
            .map(|group| {
                let members = group
                    .members
                    .iter()
                    .take(member_limit)
                    .map(|tab| BrowserTabKey {
                        browser_instance_id: tab.browser_instance_id.0.clone(),
                        extension_tab_id: tab.tab_id.clone(),
                    })
                    .collect::<Vec<_>>();
                BrowserControlGroupSummary {
                    group_id: group.group_id.0.clone(),
                    browser_instance_id: group.browser_instance_id.0.clone(),
                    owner_principal_id: group.lease.principal.id.clone(),
                    lease_id: group.lease.lease_id.clone(),
                    lease_state: lease_state(group),
                    fence: group.lease.fence,
                    expires_at_ms: group.lease.expires_at_ms,
                    admission_state: admission_state(&group.admission).to_owned(),
                    membership_revision: group.membership_revision,
                    member_count: bounded_u32(group.members.len()),
                    members_omitted: bounded_u32(group.members.len().saturating_sub(members.len())),
                    members,
                    in_flight_count: bounded_u32(in_flight(&group.group_id)),
                    settlement_pending_count: bounded_u32(
                        self.pending_settlements
                            .get(&group.group_id)
                            .map_or(0, BTreeSet::len),
                    ),
                    settlement_unknown_count: bounded_u32(
                        self.unknown_settlements
                            .get(&group.group_id)
                            .map_or(0, BTreeSet::len),
                    ),
                }
            })
            .collect();
        (summaries, bounded_u32(omitted))
    }

    pub(crate) fn settlement_counts(&self) -> (usize, usize) {
        (
            self.pending_settlements.values().map(BTreeSet::len).sum(),
            self.unknown_settlements.values().map(BTreeSet::len).sum(),
        )
    }

    pub(crate) fn create(
        &mut self,
        group_id: GroupId,
        browser_instance_id: BrowserInstanceId,
        principal: Principal,
        now_ms: u64,
    ) -> GroupSnapshot {
        self.next_lease += 1;
        let lease = LeaseSnapshot {
            lease_id: format!("lease-{}", self.next_lease),
            principal,
            group_id: group_id.clone(),
            fence: 1,
            expires_at_ms: now_ms.saturating_add(super::lease::IDLE_LEASE_MS),
            state: LeaseState::Active,
        };
        let group = GroupSnapshot {
            group_id: group_id.clone(),
            browser_instance_id,
            members: BTreeSet::new(),
            membership_revision: 0,
            lease,
            admission: GroupAdmission::Open,
        };
        self.groups.insert(group_id, group.clone());
        group
    }

    pub(crate) fn insert_recovered(&mut self, mut group: GroupSnapshot) {
        if !matches!(group.admission, GroupAdmission::RecoveryRequired) {
            group.admission = GroupAdmission::Suspended;
        }
        group.lease.state = LeaseState::Suspended;
        self.groups.insert(group.group_id.clone(), group);
    }

    pub(crate) fn get(&self, group_id: &GroupId) -> Result<&GroupSnapshot, GroupError> {
        self.groups.get(group_id).ok_or(GroupError::UnknownGroup)
    }

    pub(crate) fn get_mut(&mut self, group_id: &GroupId) -> Result<&mut GroupSnapshot, GroupError> {
        self.groups
            .get_mut(group_id)
            .ok_or(GroupError::UnknownGroup)
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = &GroupSnapshot> {
        self.groups.values()
    }

    pub(crate) fn browser_lost(&mut self, browser: &BrowserInstanceId) -> Vec<GroupId> {
        let mut affected = Vec::new();
        for group in self.groups.values_mut() {
            if &group.browser_instance_id != browser
                || matches!(group.admission, GroupAdmission::Released)
            {
                continue;
            }
            if !group.members.is_empty() {
                group.members.clear();
                group.membership_revision = group.membership_revision.saturating_add(1);
            }
            group.lease.fence = group.lease.fence.saturating_add(1);
            group.lease.state = LeaseState::Suspended;
            group.admission = GroupAdmission::RecoveryRequired;
            affected.push(group.group_id.clone());
        }
        affected
    }

    pub(crate) fn add_member(
        &mut self,
        group_id: &GroupId,
        principal: &Principal,
        tab: TabKey,
    ) -> Result<GroupSnapshot, GroupError> {
        let group = self.get_mut(group_id)?;
        Self::check_open_owner(group, principal)?;
        if tab.browser_instance_id != group.browser_instance_id {
            return Err(GroupError::WrongBrowserInstance);
        }
        if group.members.insert(tab) {
            group.membership_revision += 1;
        }
        Ok(group.clone())
    }

    pub(crate) fn validate(
        &self,
        proof: &LeaseProof,
        principal: &Principal,
        tab: Option<&TabKey>,
        now_ms: u64,
    ) -> Result<(), GroupError> {
        let group = self.get(&proof.group_id)?;
        Self::check_open_owner(group, principal)?;
        if group.lease.lease_id != proof.lease_id || group.lease.fence != proof.fence {
            return Err(GroupError::StaleFence);
        }
        if group.lease.blocks_admission(now_ms) {
            return Err(GroupError::AdmissionClosed);
        }
        if tab.is_some_and(|tab| !group.members.contains(tab)) {
            return Err(GroupError::WrongBrowserInstance);
        }
        Ok(())
    }

    pub(crate) fn validate_bridge_global(
        &self,
        group_id: &GroupId,
        principal: &Principal,
        browser: &BrowserInstanceId,
        now_ms: u64,
    ) -> Result<(), GroupError> {
        let group = self.get(group_id)?;
        Self::check_open_owner(group, principal)?;
        if &group.browser_instance_id != browser {
            return Err(GroupError::WrongBrowserInstance);
        }
        if group.lease.blocks_admission(now_ms) {
            return Err(GroupError::AdmissionClosed);
        }
        Ok(())
    }

    pub(crate) fn renew(
        &mut self,
        proof: &LeaseProof,
        principal: &Principal,
        now_ms: u64,
    ) -> Result<LeaseSnapshot, GroupError> {
        let group = self.get_mut(&proof.group_id)?;
        if group.lease.principal != *principal {
            return Err(GroupError::WrongPrincipal);
        }
        if group.lease.lease_id != proof.lease_id || group.lease.fence != proof.fence {
            return Err(GroupError::StaleFence);
        }
        let reconnecting_inside_grace = match group.lease.state {
            LeaseState::OrphanedGrace { grace_until_ms } if now_ms < grace_until_ms => true,
            LeaseState::OrphanedGrace { .. } => return Err(GroupError::AdmissionClosed),
            _ => false,
        };
        match group.admission {
            GroupAdmission::Open => {}
            GroupAdmission::Suspended => return Err(GroupError::AdmissionClosed),
            GroupAdmission::RecoveryRequired | GroupAdmission::Released => {
                return Err(GroupError::SettlementRequired);
            }
            GroupAdmission::HandoffPending(_)
            | GroupAdmission::ExpiryPending
            | GroupAdmission::SettlementPending
                if reconnecting_inside_grace =>
            {
                group.lease.renew(now_ms);
                return Ok(group.lease.clone());
            }
            GroupAdmission::HandoffPending(_)
            | GroupAdmission::ExpiryPending
            | GroupAdmission::SettlementPending => {
                return Err(GroupError::AdmissionClosed);
            }
        }
        group.admission = GroupAdmission::Open;
        group.lease.renew(now_ms);
        Ok(group.lease.clone())
    }

    pub(crate) fn mark_disconnected(&mut self, principal: &Principal, now_ms: u64) {
        for group in self.groups.values_mut() {
            if group.lease.principal == *principal
                && !matches!(group.lease.state, LeaseState::Released)
            {
                group.lease.disconnect(now_ms);
            }
        }
    }

    pub(crate) fn resume_recovered(
        &mut self,
        group_id: &GroupId,
        browser: &BrowserInstanceId,
        principal: &Principal,
        members: &BTreeSet<TabKey>,
        expected_revision: u64,
        now_ms: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        {
            let group = self.get(group_id)?;
            if matches!(group.admission, GroupAdmission::RecoveryRequired) {
                return Err(GroupError::SettlementRequired);
            }
            if !matches!(group.admission, GroupAdmission::Suspended) {
                return Err(GroupError::AdmissionClosed);
            }
            if &group.browser_instance_id != browser
                || &group.lease.principal != principal
                || &group.members != members
                || group.membership_revision != expected_revision
            {
                return Err(GroupError::RecoveryIdentityMismatch);
            }
        }
        self.next_lease = self.next_lease.saturating_add(1);
        let lease_id = format!("lease-{}", self.next_lease);
        let group = self.get_mut(group_id)?;
        group.lease.lease_id = lease_id;
        group.lease.renew(now_ms);
        group.admission = GroupAdmission::Open;
        Ok(group.clone())
    }

    pub(crate) fn offer_handoff(
        &mut self,
        group_id: &GroupId,
        principal: &Principal,
        target: Principal,
        expected_revision: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        let group = self.get_mut(group_id)?;
        Self::check_open_owner(group, principal)?;
        if group.membership_revision != expected_revision {
            return Err(GroupError::StaleMembershipRevision);
        }
        group.admission = GroupAdmission::HandoffPending(HandoffOffer {
            target,
            membership_revision: expected_revision,
        });
        Ok(group.clone())
    }

    pub(crate) fn accept_handoff(
        &mut self,
        group_id: &GroupId,
        target: &Principal,
        expected_revision: u64,
        in_flight: usize,
        now_ms: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        if in_flight != 0 {
            return Err(GroupError::InFlight);
        }
        let group = self.get_mut(group_id)?;
        let GroupAdmission::HandoffPending(offer) = &group.admission else {
            return Err(GroupError::NoHandoffOffer);
        };
        if offer.target != *target {
            return Err(GroupError::WrongHandoffTarget);
        }
        if offer.membership_revision != expected_revision
            || group.membership_revision != expected_revision
        {
            return Err(GroupError::StaleMembershipRevision);
        }
        Self::move_owner(group, target.clone(), now_ms);
        Ok(group.clone())
    }

    pub(crate) fn force_handoff(
        &mut self,
        group_id: &GroupId,
        requester: &Principal,
        target: Principal,
        expected_revision: u64,
        in_flight: usize,
        now_ms: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        if self.has_unresolved_settlement(group_id)
            || matches!(
                self.get(group_id)?.admission,
                GroupAdmission::RecoveryRequired | GroupAdmission::SettlementPending
            )
        {
            return Err(GroupError::SettlementRequired);
        }
        let group = self.get_mut(group_id)?;
        if requester.uid != group.lease.principal.uid || target.uid != requester.uid {
            return Err(GroupError::DifferentUid);
        }
        if group.membership_revision != expected_revision {
            return Err(GroupError::StaleMembershipRevision);
        }
        if in_flight != 0 {
            return Err(GroupError::InFlight);
        }
        Self::move_owner(group, target, now_ms);
        Ok(group.clone())
    }

    pub(crate) fn begin_settlement(&mut self, group_id: &GroupId, operation_id: OperationId) {
        if !self.has_unresolved_settlement(group_id)
            && let Some(group) = self.groups.get(group_id)
        {
            self.deferred_admission
                .insert(group_id.clone(), group.admission.clone());
        }
        self.pending_settlements
            .entry(group_id.clone())
            .or_default()
            .insert(operation_id);
        if let Some(group) = self.groups.get_mut(group_id) {
            group.admission = GroupAdmission::SettlementPending;
        }
    }

    pub(crate) fn mark_settlement_unknown(
        &mut self,
        group_id: &GroupId,
        operation_id: &OperationId,
    ) {
        if self
            .pending_settlements
            .get(group_id)
            .is_some_and(|pending| pending.contains(operation_id))
        {
            self.unknown_settlements
                .entry(group_id.clone())
                .or_default()
                .insert(operation_id.clone());
            if let Some(group) = self.groups.get_mut(group_id) {
                group.admission = GroupAdmission::RecoveryRequired;
            }
        }
    }

    pub(crate) fn finish_settlement(&mut self, group_id: &GroupId, operation_id: &OperationId) {
        if let Some(pending) = self.pending_settlements.get_mut(group_id) {
            pending.remove(operation_id);
            if pending.is_empty() {
                self.pending_settlements.remove(group_id);
            }
        }
        if let Some(unknown) = self.unknown_settlements.get_mut(group_id) {
            unknown.remove(operation_id);
            if unknown.is_empty() {
                self.unknown_settlements.remove(group_id);
            }
        }
        if self.has_unresolved_settlement(group_id) {
            if let Some(group) = self.groups.get_mut(group_id) {
                group.admission = if self.unknown_settlements.contains_key(group_id) {
                    GroupAdmission::RecoveryRequired
                } else {
                    GroupAdmission::SettlementPending
                };
            }
            return;
        }
        let deferred_admission = self
            .deferred_admission
            .remove(group_id)
            .unwrap_or(GroupAdmission::Open);
        if let Some(group) = self.groups.get_mut(group_id) {
            match deferred_admission {
                GroupAdmission::ExpiryPending => Self::release_expired(group),
                GroupAdmission::HandoffPending(offer) => {
                    group.admission = GroupAdmission::HandoffPending(offer);
                }
                _ => group.admission = GroupAdmission::Open,
            }
        }
    }

    pub(crate) fn finish_execution(&mut self, group_id: &GroupId) {
        if self.has_unresolved_settlement(group_id) {
            return;
        }
        if let Some(group) = self.groups.get_mut(group_id)
            && matches!(group.admission, GroupAdmission::ExpiryPending)
        {
            Self::release_expired(group);
        }
    }

    pub(crate) fn remove_target(&mut self, group_id: &GroupId, tab: &TabKey) {
        if let Some(group) = self.groups.get_mut(group_id)
            && group.members.remove(tab)
        {
            group.membership_revision += 1;
        }
    }

    pub(crate) fn remove_browser_targets(
        &mut self,
        group_id: &GroupId,
        browser: &BrowserInstanceId,
    ) {
        if let Some(group) = self.groups.get_mut(group_id)
            && &group.browser_instance_id == browser
            && !group.members.is_empty()
        {
            group.members.clear();
            group.membership_revision += 1;
        }
    }

    pub(crate) fn has_unresolved_settlement(&self, group_id: &GroupId) -> bool {
        self.pending_settlements
            .get(group_id)
            .is_some_and(|pending| !pending.is_empty())
    }

    pub(crate) fn end_lifecycle(
        &mut self,
        group_id: &GroupId,
        principal: &Principal,
        in_flight: usize,
    ) -> Result<GroupSnapshot, GroupError> {
        let has_settlement = self.has_unresolved_settlement(group_id);
        if self.get(group_id)?.lease.principal != *principal {
            return Err(GroupError::WrongPrincipal);
        }
        if has_settlement {
            self.deferred_admission
                .insert(group_id.clone(), GroupAdmission::ExpiryPending);
        }
        let group = self.get_mut(group_id)?;
        group.admission = GroupAdmission::ExpiryPending;
        group.lease.state = LeaseState::ExpiryPending;
        if !has_settlement && in_flight == 0 {
            Self::release_expired(group);
        }
        Ok(group.clone())
    }

    pub(crate) fn expire(&mut self, now_ms: u64, in_flight: impl Fn(&GroupId) -> usize) -> bool {
        let mut changed = false;
        for group in self.groups.values_mut() {
            // Restart hints carry no live lease authority and therefore no
            // meaningful idle deadline. Keep them suspended until an exact
            // identity/membership reconciliation resumes them explicitly.
            if matches!(group.lease.state, LeaseState::Suspended) {
                continue;
            }
            let grace_expired = matches!(
                group.lease.state,
                LeaseState::OrphanedGrace { grace_until_ms } if now_ms >= grace_until_ms
            );
            let idle_expired = now_ms >= group.lease.expires_at_ms;
            if (grace_expired || idle_expired)
                && !matches!(group.admission, GroupAdmission::Released)
            {
                if self
                    .pending_settlements
                    .get(&group.group_id)
                    .is_some_and(|pending| !pending.is_empty())
                {
                    self.deferred_admission
                        .insert(group.group_id.clone(), GroupAdmission::ExpiryPending);
                    group.lease.state = LeaseState::ExpiryPending;
                    changed = true;
                    continue;
                }
                if matches!(group.admission, GroupAdmission::RecoveryRequired) {
                    continue;
                }
                group.admission = GroupAdmission::ExpiryPending;
                group.lease.state = LeaseState::ExpiryPending;
                changed = true;
                if in_flight(&group.group_id) == 0 {
                    group.lease.fence += 1;
                    group.admission = GroupAdmission::Released;
                    group.lease.state = LeaseState::Released;
                }
            }
        }
        changed
    }

    fn check_open_owner(group: &GroupSnapshot, principal: &Principal) -> Result<(), GroupError> {
        if group.lease.principal != *principal {
            return Err(GroupError::WrongPrincipal);
        }
        if !matches!(group.admission, GroupAdmission::Open) {
            return Err(
                if matches!(
                    group.admission,
                    GroupAdmission::RecoveryRequired | GroupAdmission::SettlementPending
                ) {
                    GroupError::SettlementRequired
                } else {
                    GroupError::AdmissionClosed
                },
            );
        }
        Ok(())
    }

    fn move_owner(group: &mut GroupSnapshot, target: Principal, now_ms: u64) {
        group.lease.principal = target;
        group.lease.fence += 1;
        group.lease.renew(now_ms);
        group.admission = GroupAdmission::Open;
    }

    fn release_expired(group: &mut GroupSnapshot) {
        group.lease.fence += 1;
        group.admission = GroupAdmission::Released;
        group.lease.state = LeaseState::Released;
    }
}

fn admission_state(admission: &GroupAdmission) -> &'static str {
    match admission {
        GroupAdmission::Open => "open",
        GroupAdmission::HandoffPending(_) => "handoff_pending",
        GroupAdmission::ExpiryPending => "expiry_pending",
        GroupAdmission::SettlementPending => "settlement_pending",
        GroupAdmission::RecoveryRequired => "recovery_required",
        GroupAdmission::Suspended => "suspended",
        GroupAdmission::Released => "released",
    }
}

fn lease_state(group: &GroupSnapshot) -> BrowserLeaseState {
    match group.admission {
        GroupAdmission::HandoffPending(_) => BrowserLeaseState::HandoffPending,
        GroupAdmission::SettlementPending | GroupAdmission::RecoveryRequired => {
            BrowserLeaseState::RecoveryRequired
        }
        _ => match group.lease.state {
            LeaseState::Active => BrowserLeaseState::Active,
            LeaseState::OrphanedGrace { .. } => BrowserLeaseState::OrphanedGrace,
            LeaseState::ExpiryPending => BrowserLeaseState::ExpiryPending,
            LeaseState::Suspended => BrowserLeaseState::Suspended,
            LeaseState::Released => BrowserLeaseState::Released,
        },
    }
}

fn bounded_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
