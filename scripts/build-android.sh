#!/usr/bin/env bash
# Build Fujihime Android APK (min SDK 26). Requires JDK 17, Android SDK 34 + NDK r27, Rust + cargo-ndk.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"
if ! command -v cargo-ndk >/dev/null 2>&1; then echo "Installing cargo-ndk..."; cargo install cargo-ndk --locked || cargo install cargo-ndk; fi
rustup target add aarch64-linux-android x86_64-linux-android || true
cd "$ROOT/src/core"
cargo ndk -t arm64-v8a -t x86_64 -o "$ROOT/src/android/app/src/main/jniLibs" build --release
# cargo-ndk outputs libfujihime_core.so per ABI; verify
find "$ROOT/src/android/app/src/main/jniLibs" -name "*.so" -print
cd "$ROOT/src/android"
if [ -x ./gradlew ]; then ./gradlew assembleDebug; else gradle assembleDebug; fi
echo "APK: $ROOT/src/android/app/build/outputs/apk/debug/app-debug.apk"
