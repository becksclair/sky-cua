package com.skycua.phonecompanion

import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat

/** Themed one-time SAF tree picker launched by a remote storage request. */
class SafGrantActivity : AppCompatActivity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        setContentView(
            LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(dp(24), dp(32), dp(24), dp(32))
                setBackgroundColor(color(R.color.sky_bg))
                addView(TextView(this@SafGrantActivity).apply {
                    text = getString(R.string.saf_grant_title)
                    textSize = 24f
                    typeface = Typeface.DEFAULT_BOLD
                    setTextColor(color(R.color.sky_heading))
                }, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
                addView(TextView(this@SafGrantActivity).apply {
                    text = getString(R.string.saf_grant_body)
                    textSize = 15f
                    setTextColor(color(R.color.sky_text))
                    setPadding(0, dp(12), 0, 0)
                }, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
            },
        )
        if (state == null) {
            startActivityForResult(
                Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
                ),
                REQUEST_TREE,
            )
        }
    }

    @Deprecated("Activity result API is sufficient for this one-shot system picker")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQUEST_TREE && resultCode == RESULT_OK) {
            data?.data?.let { uri ->
                val flags = data.flags and
                    (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                contentResolver.takePersistableUriPermission(uri, flags)
            }
        }
        finish()
    }

    private fun color(id: Int) = ContextCompat.getColor(this, id)
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    companion object { private const val REQUEST_TREE = 41 }
}
