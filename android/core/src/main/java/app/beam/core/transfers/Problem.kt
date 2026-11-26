package app.beam.core.transfers

import app.beam.core.BeamException
import app.beam.core.NetworkFailure

/** Why a device couldn't be reached, so the screens can say what to do about it. */
enum class Problem {
    /** A VPN on this phone captures traffic to nearby devices: allow local access, or pause it. */
    VPN_BLOCKS,

    /**
     * The device is on this phone's network but never answers: the network most likely stops
     * devices from reaching each other (common on guest, hotel and office Wi-Fi).
     */
    NETWORK_BLOCKS_DEVICES,

    /** The device answered, but Beam isn't open on it to receive. */
    NOT_RECEIVING,

    /** Nothing answered and it isn't on this phone's network: it left, or is switched off. */
    OUT_OF_REACH
}

/** One failed try at one of a device's addresses. */
internal data class Attempt(val failure: NetworkFailure, val onLocalNetwork: Boolean)

/**
 * The one [Problem] behind a device's failed [attempts], or null when the link dropped
 * partway rather than never connecting (a plain network error, worth trying again).
 */
internal fun problemOf(attempts: List<Attempt>, behindVpn: Boolean): Problem? = when {
    attempts.isEmpty() -> Problem.OUT_OF_REACH
    attempts.any { it.failure == NetworkFailure.REFUSED } -> Problem.NOT_RECEIVING
    attempts.any { it.failure != NetworkFailure.NO_ANSWER } -> null
    behindVpn -> Problem.VPN_BLOCKS
    attempts.any { it.onLocalNetwork } -> Problem.NETWORK_BLOCKS_DEVICES
    else -> Problem.OUT_OF_REACH
}

/**
 * A send that couldn't reach the device, and why. Like a network error it is kept and
 * resumed when the device is reachable again.
 */
class DeviceUnreachable(val problem: Problem, cause: BeamException.Network) :
    Exception(cause.reason(), cause)
