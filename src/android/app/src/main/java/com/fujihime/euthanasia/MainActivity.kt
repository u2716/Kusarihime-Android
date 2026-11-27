package com.fujihime.euthanasia
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.KeyEvent
import android.view.WindowManager
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.documentfile.provider.DocumentFile

class MainActivity : AppCompatActivity() {
    private lateinit var bridge: EngineBridge
    private var gameView: GameView? = null; private var audio: AudioOutput? = null
    private var dialogShowing = false
    private val gameFolderPicker = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri -> if (uri != null) importGameFolder(uri) }
    private val saveFilePicker = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) importSaveFile(uri) }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN or WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        bridge = EngineBridge(this)
        val view = GameView(this, bridge, onQuickMenu = { showQuickMenu() }, onDialog = { showEngineDialog(it) }, onState = { st -> window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) })
        gameView = view; setContentView(view)
        if (bridge.hasGameAssets) { audio = AudioOutput(bridge); audio?.start() } else showMissingAssets()
    }
    private fun showEngineDialog(dlg: PendingDialogInfo) {
        if (dialogShowing) return; dialogShowing = true
        val b = AlertDialog.Builder(this).setTitle(dlg.title).setMessage(dlg.message).setCancelable(false)
        if (dlg.kind == 0) b.setPositiveButton("OK") { _, _ -> dialogShowing = false; bridge.confirmDialog(dlg.id, true) }
        else { val chs = bridge.currentLanguageMode == EngineBridge.LANG_CHS; b.setNegativeButton(if (chs) "否" else "いいえ") { _, _ -> dialogShowing = false; bridge.confirmDialog(dlg.id, false) }; b.setPositiveButton(if (chs) "是" else "はい") { _, _ -> dialogShowing = false; bridge.confirmDialog(dlg.id, true) } }
        b.show()
    }
    private fun showQuickMenu() {
        if (!bridge.hasGameAssets) { showMissingAssets(); return }
        val chs = bridge.currentLanguageMode == EngineBridge.LANG_CHS
        val items = mutableListOf<String>(); val actions = mutableListOf<Int>()
        fun add(label: String, act: Int) { items += label; actions += act }
        add(if (chs) "切换至原版日文" else "簡体字中国語 V1.0 へ切替", -1)
        add(if (chs) "存档 (Save)" else "セーブ (Save)", 0); add(if (chs) "读档 (Load)" else "ロード (Load)", 1)
        add(if (chs) "系统设置 (Config)" else "環境設定 (Config)", 2); add(if (chs) "自动播放 (Auto)" else "オート (Auto)", 5)
        add(if (chs) "快进模式 (Skip)" else "スキップ (Skip)", 4); add(if (chs) "回看历史 (Backlog)" else "バックログ (Backlog)", 3)
        add(if (chs) "一步回退 (Rollback)" else "一つ戻る (Rollback)", 7); add(if (chs) "隐藏 / 显示文本框" else "ウィンドウ消去 / 表示", 6)
        add(if (chs) "回到标题画面" else "タイトルに戻る", 8); add(if (chs) "导入 2002 CD 存档" else "2002 CD セーブ取込", -2)
        add(if (chs) "操作说明" else "操作ガイド", -3)
        AlertDialog.Builder(this).setTitle(if (chs) "腐姫 快捷菜单" else "腐姫 クイックメニュー").setMessage(if (chs) "当前模式：简体中文 V1.0\n字体：${bridge.loadedFontSummary}" else "現在モード：オリジナル日本語\nフォント：${bridge.loadedFontSummary}").setItems(items.toTypedArray()) { _, which ->
            when (actions[which]) {
                -1 -> bridge.setLanguageMode(if (chs) EngineBridge.LANG_JP else EngineBridge.LANG_CHS)
                -2 -> saveFilePicker.launch(arrayOf("*/*"))
                -3 -> showHelp()
                else -> bridge.performAction(actions[which])
            }
        }.setNegativeButton(if (chs) "取消" else "キャンセル", null).show()
    }
    private fun showHelp() { AlertDialog.Builder(this).setTitle(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "操作说明" else "操作ガイド").setMessage("单指点击：左键推进 / 确定\n单指拖动：悬停高亮 / 滑块\n单指长按 (≥0.45s)：按住快进 (Ctrl)\n双指轻点：右键\n双指上下滑动：Backlog\n三指轻点：快捷菜单\n硬件键盘：Enter / Space / Esc / Ctrl / Shift / Tab / 方向键 / PageUp / PageDown / F1").setPositiveButton("OK", null).show() }
    private fun showMissingAssets() {
        AlertDialog.Builder(this).setTitle("Missing game assets").setMessage("Put the original game in filesDir/game (adb push game/ to /sdcard/Android/data/com.fujihime.euthanasia/files/game/) or pick a folder below. Also put fonts in filesDir/Fonts or assets/Fonts.").setCancelable(false)
            .setPositiveButton("Pick game folder") { _, _ -> gameFolderPicker.launch(null) }
            .setNegativeButton("Copy bundled assets") { _, _ -> copyBundledAssets() }
            .show()
    }
    private fun copyBundledAssets() {
        Thread { try { val list = assets.list("game") ?: emptyArray(); if (list.isEmpty()) { runOnUiThread { AlertDialog.Builder(this).setTitle("No bundled assets").setMessage("No assets/game in APK. Use Pick game folder.").setPositiveButton("OK", null).show() }; return@Thread }; copyAssetDir("game", java.io.File(filesDir, "game")); runOnUiThread { recreate() } } catch (e: Exception) { runOnUiThread { AlertDialog.Builder(this).setTitle("Copy failed").setMessage(e.toString()).setPositiveButton("OK", null).show() } } }.start()
    }
    private fun copyAssetDir(assetPath: String, dest: java.io.File) { dest.mkdirs(); for (n in assets.list(assetPath) ?: emptyArray()) { val p = "$assetPath/$n"; try { assets.open(p).use { ins -> java.io.File(dest, n).outputStream().use { ins.copyTo(it) } } } catch (_: Exception) { copyAssetDir(p, java.io.File(dest, n)) } } }
    private fun importGameFolder(uri: Uri) {
        Thread { try { val root = DocumentFile.fromTreeUri(this, uri) ?: return@Thread; val dest = java.io.File(filesDir, "game"); copyDocTree(root, dest); runOnUiThread { recreate() } } catch (e: Exception) { runOnUiThread { AlertDialog.Builder(this).setTitle("Import failed").setMessage(e.toString()).setPositiveButton("OK", null).show() } } }.start()
    }
    private fun copyDocTree(doc: DocumentFile, dest: java.io.File) { if (doc.isDirectory) { dest.mkdirs(); for (c in doc.listFiles()) copyDocTree(c, java.io.File(dest, c.name ?: "file")) } else { dest.parentFile?.mkdirs(); contentResolver.openInputStream(doc.uri)?.use { ins -> dest.outputStream().use { ins.copyTo(it) } } } }
    private fun importSaveFile(uri: Uri) {
        Thread { try { val bytes = contentResolver.openInputStream(uri)?.readBytes() ?: return@Thread; if (bytes.size == 206024) { val ok = bridge.import2002CdSave(bytes); runOnUiThread { AlertDialog.Builder(this).setTitle(if (ok) "导入成功" else "导入失败").setMessage(if (ok) "2002 CD save imported." else "Invalid save.").setPositiveButton("OK", null).show() } } } catch (e: Exception) {} }.start()
    }
    private fun mapKey(code: Int): Int? = when (code) { KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> 0x0D; KeyEvent.KEYCODE_SPACE -> 0x20; KeyEvent.KEYCODE_ESCAPE -> 0x1B; KeyEvent.KEYCODE_TAB -> 0x09; KeyEvent.KEYCODE_CTRL_LEFT, KeyEvent.KEYCODE_CTRL_RIGHT -> 0x11; KeyEvent.KEYCODE_SHIFT_LEFT, KeyEvent.KEYCODE_SHIFT_RIGHT -> 0x10; KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_PAGE_UP -> 0x26; KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_PAGE_DOWN -> 0x28; KeyEvent.KEYCODE_F1 -> 0x70; else -> null }
    override fun onKeyDown(k: Int, e: KeyEvent): Boolean { val vk = mapKey(k); if (vk != null && e.repeatCount == 0) { bridge.keyEvent(vk, true); return true }; return super.onKeyDown(k, e) }
    override fun onKeyUp(k: Int, e: KeyEvent): Boolean { val vk = mapKey(k); if (vk != null) { bridge.keyEvent(vk, false); return true }; return super.onKeyUp(k, e) }
    override fun onBackPressed() { bridge.keyEvent(0x1B, true); bridge.keyEvent(0x1B, false) }
    override fun onPause() { super.onPause(); gameView?.onPause(); audio?.pause(); if (bridge.hasGameAssets) bridge.saveGlobal() }
    override fun onResume() { super.onResume(); gameView?.onResume(); if (bridge.hasGameAssets) audio?.resume() }
    override fun onDestroy() { super.onDestroy(); audio?.stop(); if (bridge.hasGameAssets) bridge.saveGlobal(); bridge.destroy() }
}
