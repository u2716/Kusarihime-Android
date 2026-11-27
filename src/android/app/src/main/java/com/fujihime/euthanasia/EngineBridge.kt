package com.fujihime.euthanasia
import android.content.Context
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

data class PendingDialogInfo(val id: Long, val kind: Int, val title: String, val message: String)
data class EngineState(val scene: Int, val languageMode: Int, val isPlayingVideo: Boolean, val isWaitingClick: Boolean, val isWaitingSelect: Boolean, val isTransitioning: Boolean, val exitRequested: Boolean, val scriptId: Int, val pc: Long)

class EngineBridge(private val context: Context) {
    companion object {
        init { System.loadLibrary("fujihime_core") }
        const val GAME_WIDTH = 800; const val GAME_HEIGHT = 600
        const val PRESENT_LEN = GAME_WIDTH * GAME_HEIGHT * 4
        const val SAMPLE_RATE = 44100
        const val LANG_JP = 0; const val LANG_CHS = 1
    }
    private val prefs = context.getSharedPreferences("fujihime", Context.MODE_PRIVATE)
    val saveDirPath: String; val overrideDirPath: String; val gameRootPath: String
    val origBackupPath: String; val fontsDirPath: String
    var hasGameAssets: Boolean = false; private set
    var currentLanguageMode: Int = prefs.getInt("fujihime.language_mode", LANG_JP); private set
    var loadedFontSummary: String = "none"; private set
    var isDestroyed = false; private set
    private var handle: Long = 0
    val frameBuffer: ByteBuffer = ByteBuffer.allocateDirect(PRESENT_LEN).order(ByteOrder.nativeOrder())
    init {
        val filesDir = context.filesDir
        val saveDir = File(filesDir, "save"); val overrideDir = File(filesDir, "override")
        saveDir.mkdirs(); overrideDir.mkdirs()
        saveDirPath = saveDir.absolutePath; overrideDirPath = overrideDir.absolutePath
        fontsDirPath = File(filesDir, "Fonts").absolutePath
        val filesGame = File(filesDir, "game")
        val extGame = context.getExternalFilesDir(null)?.let { File(it, "game") }
        gameRootPath = when {
            File(filesGame, "scr.xfl").exists() || File(filesGame, "orig_backup/scr.xfl").exists() -> filesGame.absolutePath
            extGame != null && (File(extGame, "scr.xfl").exists()) -> extGame.absolutePath
            else -> filesGame.absolutePath
        }
        origBackupPath = File(gameRootPath, "orig_backup").absolutePath
        hasGameAssets = File(gameRootPath, "scr.xfl").exists() || File(gameRootPath, "orig_backup/scr.xfl").exists()
        if (hasGameAssets) initializeEngine()
    }
    @Synchronized fun initializeEngine(): Boolean {
        if (handle != 0L) return true
        val repair = currentLanguageMode == LANG_CHS
        handle = nativeCreate(gameRootPath, saveDirPath, overrideDirPath, origBackupPath, currentLanguageMode, repair)
        if (handle == 0L) return false
        loadFontsForLanguage(currentLanguageMode)
        import2002CdSaveIfPresentOnStartup()
        startTitle(); return true
    }
    @Synchronized fun destroy() { isDestroyed = true; if (handle != 0L) { try { nativeDestroy(handle) } catch (_: Exception) {}; handle = 0 } }
    private fun findFontBytes(candidates: List<String>): Pair<String, ByteArray>? {
        for (name in candidates) {
            val f = File(fontsDirPath, name); if (f.exists() && f.length() > 0) return name to f.readBytes()
            try { context.assets.open("Fonts/$name").use { b -> val d = b.readBytes(); if (d.isNotEmpty()) return name to d } } catch (_: Exception) {}
        }
        return null
    }
    private fun loadFontsForLanguage(mode: Int) {
        val jpGothic = findFontBytes(listOf("jp_gothic.ttf","jp_gothic.ttc","jp_gothic.otf","msgothic.ttc","ipag.ttf"))
        val jpMincho = findFontBytes(listOf("jp_mincho.ttf","jp_mincho.ttc","jp_mincho.otf","msmincho.ttc","ipam.ttf"))
        val chsHei = findFontBytes(listOf("chs_hei.ttf","chs_hei.ttc","chs_hei.otf","wqy-zenhei.ttc","simhei.ttf"))
        val parts = mutableListOf<String>()
        fun reg(pair: Pair<String, ByteArray>?, role: Int, label: String) { if (pair != null && handle != 0L && nativeLoadFont(handle, role, pair.second, 0) == 0) parts += "$label=${pair.first}" }
        if (mode == LANG_CHS) { reg(chsHei ?: jpGothic, 0, "gothic"); reg(chsHei ?: jpMincho, 1, "mincho"); reg(jpGothic, 2, "fallback") }
        else { reg(jpGothic ?: chsHei, 0, "gothic"); reg(jpMincho, 1, "mincho"); reg(chsHei, 2, "fallback") }
        loadedFontSummary = if (parts.isEmpty()) "none" else parts.joinToString(", ")
    }
    @Synchronized fun startTitle(): Boolean = handle != 0L && nativeStartTitle(handle) == 0
    @Synchronized fun tick(deltaMs: Int): Boolean = handle != 0L && nativeTick(handle, deltaMs) == 0
    @Synchronized fun copyPresent(): Boolean { if (handle == 0L) return false; frameBuffer.clear(); return nativeCopyPresent(handle, frameBuffer) == 0 }
    @Synchronized fun renderAudio(buf: ByteBuffer, frames: Int): Int { if (handle == 0L) return 0; buf.clear(); return nativeRenderAudio(handle, buf, frames) }
    @Synchronized fun pointerEvent(kind: Int, x: Int, y: Int) { if (handle != 0L) nativePointerEvent(handle, kind, x, y) }
    @Synchronized fun keyEvent(vk: Int, down: Boolean) { if (handle != 0L) nativeKeyEvent(handle, vk, down) }
    @Synchronized fun performAction(a: Int) { if (handle != 0L) nativePerformAction(handle, a) }
    @Synchronized fun saveGlobal(): Boolean = handle != 0L && nativeSaveGlobal(handle) == 0
    @Synchronized fun setLanguageMode(mode: Int): Boolean {
        if (handle == 0L) return false
        val ok = nativeSetLanguageMode(handle, mode, origBackupPath, mode == LANG_CHS) == 0
        if (ok) { currentLanguageMode = mode; prefs.edit().putInt("fujihime.language_mode", mode).apply(); loadFontsForLanguage(mode) }
        return ok
    }
    @Synchronized fun getState(): EngineState? { if (handle == 0L) return null; val s = nativeGetState(handle) ?: return null; val p = s.split("|"); if (p.size < 9) return null; return EngineState(p[0].toInt(), p[1].toInt(), p[2] == "1", p[3] == "1", p[4] == "1", p[5] == "1", p[6] == "1", p[7].toInt(), p[8].toLong()) }
    @Synchronized fun pollPendingDialog(): PendingDialogInfo? { if (handle == 0L) return null; val s = nativePollDialog(handle) ?: return null; val p = s.split("\u001f"); if (p.size < 4) return null; return PendingDialogInfo(p[0].toLong(), p[1].toInt(), p[2], p[3]) }
    @Synchronized fun confirmDialog(id: Long, accepted: Boolean) { if (handle != 0L) nativeConfirmDialog(handle, id, accepted) }
    @Synchronized fun import2002CdSaveIfPresentOnStartup(): Boolean { val f = File(saveDirPath, "KhDLsave_2002CD.dat"); if (!f.exists()) return false; val d = f.readBytes(); if (d.size != 206024) return false; val ok = import2002CdSave(d); if (ok) { File(saveDirPath, "KhDLsave_2002CD.dat.imported").delete(); f.renameTo(File(saveDirPath, "KhDLsave_2002CD.dat.imported")) }; return ok }
    @Synchronized fun import2002CdSave(data: ByteArray): Boolean = handle != 0L && data.size == 206024 && nativeImport2002CdSave(handle, data) == 0
    // JNI (must match android_jni.rs names)
    private external fun nativeCreate(gameRoot: String, saveDir: String, overrideDir: String, origBackup: String, lang: Int, repair: Boolean): Long
    private external fun nativeDestroy(handle: Long)
    private external fun nativeLoadFont(handle: Long, role: Int, data: ByteArray, faceIndex: Int): Int
    private external fun nativeStartTitle(handle: Long): Int
    private external fun nativeStartGame(handle: Long, scriptId: Int): Int
    private external fun nativeTick(handle: Long, deltaMs: Int): Int
    private external fun nativePointerEvent(handle: Long, kind: Int, x: Int, y: Int): Int
    private external fun nativeKeyEvent(handle: Long, vk: Int, down: Boolean): Int
    private external fun nativePerformAction(handle: Long, action: Int): Int
    private external fun nativeSaveGlobal(handle: Long): Int
    private external fun nativeSaveSlot(handle: Long, slot: Int, y: Int, mo: Int, d: Int, h: Int, mi: Int): Int
    private external fun nativeLoadSlot(handle: Long, slot: Int): Int
    private external fun nativeRollback(handle: Long): Int
    private external fun nativeSetLanguageMode(handle: Long, mode: Int, origBackup: String, repair: Boolean): Int
    private external fun nativeConfirmDialog(handle: Long, dialogId: Long, accepted: Boolean): Int
    private external fun nativeImport2002CdSave(handle: Long, data: ByteArray): Int
    private external fun nativePollDialog(handle: Long): String?
    private external fun nativeGetState(handle: Long): String?
    private external fun nativeCopyPresent(handle: Long, buf: ByteBuffer): Int
    private external fun nativeRenderAudio(handle: Long, buf: ByteBuffer, frames: Int): Int
}
