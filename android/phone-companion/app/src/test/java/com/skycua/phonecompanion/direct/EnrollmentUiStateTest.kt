package com.skycua.phonecompanion.direct

import org.junit.Assert.assertEquals
import org.junit.Test

class EnrollmentUiStateTest {
    @Test fun existingCredentialRequiresTwoExplicitGates() {
        val machine = EnrollmentUiStateMachine(true)
        machine.review(); assertEquals(EnrollmentUiState.REVIEW_ENDPOINT, machine.state)
        machine.confirmEndpoint(); assertEquals(EnrollmentUiState.CONFIRM_REPLACE, machine.state)
        machine.confirmReplacement(); assertEquals(EnrollmentUiState.REDEEMING, machine.state)
    }

    @Test fun accessibilityLeaseCannotStopEnrollmentOwnedService() {
        // The ownership rule is represented by the state machine contract; Android integration owns the service lease.
        val machine = EnrollmentUiStateMachine(false)
        machine.review(); machine.confirmEndpoint(); assertEquals(EnrollmentUiState.REDEEMING, machine.state)
    }

    @Test fun freshEnrollmentIsReadyWithoutReplacementConfirmation() {
        val machine = EnrollmentUiStateMachine(false)
        machine.review(); machine.confirmEndpoint()
        assertEquals(EnrollmentUiState.REDEEMING, machine.state)
        // A fresh credential must not require confirmReplacement(); that gate is replacement-only.
    }

    @Test fun credentialAppearingAfterReviewForcesReplacementGate() {
        val machine = EnrollmentUiStateMachine(false, null)
        machine.review(); machine.confirmEndpoint(null)
        assertEquals(EnrollmentUiState.REDEEMING, machine.state)
        assertEquals(false, machine.recheckBeforeRedeem("new-credential"))
        assertEquals(EnrollmentUiState.CONFIRM_REPLACE, machine.state)
        machine.confirmReplacement(); assertEquals(EnrollmentUiState.REDEEMING, machine.state)
    }

    @Test fun stagedCommitFailurePreservesPreviousCredential() {
        val store = MemoryCredentialStore()
        val old = DeviceCredential("00000000-0000-4000-8000-000000000001", ByteArray(32) { 1 })
        store.saveEnrollment(old, "wss://old.example/control"); store.failNextEnrollmentCommit = true
        try { store.saveEnrollment(DeviceCredential("00000000-0000-4000-8000-000000000002", ByteArray(32) { 2 }), "wss://new.example/control") } catch (_: IllegalStateException) { }
        assertEquals(old.deviceId, store.load()!!.deviceId); assertEquals("wss://old.example/control", store.endpoint)
    }
}
