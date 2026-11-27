package app.beam.core.clipboard

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import app.beam.core.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Keeps clipboard links open while devices are linked, with the screen off or Beam in the
 * background (a `connectedDevice` foreground service). Its notification names the linked
 * devices and has the buttons people use from anywhere: send the clipboard, or disconnect.
 * It stops when the last link ends.
 */
class ClipLinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var links: ClipLinks
    private lateinit var manager: NotificationManager
    private var watching: Job? = null

    override fun onCreate() {
        super.onCreate()
        links = ClipLinks.get(this)
        manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL,
                getString(R.string.beam_channel_linked),
                NotificationManager.IMPORTANCE_LOW
            ).apply { description = getString(R.string.beam_channel_linked_description) }
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!goForeground()) return START_NOT_STICKY
        if (intent?.action == ACTION_DISCONNECT) links.disconnectAll()
        if (watching == null) watching = scope.launch { watch() }
        return START_NOT_STICKY // links don't survive the process; devices link again
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun goForeground(): Boolean = try {
        val notification = notification(links.linked.value)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } else {
            startForeground(ID, notification)
        }
        true
    } catch (e: IllegalStateException) {
        // Android refused (started from the background): links work while Beam is open.
        Log.w(TAG, "could not keep clipboard links open in the background", e)
        stopSelf()
        false
    } catch (e: SecurityException) {
        Log.w(TAG, "not allowed to run a connected-device service", e)
        stopSelf()
        false
    }

    private suspend fun watch() {
        links.linked.collect { devices ->
            if (devices.isEmpty()) {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            } else {
                manager.notify(ID, notification(devices))
            }
        }
    }

    private fun notification(devices: List<LinkedDevice>): Notification {
        val names = devices.joinToString { it.name }
        val send = PendingIntent.getActivity(
            this,
            0,
            ClipFocusActivity.intent(this),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val disconnect = PendingIntent.getService(
            this,
            0,
            Intent(this, ClipLinkService::class.java).setAction(ACTION_DISCONNECT),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE)
        }
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_beam_notification)
            .setContentTitle(getString(R.string.beam_linked_title))
            .setContentText(getString(R.string.beam_linked_with, names))
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(action(R.string.beam_send_clipboard, send))
            .addAction(action(R.string.beam_disconnect, disconnect))
            .build()
    }

    private fun action(label: Int, intent: PendingIntent) =
        Notification.Action.Builder(null, getString(label), intent).build()

    companion object {
        private const val ACTION_DISCONNECT = "app.beam.core.clipboard.DISCONNECT"
        private const val CHANNEL = "beam.linked"
        private const val ID = 3 // transfers use 1 and 2
        private const val TAG = "BeamClipLinks"

        /** Asks Android to keep links open in the background; they work anyway while open. */
        internal fun start(context: Context) {
            try {
                context.startForegroundService(Intent(context, ClipLinkService::class.java))
            } catch (e: IllegalStateException) {
                Log.w(TAG, "could not start the link service; links work while Beam is open", e)
            }
        }
    }
}
