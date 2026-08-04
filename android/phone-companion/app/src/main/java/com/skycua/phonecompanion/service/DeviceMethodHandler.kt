package com.skycua.phonecompanion.service

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import com.skycua.phonecompanion.BuildConfig
import com.skycua.phonecompanion.app.AppManager
import com.skycua.phonecompanion.json.JsonValue
import com.skycua.phonecompanion.protocol.AccessibilityTreeParams
import com.skycua.phonecompanion.protocol.AccessibilityTreeResult
import com.skycua.phonecompanion.protocol.AppListParams
import com.skycua.phonecompanion.protocol.AppListResult
import com.skycua.phonecompanion.protocol.AppOpParams
import com.skycua.phonecompanion.protocol.AppShotParams
import com.skycua.phonecompanion.protocol.CapabilitiesState
import com.skycua.phonecompanion.protocol.CurrentAppResult
import com.skycua.phonecompanion.protocol.CursorOverlayParams
import com.skycua.phonecompanion.protocol.GestureParams
import com.skycua.phonecompanion.protocol.HealthState
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.MethodHandler
import com.skycua.phonecompanion.protocol.NotificationOpParams
import com.skycua.phonecompanion.protocol.NotificationsParams
import com.skycua.phonecompanion.protocol.NotificationsResult
import com.skycua.phonecompanion.protocol.OverlayActiveParams
import com.skycua.phonecompanion.protocol.OverlayGestureParams
import com.skycua.phonecompanion.protocol.Protocol
import com.skycua.phonecompanion.protocol.ScreenshotParams
import com.skycua.phonecompanion.protocol.ScreenshotResult
import com.skycua.phonecompanion.appshot.AndroidPhoneAppShotSource
import com.skycua.phonecompanion.appshot.PhoneAppShotProducer
import com.skycua.phonecompanion.appshot.toJson
import com.skycua.phonecompanion.protocol.cursorOverlayResult
import com.skycua.phonecompanion.protocol.gestureDispatchedResult
import com.skycua.phonecompanion.protocol.okResult
import com.skycua.phonecompanion.protocol.overlayActiveResult
import com.skycua.phonecompanion.protocol.overlayGestureResult
import com.skycua.phonecompanion.direct.DirectContentResolver

/**
 * Wires the protocol [MethodHandler] to the live Android services. Each method
 * resolves the relevant service instance at call time; a disabled or unbound
 * service produces a structured application error instead of a transport
 * failure.
 */
