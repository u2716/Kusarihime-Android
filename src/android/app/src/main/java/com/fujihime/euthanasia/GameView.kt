package com.fujihime.euthanasia
import android.content.Context
import android.opengl.GLES20
import android.opengl.GLSurfaceView
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10
import kotlin.math.hypot

class GameView(ctx: Context, val bridge: EngineBridge, val onQuickMenu: () -> Unit, val onDialog: (PendingDialogInfo) -> Unit, val onState: (EngineState) -> Unit) : GLSurfaceView(ctx) {
    private val renderer = Renderer()
    private val mainHandler = Handler(Looper.getMainLooper())
    private var activePointerId = -1; private var touchStartX = 0f; private var touchStartY = 0f
    private var touchSentDown = false; private var multiTouch = false; private var longPressFired = false
    private var panAccY = 0f; private var secondDownTime = 0L; private var secondMoved = false
    private val longPressRunnable = Runnable { if (activePointerId != -1 && !multiTouch) { longPressFired = true; bridge.keyEvent(0x11, true) } }
    init { setEGLContextClientVersion(2); setRenderer(renderer); renderMode = RENDERMODE_CONTINUOUSLY; isFocusableInTouchMode = true }
    fun viewportRect(w: Float, h: Float): FloatArray { val target = EngineBridge.GAME_WIDTH.toFloat() / EngineBridge.GAME_HEIGHT; val view = w / h; return if (view > target) { val rw = h * target; floatArrayOf((w - rw) / 2, 0f, rw, h) } else { val rh = w / target; floatArrayOf(0f, (h - rh) / 2, w, rh) } }
    fun viewToGame(x: Float, y: Float): Pair<Int, Int>? { val r = viewportRect(width.toFloat(), height.toFloat()); if (x < r[0] || x > r[0] + r[2] || y < r[1] || y > r[1] + r[3]) return null; return ((x - r[0]) / r[2] * EngineBridge.GAME_WIDTH).toInt().coerceIn(0, 799) to ((y - r[1]) / r[3] * EngineBridge.GAME_HEIGHT).toInt().coerceIn(0, 599) }
    fun clampedViewToGame(x: Float, y: Float): Pair<Int, Int> { val r = viewportRect(width.toFloat(), height.toFloat()); val cx = x.coerceIn(r[0], r[0] + r[2]); val cy = y.coerceIn(r[1], r[1] + r[3]); return ((cx - r[0]) / r[2] * EngineBridge.GAME_WIDTH).toInt().coerceIn(0, 799) to ((cy - r[1]) / r[3] * EngineBridge.GAME_HEIGHT).toInt().coerceIn(0, 599) }
    private fun cancelPendingLongPress() { mainHandler.removeCallbacks(longPressRunnable) }
    private fun finishSingle(cancelled: Boolean) { cancelPendingLongPress(); if (longPressFired) bridge.keyEvent(0x11, false) else if (touchSentDown && !multiTouch) { if (!cancelled) { val p = clampedViewToGame(touchStartX, touchStartY); bridge.pointerEvent(2, p.first, p.second) } else bridge.pointerEvent(2, -1, -1) }; activePointerId = -1; touchSentDown = false; longPressFired = false }
    override fun onTouchEvent(e: MotionEvent): Boolean {
        if (!bridge.hasGameAssets) return true
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> { val g = viewToGame(e.x, e.y) ?: return true; activePointerId = e.getPointerId(0); touchStartX = e.x; touchStartY = e.y; touchSentDown = true; multiTouch = false; longPressFired = false; bridge.pointerEvent(0, g.first, g.second); bridge.pointerEvent(1, g.first, g.second); mainHandler.postDelayed(longPressRunnable, 450) }
            MotionEvent.ACTION_MOVE -> {
                if (e.pointerCount >= 2) { panAccY += e.getY(1) - touchStartY; touchStartY = e.getY(1); if (kotlin.math.abs(panAccY) > 8) secondMoved = true; while (panAccY >= 28) { panAccY -= 28; bridge.pointerEvent(5, 400, 300) }; while (panAccY <= -28) { panAccY += 28; bridge.pointerEvent(6, 400, 300) }; return true }
                val idx = e.findPointerIndex(activePointerId); if (idx < 0 || multiTouch) return true; val x = e.getX(idx); val y = e.getY(idx); if (hypot(x - touchStartX, y - touchStartY) >= 16) cancelPendingLongPress(); val g = clampedViewToGame(x, y); bridge.pointerEvent(0, g.first, g.second)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                if (e.pointerCount == 3) { cancelPendingLongPress(); if (touchSentDown) bridge.pointerEvent(2, -1, -1); touchSentDown = false; activePointerId = -1; multiTouch = true; mainHandler.post { onQuickMenu() }; return true }
                if (e.pointerCount == 2) { cancelPendingLongPress(); multiTouch = true; secondDownTime = System.currentTimeMillis(); secondMoved = false; panAccY = 0f; touchStartY = e.getY(1); if (touchSentDown) bridge.pointerEvent(2, -1, -1); touchSentDown = false; return true }
            }
            MotionEvent.ACTION_POINTER_UP -> { if (e.pointerCount == 2 && multiTouch && System.currentTimeMillis() - secondDownTime < 350 && !secondMoved) bridge.pointerEvent(3, 400, 300); if (e.pointerCount - 1 <= 1) multiTouch = false }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> { if (activePointerId != -1) finishSingle(e.actionMasked == MotionEvent.ACTION_CANCEL) else multiTouch = false }
        }
        return true
    }
    inner class Renderer : GLSurfaceView.Renderer {
        private var program = 0; private var texture = 0; private var lastNs = 0L
        private val quad: FloatBuffer = ByteBuffer.allocateDirect(16 * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().apply { put(floatArrayOf(-1f,-1f, 1f,-1f, -1f,1f, 1f,1f)); position(0) }
        private val texCoords: FloatBuffer = ByteBuffer.allocateDirect(8 * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().apply { put(floatArrayOf(0f,1f, 1f,1f, 0f,0f, 1f,0f)); position(0) }
        private fun shader(type: Int, src: String): Int { val s = GLES20.glCreateShader(type); GLES20.glShaderSource(s, src); GLES20.glCompileShader(s); return s }
        override fun onSurfaceCreated(gl: GL10?, c: EGLConfig?) {
            program = GLES20.glCreateProgram(); GLES20.glAttachShader(program, shader(GLES20.GL_VERTEX_SHADER, "attribute vec2 aPos; attribute vec2 aTex; varying vec2 vTex; uniform vec2 uScale; void main(){ vTex=aTex; gl_Position=vec4(aPos*uScale,0.0,1.0); }")); GLES20.glAttachShader(program, shader(GLES20.GL_FRAGMENT_SHADER, "precision mediump float; varying vec2 vTex; uniform sampler2D uTex; void main(){ vec4 c=texture2D(uTex,vTex); gl_FragColor=vec4(c.b,c.g,c.r,1.0); }")); GLES20.glLinkProgram(program)
            val tex = IntArray(1); GLES20.glGenTextures(1, tex, 0); texture = tex[0]; GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, texture); GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR); GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR); GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE); GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
            GLES20.glClearColor(0f, 0f, 0f, 1f)
        }
        override fun onSurfaceChanged(gl: GL10?, w: Int, h: Int) { GLES20.glViewport(0, 0, w, h) }
        override fun onDrawFrame(gl: GL10?) {
            val now = System.nanoTime(); val delta = if (lastNs == 0L) 16 else ((now - lastNs) / 1_000_000).toInt().coerceIn(1, 100); lastNs = now
            if (!bridge.hasGameAssets) { GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT); return }
            bridge.tick(delta)
            if (bridge.copyPresent()) { GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, texture); GLES20.glTexImage2D(GLES20.GL_TEXTURE_2D, 0, GLES20.GL_RGBA, EngineBridge.GAME_WIDTH, EngineBridge.GAME_HEIGHT, 0, GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, bridge.frameBuffer) }
            GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT); GLES20.glUseProgram(program)
            val target = EngineBridge.GAME_WIDTH.toFloat() / EngineBridge.GAME_HEIGHT; val view = width.toFloat() / height.coerceAtLeast(1); val (sx, sy) = if (view > target) (target / view) to 1f else 1f to (view / target)
            GLES20.glUniform2f(GLES20.glGetUniformLocation(program, "uScale"), sx, sy)
            val posLoc = GLES20.glGetAttribLocation(program, "aPos"); val texLoc = GLES20.glGetAttribLocation(program, "aTex")
            GLES20.glEnableVertexAttribArray(posLoc); GLES20.glVertexAttribPointer(posLoc, 2, GLES20.GL_FLOAT, false, 0, quad)
            GLES20.glEnableVertexAttribArray(texLoc); GLES20.glVertexAttribPointer(texLoc, 2, GLES20.GL_FLOAT, false, 0, texCoords)
            GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
            val st = bridge.getState(); if (st != null) { mainHandler.post { onState(st); if (st.exitRequested) { bridge.saveGlobal(); bridge.startTitle() } } }
            val dlg = bridge.pollPendingDialog(); if (dlg != null) mainHandler.post { onDialog(dlg) }
        }
    }
}
