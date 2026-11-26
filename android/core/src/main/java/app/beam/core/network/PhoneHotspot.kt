package app.beam.core.network

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.job
import kotlinx.coroutines.launch

/** Where the phone's hotspot is, for the button that opens it. */
sealed interface HotspotState {
    /** Closed; [why] is null until it has been opened once. */
    data class Off(val why: HotspotOff?) : HotspotState

    /** Asked Android to open it. */
    data object Opening : HotspotState

    /** Open: what the other device needs to join it. */
    data class On(val details: HotspotDetails) : HotspotState

    /** Android wouldn't open it. */
    data class Failed(val problem: HotspotProblem) : HotspotState
}

/** Why an open hotspot closed. */
enum class HotspotOff {
    /** The person closed it. */
    CLOSED,

    /** No transfer ran for a while, so Beam closed it. */
    IDLE,

    /** Android closed it (another app's hotspot, say, or Wi-Fi settings). */
    STOPPED_BY_ANDROID
}

/**
 * The phone's hotspot behind its button: people open it when the devices share no Wi-Fi. It
 * stays open while transfers run and closes once none has run for [idleMillis], so it never
 * lingers.
 */
class PhoneHotspot(
    private val opener: HotspotOpener,
    private val transfersRunning: Flow<Boolean>,
    private val scope: CoroutineScope,
    private val idleMillis: Long = IDLE_MILLIS
) {
    private val current = MutableStateFlow<HotspotState>(HotspotState.Off(null))
    private var job: Job? = null

    val state: StateFlow<HotspotState> = current.asStateFlow()

    /** Opens the hotspot; does nothing while it is open or opening. */
    @Synchronized
    fun open() {
        if (job?.isActive == true) return
        current.value = HotspotState.Opening
        job = scope.launch { keepOpen() }
    }

    /** Closes the hotspot. */
    @Synchronized
    fun close() {
        job?.cancel()
        job = null
        val now = current.value
        if (now is HotspotState.Opening || now is HotspotState.On) {
            current.value = HotspotState.Off(HotspotOff.CLOSED)
        }
    }

    private suspend fun keepOpen() {
        val me = currentCoroutineContext().job
        try {
            opener.open().use { hotspot ->
                publish(me, HotspotState.On(hotspot.details))
                publish(me, HotspotState.Off(untilClosed(hotspot)))
            }
        } catch (refused: HotspotException) {
            publish(me, HotspotState.Failed(refused.problem))
        }
    }

    /** Only the run [open] started last speaks for the hotspot; a closed one stays quiet. */
    @Synchronized
    private fun publish(run: Job, state: HotspotState) {
        if (run === job && run.isActive) current.value = state
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    private suspend fun untilClosed(hotspot: OpenHotspot): HotspotOff = merge(
        hotspot.isOn.filter { on -> !on }.map { HotspotOff.STOPPED_BY_ANDROID },
        transfersRunning.distinctUntilChanged().transformLatest { running ->
            if (!running) {
                delay(idleMillis)
                emit(HotspotOff.IDLE)
            }
        }
    ).first()

    companion object {
        /** How long an open hotspot waits for a transfer before closing. */
        const val IDLE_MILLIS = 10 * 60 * 1000L
    }
}
