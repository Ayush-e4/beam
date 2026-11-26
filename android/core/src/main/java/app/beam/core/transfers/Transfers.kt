package app.beam.core.transfers

import android.content.Context
import app.beam.core.history.TransferHistory
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/**
 * The app's transfers: one [registry] and [runner] per process, outliving every screen.
 * Starting a transfer through [runner] keeps it running in the background until it ends;
 * [history] keeps what was sent and received.
 */
class Transfers private constructor(context: Context) {
    val registry = TransferRegistry()
    val history = TransferHistory.forApp(context)
    val runner = TransferRunner(
        CoroutineScope(SupervisorJob() + Dispatchers.Default),
        registry,
        onStarted = { TransferService.start(context) }
    )

    companion object {
        @Volatile
        private var instance: Transfers? = null

        /** This process's transfers. */
        fun get(context: Context): Transfers = instance ?: synchronized(this) {
            instance ?: Transfers(context.applicationContext).also { instance = it }
        }
    }
}
