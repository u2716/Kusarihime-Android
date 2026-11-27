package com.fujihime.euthanasia
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.concurrent.thread

class AudioOutput(private val bridge: EngineBridge) {
    @Volatile private var running = false; private var track: AudioTrack? = null; private var worker: Thread? = null
    fun start() {
        if (running) return
        val minBuf = AudioTrack.getMinBufferSize(EngineBridge.SAMPLE_RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_FLOAT)
        val bufSize = (minBuf * 4).coerceAtLeast(44100 * 2 * 4)
        val track = AudioTrack.Builder().setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_GAME).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()).setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_FLOAT).setSampleRate(EngineBridge.SAMPLE_RATE).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).build()).setBufferSizeInBytes(bufSize).setTransferMode(AudioTrack.MODE_STREAM).build()
        this.track = track; running = true; track.play()
        worker = thread(name = "fujihime-audio", isDaemon = true) {
            val buf = ByteBuffer.allocateDirect(4096 * 2 * 4).order(ByteOrder.nativeOrder())
            while (running) { val frames = bridge.renderAudio(buf, 4096); if (frames > 0) { buf.position(0); buf.limit(frames * 2 * 4); try { track.write(buf, frames * 2 * 4, AudioTrack.WRITE_BLOCKING) } catch (_: Exception) {} } else { try { Thread.sleep(2) } catch (_: InterruptedException) { break } } }
        }
    }
    fun pause() { try { track?.pause() } catch (_: Exception) {} }
    fun resume() { try { track?.play() } catch (_: Exception) {} }
    fun stop() { running = false; try { worker?.join(500) } catch (_: Exception) {}; try { track?.stop(); track?.release() } catch (_: Exception) {}; track = null; worker = null }
}
