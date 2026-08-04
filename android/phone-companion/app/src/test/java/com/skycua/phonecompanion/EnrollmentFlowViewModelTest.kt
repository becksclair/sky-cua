package com.skycua.phonecompanion

import com.skycua.phonecompanion.direct.DeviceCredential
import com.skycua.phonecompanion.direct.EnrollmentCodec
import com.skycua.phonecompanion.direct.EnrollmentOutcome
import com.skycua.phonecompanion.direct.EnrollmentPayload
import com.skycua.phonecompanion.direct.EnrollmentResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.UUID

class EnrollmentFlowViewModelTest {
    @Test
    fun recreationRetainsConnectingStateAndNeverNotifiesDetachedActivity() {
        val runner = FakeRunner()
        var connected = 0
        val flow = flow(runner = runner, onConnected = { connected += 1 })
        val firstActivityStates = mutableListOf<EnrollmentScreenState>()
        val firstObserver = flow.attach(firstActivityStates::add)
        val link = link(endpoint = "ws://100.70.0.1:47684/phone/control")

        flow.acceptInitialLink(link)
        flow.confirmEndpoint()
        assertEquals(EnrollmentScreenPhase.CONNECTING, flow.screenState.phase)
        assertEquals(1, runner.requests.size)
        val firstActivityCountAtDestroy = firstActivityStates.size

        flow.detach(firstObserver)
        val recreatedActivityStates = mutableListOf<EnrollmentScreenState>()
        flow.attach(recreatedActivityStates::add)
        assertEquals(EnrollmentScreenPhase.CONNECTING, recreatedActivityStates.last().phase)

        runner.completeSuccess(0)

        assertEquals(firstActivityCountAtDestroy, firstActivityStates.size)
        assertEquals(EnrollmentScreenPhase.SUCCESS, recreatedActivityStates.last().phase)
        assertEquals(1, connected)
        assertFalse(flow.screenState.toString().contains(CREDENTIAL))
    }

    @Test
    fun differentTicketIsDeferredUntilCurrentTransactionFinishes() {
        val runner = FakeRunner()
        val flow = flow(runner = runner)
        val first = link(endpoint = "ws://100.70.0.1:47684/phone/control")
        val second =
            link(
                endpoint = "ws://100.70.0.2:47684/phone/control",
                enrollmentId = UUID.randomUUID().toString(),
                credential = SECOND_CREDENTIAL,
            )

        flow.acceptInitialLink(first)
        flow.confirmEndpoint()
        flow.offerLink(second)

        assertEquals(1, runner.requests.size)
        assertEquals(EnrollmentScreenPhase.CONNECTING, flow.screenState.phase)
        assertEquals(EnrollmentNotice.FINISHING_CURRENT, flow.screenState.notice)

        runner.completeSuccess(0)

        assertEquals(1, runner.requests.size)
        assertEquals(EnrollmentScreenPhase.REVIEW, flow.screenState.phase)
        assertEquals("ws://100.70.0.2:47684/phone/control", flow.screenState.endpoint)
        assertFalse(flow.screenState.toString().contains(SECOND_CREDENTIAL))

        flow.confirmEndpoint()
        assertEquals(2, runner.requests.size)
        assertEquals(EnrollmentScreenPhase.CONNECTING, flow.screenState.phase)
    }

    @Test
    fun replacementConfirmationSurvivesObserverRecreation() {
        val runner = FakeRunner()
        val flow = flow(runner = runner, fingerprint = { "existing-fingerprint" })
        val firstObserver = flow.attach { }

        flow.acceptInitialLink(link())
        flow.confirmEndpoint()
        assertEquals(EnrollmentScreenPhase.REPLACE, flow.screenState.phase)
        assertEquals(0, runner.requests.size)

        flow.detach(firstObserver)
        val recreatedStates = mutableListOf<EnrollmentScreenState>()
        flow.attach(recreatedStates::add)
        assertEquals(EnrollmentScreenPhase.REPLACE, recreatedStates.last().phase)

        flow.submitPending()
        assertEquals(1, runner.requests.size)
        assertEquals(EnrollmentScreenPhase.CONNECTING, flow.screenState.phase)
    }

    @Test
    fun repeatedInitialIntentDoesNotReplayTicketAfterRecreation() {
        val flow = flow()
        val first = link(endpoint = "ws://100.70.0.1:47684/phone/control")
        val second =
            link(
                endpoint = "ws://100.70.0.2:47684/phone/control",
                enrollmentId = UUID.randomUUID().toString(),
            )

        flow.acceptInitialLink(first)
        flow.acceptInitialLink(second)

        assertEquals("ws://100.70.0.1:47684/phone/control", flow.screenState.endpoint)
    }

    private fun flow(
        runner: FakeRunner = FakeRunner(),
        fingerprint: () -> String? = { null },
        onConnected: () -> Unit = {},
    ): EnrollmentFlowViewModel =
        EnrollmentFlowViewModel(
            transactionRunner = runner,
            fingerprintProvider = fingerprint,
            onConnected = onConnected,
            nowMs = { NOW_MS },
            dispatcher = EnrollmentUiDispatcher { it() },
        )

    private fun link(
        endpoint: String = "ws://100.70.0.1:47684/phone/control",
        enrollmentId: String = UUID.randomUUID().toString(),
        credential: String = CREDENTIAL,
    ): String =
        EnrollmentCodec.encode(
            EnrollmentPayload(
                protocol = "phone-control.v2",
                endpoint = endpoint,
                enrollmentId = enrollmentId,
                bootstrapCredential = credential,
                expiresAtMs = EXPIRES_AT_MS,
            ),
        )

    private class FakeRunner : EnrollmentTransactionRunner {
        data class Request(
            val payload: EnrollmentPayload,
            val callback: (EnrollmentOutcome) -> Unit,
        )

        val requests = mutableListOf<Request>()

        override fun redeem(payload: EnrollmentPayload, callback: (EnrollmentOutcome) -> Unit) {
            requests += Request(payload, callback)
        }

        fun completeSuccess(index: Int) {
            val request = requests[index]
            request.callback(
                EnrollmentOutcome.Success(
                    EnrollmentResult(
                        credential = DeviceCredential(request.payload.enrollmentId, ByteArray(32) { 7 }),
                        endpoint = request.payload.endpoint,
                    ),
                ),
            )
        }
    }

    private companion object {
        const val NOW_MS = 1_000L
        const val EXPIRES_AT_MS = 10_000L
        const val CREDENTIAL = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        const val SECOND_CREDENTIAL = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
    }
}
