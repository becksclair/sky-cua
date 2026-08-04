package com.skycua.phonecompanion

import android.os.Parcelable
import android.util.SparseArray
import android.widget.EditText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
class EnrollmentCredentialHierarchyStateTest {
    @Test
    fun typedCredentialIsAbsentFromHierarchyStateAndRecreation() {
        val controller = Robolectric.buildActivity(EnrollmentActivity::class.java).setup()
        val firstActivity = controller.get()
        val firstInput =
            firstActivity.window.decorView.findViewWithTag<EditText>(ENROLLMENT_INPUT_TAG)
        firstInput.setText(SECRET)

        assertFalse(firstInput.isSaveEnabled)
        assertFalse(firstInput.isSaveFromParentEnabled)
        val hierarchyState = SparseArray<Parcelable>()
        firstActivity.window.decorView.saveHierarchyState(hierarchyState)
        assertNull("credential field must not enter hierarchy state", hierarchyState[firstInput.id])

        controller.recreate()

        val recreatedInput =
            controller.get().window.decorView.findViewWithTag<EditText>(ENROLLMENT_INPUT_TAG)
        assertEquals("", recreatedInput.text.toString())
        assertFalse(hierarchyState.toString().contains(SECRET))
    }

    private companion object {
        const val SECRET = "do-not-save-this-enrollment-credential"
    }
}
