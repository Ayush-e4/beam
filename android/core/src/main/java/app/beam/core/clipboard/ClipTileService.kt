package app.beam.core.clipboard

import android.app.PendingIntent
import android.app.StatusBarManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import androidx.annotation.RequiresApi
import app.beam.core.R
import java.util.concurrent.Executor
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * The "Send clipboard" Quick Settings tile: one tap from anywhere sends what is copied to the
 * linked devices, through [ClipFocusActivity]. It shows as on while a device is linked; with
 * none, tapping opens Beam to link one.
 */
class ClipTileService : TileService() {
    private var scope: CoroutineScope? = null

    override fun onStartListening() {
        super.onStartListening()
        val watching = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        scope = watching
        watching.launch { ClipLinks.get(this@ClipTileService).linked.collect(::show) }
    }

    override fun onStopListening() {
        scope?.cancel()
        scope = null
        super.onStopListening()
    }

    override fun onClick() {
        super.onClick()
        val linked = ClipLinks.get(this).linked.value.isNotEmpty()
        val intent = if (linked) {
            ClipFocusActivity.intent(this)
        } else {
            packageManager.getLaunchIntentForPackage(packageName) ?: return
        }
        if (isLocked) unlockAndRun { open(intent) } else open(intent)
    }

    private fun open(intent: Intent) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val pending = PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE)
            startActivityAndCollapse(pending)
        } else {
            @Suppress("DEPRECATION", "StartActivityAndCollapseDeprecated") // only way before 14
            startActivityAndCollapse(intent)
        }
    }

    private fun show(devices: List<LinkedDevice>) {
        val tile = qsTile ?: return
        val look = TileLook.of(devices)
        tile.state = if (look.active) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = getString(R.string.beam_send_clipboard)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            tile.subtitle = look.subtitle ?: getString(R.string.beam_tile_not_linked)
        }
        tile.icon = Icon.createWithResource(this, R.drawable.ic_beam_notification)
        tile.updateTile()
    }

    companion object {
        /**
         * Asks people to add the tile to Quick Settings (Android 13+; before, they add it by
         * editing Quick Settings). [onResult] gets a `StatusBarManager.TILE_ADD_REQUEST_*`.
         */
        @RequiresApi(Build.VERSION_CODES.TIRAMISU)
        fun requestAdd(context: Context, executor: Executor, onResult: (Int) -> Unit) {
            context.getSystemService(StatusBarManager::class.java).requestAddTileService(
                ComponentName(context, ClipTileService::class.java),
                context.getString(R.string.beam_send_clipboard),
                Icon.createWithResource(context, R.drawable.ic_beam_notification),
                executor
            ) { result -> onResult(result) }
        }
    }
}

/** How the tile looks for the linked [devices]: on with their names, or off. */
internal data class TileLook(val active: Boolean, val subtitle: String?) {
    companion object {
        fun of(devices: List<LinkedDevice>): TileLook = when (devices.size) {
            0 -> TileLook(active = false, subtitle = null)
            1 -> TileLook(active = true, subtitle = devices.single().name)
            else -> TileLook(
                active = true,
                subtitle = "${devices.first().name} +${devices.size - 1}"
            )
        }
    }
}
