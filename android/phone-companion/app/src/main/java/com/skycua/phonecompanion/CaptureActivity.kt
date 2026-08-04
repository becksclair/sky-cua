package com.skycua.phonecompanion

import android.graphics.Color
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.view.PreviewView
import com.skycua.phonecompanion.camera.CameraRuntime

/** Visible, Companion-themed surface required for remote camera activation. */
class CaptureActivity : AppCompatActivity() {
    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val preview = PreviewView(this).apply { scaleType = PreviewView.ScaleType.FILL_CENTER }
        status = TextView(this).apply {
            text = "Preparing camera…"
            setTextColor(Color.WHITE)
            setBackgroundColor(0x99000000.toInt())
            textSize = 16f
            gravity = Gravity.CENTER
            setPadding(24, 16, 24, 16)
        }
        setContentView(FrameLayout(this).apply {
            setBackgroundColor(Color.BLACK)
            addView(preview, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            addView(status, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM))
        })
        val requestId = intent.getStringExtra(EXTRA_REQUEST_ID) ?: run { finish(); return }
        CameraRuntime.attach(this, preview, requestId)
    }

    fun showActive(message: String) { status.text = message }

    override fun onDestroy() {
        CameraRuntime.activityDestroyed(this)
        super.onDestroy()
    }

    companion object { const val EXTRA_REQUEST_ID = "request_id" }
}
