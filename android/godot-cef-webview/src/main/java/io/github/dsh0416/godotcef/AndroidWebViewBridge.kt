package io.github.dsh0416.godotcef

import android.app.Activity
import android.app.Presentation
import android.content.Context
import android.graphics.PixelFormat
import android.hardware.HardwareBuffer
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.Image
import android.media.ImageReader
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.view.Display
import android.view.MotionEvent
import android.view.Surface
import android.view.ViewGroup
import android.view.inputmethod.InputMethodManager
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.webkit.WebViewClient
import java.util.concurrent.ConcurrentHashMap

/**
 * Experimental Android peer for `AndroidWebViewTexture`.
 *
 * This source is intentionally isolated from the desktop CEF backend. A Godot Android plugin
 * should call `initialize(activity)` before the Rust GDExtension asks for a WebView session.
 */
@android.annotation.TargetApi(Build.VERSION_CODES.Q)
object AndroidWebViewBridge {
    private val sessions = ConcurrentHashMap<Long, Session>()
    private val imageThread = HandlerThread("GodotCefWebViewImages").apply { start() }
    private val imageHandler = Handler(imageThread.looper)

    @Volatile
    private var activity: Activity? = null

    @JvmStatic
    fun initialize(activity: Activity) {
        this.activity = activity
        WebView.setWebContentsDebuggingEnabled(true)
    }

    @JvmStatic
    fun create(id: Long, width: Int, height: Int, url: String, javaScriptEnabled: Boolean) {
        val currentActivity = requireActivity()
        val safeWidth = width.coerceAtLeast(1)
        val safeHeight = height.coerceAtLeast(1)

        currentActivity.runOnUiThread {
            sessions.remove(id)?.release()
            val session = Session(
                activity = currentActivity,
                id = id,
                width = safeWidth,
                height = safeHeight,
                url = url,
                javaScriptEnabled = javaScriptEnabled,
            )
            sessions[id] = session
            session.start()
        }
    }

    @JvmStatic
    fun shutdown(id: Long) {
        val session = sessions.remove(id) ?: return
        session.activity.runOnUiThread { session.release() }
    }

    @JvmStatic
    fun acquireLatestFrame(id: Long): HardwareBuffer? {
        return sessions[id]?.currentHardwareBuffer()
    }

    @JvmStatic
    fun getFrameWidth(id: Long): Int {
        return sessions[id]?.width ?: 0
    }

    @JvmStatic
    fun getFrameHeight(id: Long): Int {
        return sessions[id]?.height ?: 0
    }

    @JvmStatic
    fun loadUrl(id: Long, url: String) {
        sessions[id]?.runOnWebView { loadUrl(url) }
    }

    @JvmStatic
    fun eval(id: Long, code: String) {
        sessions[id]?.runOnWebView { evaluateJavascript(code, null) }
    }

    @JvmStatic
    fun reload(id: Long) {
        sessions[id]?.runOnWebView { reload() }
    }

    @JvmStatic
    fun goBack(id: Long) {
        sessions[id]?.runOnWebView {
            if (canGoBack()) {
                goBack()
            }
        }
    }

    @JvmStatic
    fun goForward(id: Long) {
        sessions[id]?.runOnWebView {
            if (canGoForward()) {
                goForward()
            }
        }
    }

    @JvmStatic
    fun focus(id: Long) {
        sessions[id]?.focusWebView()
    }

    @JvmStatic
    fun clearFocus(id: Long) {
        sessions[id]?.clearWebViewFocus()
    }

    @JvmStatic
    fun resize(id: Long, width: Int, height: Int) {
        val session = sessions[id] ?: return
        val currentUrl = session.currentUrl()
        val javaScriptEnabled = session.javaScriptEnabled
        shutdown(id)
        create(id, width, height, currentUrl, javaScriptEnabled)
    }

    @JvmStatic
    fun setJavaScriptEnabled(id: Long, enabled: Boolean) {
        sessions[id]?.runOnWebView {
            settings.javaScriptEnabled = enabled
        }
    }

    @JvmStatic
    fun touch(id: Long, pointerId: Int, x: Float, y: Float, pressed: Boolean) {
        val action = if (pressed) MotionEvent.ACTION_DOWN else MotionEvent.ACTION_UP
        sessions[id]?.dispatchMotion(pointerId, x, y, action)
    }

    @JvmStatic
    fun drag(id: Long, pointerId: Int, x: Float, y: Float) {
        sessions[id]?.dispatchMotion(pointerId, x, y, MotionEvent.ACTION_MOVE)
    }

    @JvmStatic
    fun mouseButton(id: Long, x: Float, y: Float, pressed: Boolean) {
        touch(id, 0, x, y, pressed)
    }

