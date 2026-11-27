package app.beam.core.clipboard

import android.content.Context
import android.widget.Toast
import app.beam.core.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Sends a clip on behalf of a window that already closed (the focus window, the share
 * target), then says how it went in a toast.
 */
internal object ClipSending {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    fun sendAndTell(context: Context, read: ClipboardRead) {
        val app = context.applicationContext
        scope.launch {
            val result = ClipLinks.get(app).send(read)
            withContext(Dispatchers.Main) {
                Toast.makeText(app, message(result), Toast.LENGTH_SHORT).show()
            }
        }
    }

    fun message(result: SendResult): Int = when (result) {
        SendResult.SENT -> R.string.beam_clip_sent
        SendResult.ALREADY_THERE -> R.string.beam_clip_already_there
        SendResult.NOBODY_LINKED -> R.string.beam_clip_nobody
        SendResult.NOTHING_COPIED -> R.string.beam_clip_nothing
        SendResult.TOO_LARGE -> R.string.beam_clip_too_large
        SendResult.UNSUPPORTED -> R.string.beam_clip_unsupported
        SendResult.FAILED -> R.string.beam_clip_failed
    }
}
