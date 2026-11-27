package app.beam.core.clipboard

import android.app.Activity
import android.content.Context
import android.content.Intent

/**
 * Sends the clipboard to the linked devices. Android 10+ only lets an app read the clipboard
 * while one of its windows has focus, so this opens an invisible window for a moment, reads
 * once it has focus, and closes. The notification button and the Quick Settings tile open it.
 */
class ClipFocusActivity : Activity() {
    private var read = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || read) return
        read = true
        val found = PhoneClipboard(this).read()
        finish()
        ClipSending.sendAndTell(this, found)
    }

    companion object {
        /** Opens it, from outside an activity too. */
        fun intent(context: Context): Intent = Intent(context, ClipFocusActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)
    }
}