class DeviceMethodHandler(
    private val context: Context,
    private val contentResolver: DirectContentResolver? = null,
) : MethodHandler {
    private val appManager = AppManager(context)
    private val clipboardController = ClipboardController(context, contentResolver)
    private val storageController = StorageController(context, contentResolver)
    private val cameraController = CameraController(context)

    private fun accessibility(): SkyAccessibilityService? = SkyAccessibilityService.instance()

    private fun notifications(): SkyNotificationListenerService? =
        SkyNotificationListenerService.instance()

    fun directCapabilityNames(): Set<String> {
        val state = health()
        return buildSet {
            addAll(listOf("content", "clipboard", "editor", "camera", "storage", "app_management"))
            if (state.accessibilityEnabled) add("accessibility")
            if (state.canPerformGestures) add("gestures")
            if (state.canRetrieveWindowContent) add("accessibility_tree")
            if (state.canTakeScreenshot) add("screenshot")
            if (state.notificationListenerEnabled) add("notifications")
        }
    }

    override fun health(): HealthState {
        val a11y = accessibility()
        val notif = notifications()
        return HealthState(
            version = BuildConfig.VERSION_NAME,
            versionCode = BuildConfig.VERSION_CODE,
            packageName = context.packageName,
            accessibilityEnabled = a11y != null,
            canPerformGestures = a11y?.canPerformGestures() ?: false,
            canRetrieveWindowContent = a11y?.canRetrieveWindowContent() ?: false,
            canTakeScreenshot = a11y?.canTakeScreenshot() ?: false,
            notificationListenerEnabled = notif?.listenerEnabled ?: false,
            nativeOverlay = a11y != null,
            nativeOverlayPassThrough = a11y?.overlayPassThrough() ?: true,
            privilegedSetup = null,
        )
    }

    override fun capabilities(): CapabilitiesState {
        val health = health()
        val a11y = accessibility()
        val apiLevel = a11y?.screenshotApiLevel() ?: Build.VERSION.SDK_INT
        val screenshotSupported =
            apiLevel >= Build.VERSION_CODES.R && (a11y?.canTakeScreenshot() ?: false)
        val gestureSupported =
            apiLevel >= Build.VERSION_CODES.N && (a11y?.canPerformGestures() ?: false)
        return CapabilitiesState(
            health = health,
            screenshotApiLevel = apiLevel,
            screenshotSupported = screenshotSupported,
            gestureSupported = gestureSupported,
        )
    }

    override fun accessibilityTree(params: AccessibilityTreeParams): AccessibilityTreeResult =
        requireAccessibility().captureTree(params)

    override fun screenshot(params: ScreenshotParams): ScreenshotResult =
        requireAccessibility().takePhoneScreenshot(params)

    override fun appShot(params: AppShotParams): JsonValue.Obj {
        val service = requireAccessibility()
        val captured = PhoneAppShotProducer(
            AndroidPhoneAppShotSource(service),
            maxNodes = params.maxNodes,
        ).capture()
        return captured.toJson()
    }

    override fun gesture(params: GestureParams): JsonValue.Obj {
        requireAccessibility().dispatchPhoneGesture(params)
        return gestureDispatchedResult()
    }

    override fun cursorOverlay(params: CursorOverlayParams): JsonValue.Obj {
        val service = requireAccessibility()
        val (shown, passThrough) = service.setCursorOverlay(params.visible, params.x, params.y)
        return cursorOverlayResult(shown, passThrough)
    }

    /**
     * Toggles the persistent "agent in control" edge glow. When the accessibility
     * service is unavailable the glow cannot be drawn, so this reports
     * `active=false, glow_supported=false` rather than throwing — the host treats
     * the overlay as best-effort presence, not a hard dependency.
     */
    override fun overlayActive(params: OverlayActiveParams): JsonValue.Obj {
        val service =
            accessibility()
                ?: return overlayActiveResult(active = false, glowSupported = false)
        val active = service.setOverlayActive(params.active)
        return overlayActiveResult(active = active, glowSupported = true)
    }

    /**
     * Animates the agent cursor for one action (visual only). When the
     * accessibility service is unavailable the animation cannot run, so this
     * reports `animated=false` rather than throwing.
     */
    override fun overlayGesture(params: OverlayGestureParams): JsonValue.Obj {
        val service =
            accessibility()
                ?: return overlayGestureResult(animated = false)
        val animated = service.animateOverlayGesture(params)
        return overlayGestureResult(animated = animated)
    }

    override fun notifications(params: NotificationsParams): NotificationsResult {
        val service = notifications()
        if (service == null) {
            return NotificationsResult(
                listenerEnabled = false,
                events = emptyList(),
                truncated = false,
            )
        }
        return service.snapshot(params)
    }

    override fun notificationOp(params: NotificationOpParams): JsonValue.Obj {
        val service =
            notifications()
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.DISABLED_SERVICE,
                    "notification listener is not enabled",
                )
        service.performOp(params)
        return okResult()
    }

    override fun currentApp(): CurrentAppResult {
        val service =
            accessibility()
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.DISABLED_SERVICE,
                    "foreground app requires the accessibility service",
                )
        val pkg =
            try {
                service.rootInActiveWindow?.packageName?.toString()
            } catch (_: Exception) {
                null
            }
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.DISABLED_SERVICE,
                    "foreground app requires the accessibility service",
                )
        // Best-effort component name; null when it cannot be resolved reliably.
        val activity = service.currentActivity(pkg)
        val label =
            try {
                val info = context.packageManager.getApplicationInfo(pkg, 0)
                context.packageManager.getApplicationLabel(info).toString()
            } catch (_: PackageManager.NameNotFoundException) {
                null
            }
        return CurrentAppResult(packageName = pkg, activity = activity, label = label)
    }

    override fun appList(params: AppListParams): AppListResult = appManager.listApps(params)

    override fun appOp(params: AppOpParams): JsonValue.Obj {
        appManager.perform(params)
        return okResult()
    }

    override fun clipboard(params: JsonValue.Obj): JsonValue.Obj =
        clipboardController.perform(params)

    override fun editor(params: JsonValue.Obj): JsonValue.Obj =
        if (params.string("operation") == "insert_content") {
            SkyImeService.perform(params, contentResolver)
        } else {
            requireAccessibility().performEditorOperation(params)
        }

    override fun storage(params: JsonValue.Obj): JsonValue.Obj =
        storageController.perform(params)

    override fun camera(params: JsonValue.Obj): JsonValue.Obj =
        cameraController.perform(params)

    override fun key(params: JsonValue.Obj): JsonValue.Obj =
        requireAccessibility().performKey(params.string("key") ?: "")

    private fun requireAccessibility(): SkyAccessibilityService =
        accessibility()
            ?: throw MethodApplicationException(
                Protocol.ErrorCodes.DISABLED_SERVICE,
                "accessibility service is not enabled",
            )
}
