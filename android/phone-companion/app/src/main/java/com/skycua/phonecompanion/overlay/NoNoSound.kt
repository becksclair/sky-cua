package com.skycua.phonecompanion.overlay

import android.content.Context
import android.media.AudioAttributes
import android.media.SoundPool
import com.skycua.phonecompanion.R

/**
 * Plays the short Opus easter-egg blip that accompanies the cursor's "no-no"
 * head-shake. A [SoundPool] preloads the (sub-second) sample once so playback is
 * immediate and never touches the disk on the main thread, and it plays on the
 * sonification stream without taking audio focus — a quick chirp over whatever
 * audio is already running rather than pausing it.
 *
 * The clip is stored uncompressed (`noCompress "opus"` in the module build) so
 * [SoundPool.load] can open it through a raw-resource file descriptor.
 */
class NoNoSound(context: Context) {
    private val soundPool =
        SoundPool.Builder()
            .setMaxStreams(1)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build(),
            )
            .build()

    @Volatile
    private var loaded: Boolean = false

    private val sampleId: Int

    init {
        soundPool.setOnLoadCompleteListener { _, id, status ->
            if (id == sampleId && status == 0) loaded = true
        }
        // Use the application context so the preloaded sample outlives any single
        // Activity/Service that created the controller.
        sampleId = soundPool.load(context.applicationContext, R.raw.uwu, 1)
    }

    /**
     * Plays the blip once at [VOLUME] of the current media-stream volume. A no-op
     * until the sample has finished loading, so an early trigger is silently
     * skipped rather than crashing — by the time the user pokes the pointer the
     * sub-second clip is long since ready.
     */
    fun play() {
        if (loaded) {
            soundPool.play(sampleId, VOLUME, VOLUME, /* priority = */ 1, /* loop = */ 0, /* rate = */ 1f)
        }
    }

    /** Frees the native sample and stream. Safe to call once at teardown. */
    fun release() {
        soundPool.release()
    }

    private companion object {
        /**
         * Playback level as a fraction of the current media-stream volume. Android
         * scales app playback by the user's media volume and cannot exceed it, so
         * this is relative (not an absolute fraction of full system volume).
         */
        const val VOLUME: Float = 0.45f
    }
}
