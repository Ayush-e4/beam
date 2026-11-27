package app.beam.core.clipboard

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.widget.Toast
import app.beam.core.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
        val app = applicationContext
        sending.launch {
            val result = ClipLinks.get(app).send(found)
            withContext(Dispatchers.Main) {
                Toast.makeText(app, message(result), Toast.LENGTH_SHORT).show()
            }
        }
    }

    companion object {
        /** Outlives the activity, which closes before sending finishes. */
        private val sending = CoroutineScope(SupervisorJob() + Dispatchers.Default)

        /** Opens it, from outside an activity too. */
        fun intent(context: Context): Intent = Intent(context, ClipFocusActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)

        internal fun message(result: SendResult): Int = when (result) {
            SendResult.SENT -> R.string.beam_clip_sent
            SendResult.ALREADY_THERE -> R.string.beam_clip_already_there
            SendResult.NOBODY_LINKED -> R.string.beam_clip_nobody
            SendResult.NOTHING_COPIED -> R.string.beam_clip_nothing
            SendResult.TOO_LARGE -> R.string.beam_clip_too_large
            SendResult.UNSUPPORTED -> R.string.beam_clip_unsupported
            SendResult.FAILED -> R.string.beam_clip_failed
        }
    }
}
