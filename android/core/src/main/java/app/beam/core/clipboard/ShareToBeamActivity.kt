package app.beam.core.clipboard

import android.app.Activity
import android.os.Bundle

/**
 * "Share → Beam": sends the text or image another app shared to the linked devices, without
 * showing a window, then says how it went.
 */
class ShareToBeamActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val shared = SharedClip.read(this, intent)
        finish()
        ClipSending.sendAndTell(this, shared)
    }
}