    @JvmStatic
    fun mouseMove(id: Long, x: Float, y: Float) {
        drag(id, 0, x, y)
    }

    private fun requireActivity(): Activity {
        return activity ?: error(
            "AndroidWebViewBridge.initialize(activity) must be called by the Android plugin first"
        )
    }

    private class Session(
        val activity: Activity,
        private val id: Long,
        var width: Int,
        var height: Int,
        private var url: String,
        var javaScriptEnabled: Boolean,
    ) {
        private val displayManager =
            activity.getSystemService(Context.DISPLAY_SERVICE) as DisplayManager
        private val lock = Any()

        private var imageReader: ImageReader? = null
        private var outputSurface: Surface? = null
        private var virtualDisplay: VirtualDisplay? = null
        private var presentation: WebPresentation? = null
        private var currentImage: Image? = null

        fun start() {
            imageReader = ImageReader.newInstance(
                width,
                height,
                PixelFormat.RGBA_8888,
                3,
                HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_GPU_COLOR_OUTPUT,
            ).also { reader ->
                reader.setOnImageAvailableListener({ onImageAvailable(reader) }, imageHandler)
                outputSurface = reader.surface
            }

            val densityDpi = activity.resources.displayMetrics.densityDpi
            virtualDisplay = displayManager.createVirtualDisplay(
                "godot-cef-webview-$id",
                width,
                height,
                densityDpi,
                outputSurface,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_PRESENTATION or
                    DisplayManager.VIRTUAL_DISPLAY_FLAG_OWN_CONTENT_ONLY,
            )

            val display = virtualDisplay?.display ?: return
            presentation = WebPresentation(activity, display, width, height, javaScriptEnabled)
                .also { webPresentation ->
                    webPresentation.show()
                    webPresentation.webView.loadUrl(url)
                }
        }

        fun release() {
            presentation?.dismiss()
            presentation = null
            virtualDisplay?.release()
            virtualDisplay = null
            outputSurface?.release()
            outputSurface = null
            imageReader?.close()
            imageReader = null
            synchronized(lock) {
                currentImage?.close()
                currentImage = null
            }
        }

        fun currentHardwareBuffer(): HardwareBuffer? {
            synchronized(lock) {
                return currentImage?.hardwareBuffer
            }
        }

        fun currentUrl(): String {
            return presentation?.webView?.url ?: url
        }

        fun runOnWebView(block: WebView.() -> Unit) {
            activity.runOnUiThread {
                presentation?.webView?.block()
            }
        }

        fun dispatchMotion(pointerId: Int, x: Float, y: Float, action: Int) {
            activity.runOnUiThread {
                val eventTime = android.os.SystemClock.uptimeMillis()
                val event = MotionEvent.obtain(
                    eventTime,
                    eventTime,
                    action,
                    x,
                    y,
                    0,
                )
                event.source = android.view.InputDevice.SOURCE_TOUCHSCREEN
                presentation?.webView?.let { webView ->
                    if (action == MotionEvent.ACTION_DOWN) {
                        webView.requestFocus()
                        showKeyboard(webView)
                    }
                    webView.dispatchTouchEvent(event)
                }
                event.recycle()
            }
        }

        fun focusWebView() {
            activity.runOnUiThread {
                presentation?.webView?.let { webView ->
                    webView.requestFocus()
                    showKeyboard(webView)
                }
            }
        }

        fun clearWebViewFocus() {
            activity.runOnUiThread {
                presentation?.webView?.let { webView ->
                    webView.clearFocus()
                    hideKeyboard(webView)
                }
            }
        }

        private fun showKeyboard(webView: WebView) {
            val imm = activity.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
            imm.showSoftInput(webView, InputMethodManager.SHOW_IMPLICIT)
        }

        private fun hideKeyboard(webView: WebView) {
            val imm = activity.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
            imm.hideSoftInputFromWindow(webView.windowToken, 0)
        }

        private fun onImageAvailable(reader: ImageReader) {
            val image = reader.acquireLatestImage() ?: return
            synchronized(lock) {
                currentImage?.close()
                currentImage = image
            }
        }
    }

    private class WebPresentation(
        context: Context,
        display: Display,
        private val width: Int,
        private val height: Int,
        private val javaScriptEnabled: Boolean,
    ) : Presentation(context, display) {
        lateinit var webView: WebView
            private set

        override fun onCreate(savedInstanceState: Bundle?) {
            super.onCreate(savedInstanceState)
            webView = WebView(context).apply {
                layoutParams = ViewGroup.LayoutParams(width, height)
                isFocusable = true
                isFocusableInTouchMode = true
                settings.javaScriptEnabled = javaScriptEnabled
                webViewClient = WebViewClient()
                webChromeClient = WebChromeClient()
            }
            setContentView(webView)
        }
    }
}
