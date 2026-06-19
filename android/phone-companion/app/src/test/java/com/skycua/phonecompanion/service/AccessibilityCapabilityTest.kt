package com.skycua.phonecompanion.service

import android.accessibilityservice.AccessibilityServiceInfo
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AccessibilityCapabilityTest {
    @Test
    fun missingServiceInfoDoesNotAdvertiseCapabilities() {
        assertFalse(
            SkyAccessibilityService.capabilityEnabled(
                capabilities = null,
                capability = AccessibilityServiceInfo.CAPABILITY_CAN_PERFORM_GESTURES,
            ),
        )
    }

    @Test
    fun capabilityBitMustBePresent() {
        assertFalse(
            SkyAccessibilityService.capabilityEnabled(
                capabilities = AccessibilityServiceInfo.CAPABILITY_CAN_RETRIEVE_WINDOW_CONTENT,
                capability = AccessibilityServiceInfo.CAPABILITY_CAN_PERFORM_GESTURES,
            ),
        )
        assertTrue(
            SkyAccessibilityService.capabilityEnabled(
                capabilities =
                    AccessibilityServiceInfo.CAPABILITY_CAN_RETRIEVE_WINDOW_CONTENT or
                        AccessibilityServiceInfo.CAPABILITY_CAN_PERFORM_GESTURES,
                capability = AccessibilityServiceInfo.CAPABILITY_CAN_PERFORM_GESTURES,
            ),
        )
    }
}
