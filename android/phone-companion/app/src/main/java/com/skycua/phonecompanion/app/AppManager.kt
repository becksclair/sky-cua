package com.skycua.phonecompanion.app

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import com.skycua.phonecompanion.protocol.AppEntry
import com.skycua.phonecompanion.protocol.AppListParams
import com.skycua.phonecompanion.protocol.AppListResult
import com.skycua.phonecompanion.protocol.AppOp
import com.skycua.phonecompanion.protocol.AppOpParams
import com.skycua.phonecompanion.protocol.MethodApplicationException
import com.skycua.phonecompanion.protocol.Protocol

/**
 * PackageManager-based app management. Launch uses
 * `getLaunchIntentForPackage`; full inventory may be incomplete because Android
 * package visibility can hide apps, so the host acknowledges a `pm list`
 * fallback. Force-stop is best-effort: a normal app cannot force-stop arbitrary
 * packages, so it surfaces a structured error rather than faking success.
 */
class AppManager(private val context: Context) {
    private val pm: PackageManager
        get() = context.packageManager

    fun listApps(params: AppListParams): AppListResult {
        val installed = pm.getInstalledApplications(0)
        val apps =
            installed
                .map { info ->
                    val launchable = pm.getLaunchIntentForPackage(info.packageName) != null
                    AppEntry(
                        packageName = info.packageName,
                        label = pm.getApplicationLabel(info).toString(),
                        launchable = launchable,
                    )
                }.filter { if (params.launchableOnly) it.launchable else true }
                .sortedBy { it.packageName }
        return AppListResult(apps = apps, truncated = false)
    }

    fun perform(params: AppOpParams) {
        when (params.op) {
            AppOp.LAUNCH -> launch(params.packageName!!)
            AppOp.OPEN_INTENT -> openIntent(params.intentUri!!)
            AppOp.FORCE_STOP -> forceStop(params.packageName!!)
        }
    }

    private fun launch(packageName: String) {
        val intent =
            pm.getLaunchIntentForPackage(packageName)
                ?: throw MethodApplicationException(
                    Protocol.ErrorCodes.GONE,
                    "no launchable activity for $packageName",
                )
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }

    private fun openIntent(intentUri: String) {
        val intent =
            try {
                if (intentUri.startsWith("intent:")) {
                    Intent.parseUri(intentUri, Intent.URI_INTENT_SCHEME)
                } else {
                    Intent(Intent.ACTION_VIEW, Uri.parse(intentUri))
                }
            } catch (_: Exception) {
                throw MethodApplicationException(
                    Protocol.ErrorCodes.BAD_REQUEST,
                    "could not parse intent uri",
                )
            }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (_: android.content.ActivityNotFoundException) {
            throw MethodApplicationException(
                Protocol.ErrorCodes.GONE,
                "no activity resolved the intent uri",
            )
        }
    }

    private fun forceStop(packageName: String) {
        // A non-privileged app cannot force-stop other packages; only the host's
        // ADB fallback can. Report this honestly rather than pretending success.
        throw MethodApplicationException(
            Protocol.ErrorCodes.OEM_FILTERED,
            "force-stop requires the ADB fallback; companion cannot force-stop $packageName",
        )
    }
}
