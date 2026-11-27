package com.fujihime.euthanasia
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.KeyEvent
import android.view.WindowManager
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.documentfile.provider.DocumentFile

class MainActivity : AppCompatActivity() {
    private lateinit var bridge: EngineBridge
    private var gameView: GameView? = null; private var audio: AudioOutput? = null
    private var dialogShowing = false
    private var quickMenuShowing = false
    private var isDestroying = false
    private var progressDialog: AlertDialog? = null
    private var progressBar: ProgressBar? = null
    private var progressText: TextView? = null
    private var copyStateFiles = 0
    private var copyStateBytes = 0L
    private val gameFolderPicker = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri -> if (uri != null) importGameFolder(uri) }
    private val saveFilePicker = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) importSaveFile(uri) }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN or WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        bridge = EngineBridge(this)
        val view = GameView(this, bridge, onQuickMenu = { showQuickMenu() }, onDialog = { showEngineDialog(it) }, onState = { st -> window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) })
        gameView = view; setContentView(view)
        if (bridge.hasGameAssets) { audio = AudioOutput(bridge); audio?.start() } else if (hasBundledGameAssets()) { copyBundledAssetsWithProgress() } else showMissingAssets()
    }
    private fun showEngineDialog(dlg: PendingDialogInfo) {
        if (dialogShowing) return; dialogShowing = true
        val b = AlertDialog.Builder(this).setTitle(dlg.title).setMessage(dlg.message).setCancelable(false)
        if (dlg.kind == 0) b.setPositiveButton("OK") { _, _ -> dialogShowing = false; bridge.confirmDialog(dlg.id, true) }
        else { val chs = bridge.currentLanguageMode == EngineBridge.LANG_CHS; b.setNegativeButton(if (chs) "否" else "いいえ") { _, _ -> dialogShowing = false; bridge.confirmDialog(dlg.id, false) }; b.setPositiveButton(if (chs) "是" else "はい") { _, _ -> dialogShowing = false; bridge.confirmDialog(dlg.id, true) } }
        b.show()
    }
    private fun dismissQuickMenu() { quickMenuShowing = false; gameView?.resetTouchState(); gameView?.setMenuOpen(false) }
    private fun showQuickMenu() {
        if (!bridge.hasGameAssets) { showMissingAssets(); return }
        if (quickMenuShowing) return
        quickMenuShowing = true
        gameView?.setMenuOpen(true)
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
        val header = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(48, 32, 48, 8); addView(TextView(this@MainActivity).apply { text = if (chs) "腐姫 快捷菜单" else "腐姫 クイックメニュー"; textSize = 20f }); addView(TextView(this@MainActivity).apply { text = if (chs) "当前模式：简体中文 V1.0\n字体：${bridge.loadedFontSummary}" else "現在モード：オリジナル日本語\nフォント：${bridge.loadedFontSummary}"; textSize = 14f }) }
        AlertDialog.Builder(this).setCustomTitle(header).setItems(items.toTypedArray()) { _, which -> dismissQuickMenu()
            when (actions[which]) {
                -1 -> Thread { try { bridge.setLanguageMode(if (chs) EngineBridge.LANG_JP else EngineBridge.LANG_CHS) } catch (_: Exception) {} }.start()
                -2 -> saveFilePicker.launch(arrayOf("*/*"))
                -3 -> showHelp()
                else -> { if (actions[which] == 0 || actions[which] == 1) bridge.syncTime(); bridge.queueAction(actions[which]) }
            }
        }.setNegativeButton(if (chs) "取消" else "キャンセル") { _, _ -> dismissQuickMenu() }.setOnCancelListener { dismissQuickMenu() }.setOnDismissListener { dismissQuickMenu() }.show()
    }
    private fun showHelp() { AlertDialog.Builder(this).setTitle(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "操作说明" else "操作ガイド").setMessage("单指点击：左键推进 / 确定\n单指拖动：悬停高亮 / 滑块\n单指长按 (≥0.45s)：按住快进 (Ctrl)\n双指轻点：右键\n双指上下滑动：Backlog\n三指轻点：快捷菜单\n硬件键盘：Enter / Space / Esc / Ctrl / Shift / Tab / 方向键 / PageUp / PageDown / F1").setPositiveButton("OK", null).show() }
    private fun showMissingAssets() {
        // Single action: pick game folder. Bundle button removed - APK has no bundled game (895 MB),
        // and if it ever does, onCreate auto-copies it with progress, no button needed.
        AlertDialog.Builder(this).setTitle("Game files needed").setMessage("Pick the game folder (the folder with scr.xfl, bgm.xfl...) ~895 MB to copy to internal app's data.").setCancelable(false)
            .setPositiveButton("Pick game folder") { _, _ -> gameFolderPicker.launch(null) }
            .show()
    }
    private fun hasBundledGameAssets(): Boolean { return try { val l = assets.list("game"); l != null && l.isNotEmpty() } catch (_: Exception) { false } }
    private fun showProgressDialog(title: String) {
        val layout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(48, 32, 48, 16) }
        progressBar = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply { max = 100; progress = 0; layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT) }
        progressText = TextView(this).apply { text = "Starting..."; textSize = 14f }
        layout.addView(progressBar); layout.addView(progressText)
        progressDialog = AlertDialog.Builder(this).setTitle(title).setView(layout).setCancelable(false).create()
        try { progressDialog?.show() } catch (_: Exception) {}
    }
    private fun updateProgress(copiedFiles: Int, totalFiles: Int, copiedBytes: Long, totalBytes: Long, currentName: String) {
        runOnUiThread {
            val pct = if (totalBytes > 0) ((copiedBytes * 100 / totalBytes).toInt()).coerceIn(0, 100) else if (totalFiles > 0) ((copiedFiles * 100 / totalFiles).toInt()).coerceIn(0, 100) else 0
            progressBar?.progress = pct
            val mb = if (totalBytes > 0) "${copiedBytes / 1024 / 1024} / ${totalBytes / 1024 / 1024} MB" else "$copiedFiles / $totalFiles files"
            progressText?.text = "$currentName\n$mb ($pct%)"
        }
    }
    private fun dismissProgress() { runOnUiThread { try { progressDialog?.dismiss() } catch (_: Exception) {}; progressDialog = null; progressBar = null; progressText = null } }
    private fun copyBundledAssetsWithProgress() {
        Thread { try { showProgressDialog("Copying bundled game files"); copyStateFiles = 0; copyStateBytes = 0L; val total = countAssetTree("game"); copyAssetDirWithProgress("game", java.io.File(filesDir, "game"), total); try { copyAssetDir("Fonts", java.io.File(filesDir, "Fonts")) } catch (_: Exception) {}; dismissProgress(); runOnUiThread { recreate() } } catch (e: Exception) { dismissProgress(); runOnUiThread { AlertDialog.Builder(this).setTitle("Copy failed").setMessage(e.toString()).setPositiveButton("OK", null).show() } } }.start()
    }
    private fun countAssetTree(assetPath: String): Int { val list = try { assets.list(assetPath) ?: emptyArray() } catch (_: Exception) { emptyArray() }; if (list.isEmpty()) return 1; var n = 0; for (nm in list) { val p = if (assetPath.isEmpty()) nm else "$assetPath/$nm"; n += countAssetTree(p) }; return n }
    private fun copyAssetDirWithProgress(assetPath: String, dest: java.io.File, totalFiles: Int) { dest.mkdirs(); val list = try { assets.list(assetPath) ?: emptyArray() } catch (_: Exception) { emptyArray() }; if (list.isEmpty()) return; for (n in list) { val p = "$assetPath/$n"; var isFile = true; try { assets.open(p).use { } } catch (_: Exception) { isFile = false }; if (isFile) { try { assets.open(p).use { ins -> java.io.File(dest, n).outputStream().use { out -> val buf = ByteArray(256 * 1024); var r: Int; while (ins.read(buf).also { r = it } != -1) { out.write(buf, 0, r); copyStateBytes += r } } } } catch (_: Exception) {}; copyStateFiles++; updateProgress(copyStateFiles, totalFiles, copyStateBytes, 0L, n) } else { copyAssetDirWithProgress(p, java.io.File(dest, n), totalFiles) } } }
    private fun copyBundledAssets() { copyBundledAssetsWithProgress() }
    private fun copyAssetDir(assetPath: String, dest: java.io.File) { dest.mkdirs(); for (n in assets.list(assetPath) ?: emptyArray()) { val p = "$assetPath/$n"; try { assets.open(p).use { ins -> java.io.File(dest, n).outputStream().use { ins.copyTo(it) } } } catch (_: Exception) { copyAssetDir(p, java.io.File(dest, n)) } } }
    private fun importGameFolder(uri: Uri) {
        Thread { try { val root = DocumentFile.fromTreeUri(this, uri) ?: return@Thread; runOnUiThread { showProgressDialog("Copying game folder") }; copyStateFiles = 0; copyStateBytes = 0L; val (totalFiles, totalBytes) = countDocTree(root); val dest = java.io.File(filesDir, "game"); copyDocTreeWithProgress(root, dest, totalFiles, totalBytes); dismissProgress(); runOnUiThread { recreate() } } catch (e: Exception) { dismissProgress(); runOnUiThread { AlertDialog.Builder(this).setTitle("Import failed").setMessage(e.toString()).setPositiveButton("OK", null).show() } } }.start()
    }
    private fun countDocTree(doc: DocumentFile): Pair<Int, Long> { if (!doc.isDirectory) return 1 to (if (doc.length() > 0) doc.length() else 0L); var f = 0; var b = 0L; for (c in doc.listFiles()) { val (cf, cb) = countDocTree(c); f += cf; b += cb }; return f to b }
    private fun copyDocTreeWithProgress(doc: DocumentFile, dest: java.io.File, totalFiles: Int, totalBytes: Long) { if (doc.isDirectory) { dest.mkdirs(); for (c in doc.listFiles()) copyDocTreeWithProgress(c, java.io.File(dest, c.name ?: "file"), totalFiles, totalBytes) } else { dest.parentFile?.mkdirs(); contentResolver.openInputStream(doc.uri)?.use { ins -> dest.outputStream().use { out -> val buf = ByteArray(256 * 1024); var n: Int; while (ins.read(buf).also { n = it } != -1) { out.write(buf, 0, n); copyStateBytes += n; if (copyStateBytes % (1024 * 1024) < 256 * 1024) updateProgress(copyStateFiles, totalFiles, copyStateBytes, totalBytes, doc.name ?: "file") } } }; copyStateFiles++; updateProgress(copyStateFiles, totalFiles, copyStateBytes, totalBytes, doc.name ?: "file") } }
    private fun copyDocTree(doc: DocumentFile, dest: java.io.File) { if (doc.isDirectory) { dest.mkdirs(); for (c in doc.listFiles()) copyDocTree(c, java.io.File(dest, c.name ?: "file")) } else { dest.parentFile?.mkdirs(); contentResolver.openInputStream(doc.uri)?.use { ins -> dest.outputStream().use { ins.copyTo(it) } } } }
    private fun importSaveFile(uri: Uri) {
        Thread { try { val bytes = contentResolver.openInputStream(uri)?.readBytes() ?: return@Thread; if (bytes.size == 206024) { val ok = bridge.import2002CdSave(bytes); runOnUiThread { AlertDialog.Builder(this).setTitle(if (ok) "导入成功" else "导入失败").setMessage(if (ok) "2002 CD save imported." else "Invalid save.").setPositiveButton("OK", null).show() } } } catch (e: Exception) {} }.start()
    }
    private fun mapKey(code: Int): Int? = when (code) { KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> 0x0D; KeyEvent.KEYCODE_SPACE -> 0x20; KeyEvent.KEYCODE_ESCAPE -> 0x1B; KeyEvent.KEYCODE_TAB -> 0x09; KeyEvent.KEYCODE_CTRL_LEFT, KeyEvent.KEYCODE_CTRL_RIGHT -> 0x11; KeyEvent.KEYCODE_SHIFT_LEFT, KeyEvent.KEYCODE_SHIFT_RIGHT -> 0x10; KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_PAGE_UP -> 0x26; KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_PAGE_DOWN -> 0x28; KeyEvent.KEYCODE_F1 -> 0x70; else -> null }
    override fun onKeyDown(k: Int, e: KeyEvent): Boolean { val vk = mapKey(k); if (vk != null && e.repeatCount == 0) { bridge.keyEvent(vk, true); return true }; return super.onKeyDown(k, e) }
    override fun onKeyUp(k: Int, e: KeyEvent): Boolean { val vk = mapKey(k); if (vk != null) { bridge.keyEvent(vk, false); return true }; return super.onKeyUp(k, e) }
    override fun onBackPressed() {
        // DO NOT send ESC (0x1B) here. In the engine ESC toggles Backlog, so system back gesture opened Backlog.
        // Android back should close / exit, not open Backlog. Hardware Esc key still sends 0x1B via mapKey().
        if (quickMenuShowing || dialogShowing) return
        if (!bridge.hasGameAssets) { super.onBackPressed(); return }
        AlertDialog.Builder(this).setTitle(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "退出？" else "終了しますか？").setMessage(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "回到标题画面，还是关闭应用？" else "タイトルに戻るか、アプリを閉じるか選んでください。").setPositiveButton(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "关闭应用" else "アプリを閉じる") { _, _ -> finish() }.setNeutralButton(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "回到标题" else "タイトルに戻る") { _, _ -> bridge.queueAction(8) }.setNegativeButton(if (bridge.currentLanguageMode == EngineBridge.LANG_CHS) "取消" else "キャンセル", null).show()
    }
    override fun onPause() { super.onPause(); try { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) } catch (_: Exception) {}; try { gameView?.onPause() } catch (_: Exception) {}; try { audio?.pause() } catch (_: Exception) {}; if (!isDestroying && bridge.hasGameAssets) { Thread { try { bridge.saveGlobal() } catch (_: Exception) {} }.start() } }
    override fun onResume() { super.onResume(); gameView?.onResume(); if (bridge.hasGameAssets) audio?.resume() }
    override fun onDestroy() { if (isDestroying) { super.onDestroy(); return }; isDestroying = true; try { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) } catch (_: Exception) {}; try { gameView?.onPause() } catch (_: Exception) {}; try { audio?.stop() } catch (_: Exception) {}; try { if (bridge.hasGameAssets) bridge.saveGlobal() } catch (_: Exception) {}; try { bridge.destroy() } catch (_: Exception) {}; gameView = null; audio = null; super.onDestroy() }
}
