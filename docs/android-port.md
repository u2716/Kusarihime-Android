# Android port (min SDK 26 / Android 8+)
One-shot: `python3 port_to_android.py` from repo root, then `./scripts/build-android.sh`.
## Architecture
Rust core is unchanged except `cdylib` + `android_jni.rs` (JNI over the existing C11 FFI).
Kotlin shell mirrors iOS: `EngineBridge` (paths, fonts, language, dialogs, saves),
`GameView` (GLES2 textured quad, 800x600 letterbox, touch), `AudioOutput` (AudioTrack float),
`MainActivity` (lifecycle, keyboard, quick menu, SAF import).
## Paths
- Game: `filesDir/game/` (or `getExternalFilesDir()/game/`). Copy via `adb push game/ /sdcard/Android/data/com.fujihime.euthanasia/files/game/` or in-app Pick game folder (SAF DocumentFile).
- Save: `filesDir/save/KhDLsave.dat` (byte-compatible with PC/iOS).
- Override: `filesDir/override/`. Fonts: `filesDir/Fonts/` or `assets/Fonts/` (bytes passed to `load_font`, never Android font system).
- Bundled assets: put game under `src/android/app/src/main/assets/game/` and use Copy bundled assets (copies to filesDir on first launch, needed because Rust needs filesystem paths, not AssetManager).
## Input (same as iOS)
Single tap Move/Down/Up, 2-finger tap RightUp, 2-finger pan Wheel, long-press 0.45s Ctrl/skip, 3-finger tap quick menu. Keyboard: Enter/Space/Esc/Tab/Ctrl/Shift/Up/Down/PageUp/PageDown/F1 -> same VK codes.
## Build notes
- NDK r27, `targetSdk 34`, `minSdk 26`, ABIs `arm64-v8a` + `x86_64` (emulator). Add `armeabi-v7a` only if you need 32-bit.
- GLSL fragment does BGRA swizzle (`c.bgra`) so no CPU BGRA->RGBA conversion.
- Audio is pull-model: Rust `render_audio_f32` -> direct ByteBuffer -> AudioTrack PCM_FLOAT 44.1kHz stereo.
- If Gradle cannot find Rust .so, run `scripts/build-android.sh` first or set `FUJIHIME_SKIP_RUST=1` and place .so manually.
## Play Store note
APK + 895MB game cannot go in one APK. For Play, use Play Asset Delivery / split APK, or ship an empty-shell APK + SAF import (same as iOS Files-app flow).
