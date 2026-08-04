package com.skycua.phonecompanion.camera

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.content.ContextCompat
import com.skycua.phonecompanion.R

/** Foreground lifetime marker for active camera and microphone sessions. */
class CameraCaptureService : Service() {
    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, "Sky camera", NotificationManager.IMPORTANCE_LOW),
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val audio = intent?.getBooleanExtra(EXTRA_AUDIO, false) == true
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Sky camera active")
            .setContentText(if (audio) "Recording camera and microphone" else "Using camera")
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 29) {
            var type = ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
            if (audio) type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            startForeground(NOTIFICATION_ID, notification, type)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val CHANNEL = "sky-camera"
        private const val NOTIFICATION_ID = 47684
        private const val EXTRA_AUDIO = "audio"

        fun start(context: Context, audio: Boolean) {
            ContextCompat.startForegroundService(
                context,
                Intent(context, CameraCaptureService::class.java).putExtra(EXTRA_AUDIO, audio),
            )
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, CameraCaptureService::class.java))
        }
    }
}
