# 腐姫～euthanasia～ iOS 14.4+ Port (`fujihime-ios`)

[[CI and iOS 14.4 IPA Build](https://github.com/turboegg1145/fujihime-ios/actions/workflows/ci-and-ios-ipa.yml/badge.svg)](https://github.com/turboegg1145/fujihime-ios/actions/workflows/ci-and-ios-ipa.yml)
[Platform](https://img.shields.io/badge/Platform-iOS%2014.4%2B%20(arm64)-007AFF?logo=apple)
[Language](https://img.shields.io/badge/Core-Rust%20%2B%20Swift%205.0%20%2B%20Metal-DEA584)
[License](https://img.shields.io/badge/License-MIT-green)

> **1:1 faithful iOS 14.4+ (`arm64`) engine reimplementation of *Kusarihime ~euthanasia~* (*《腐姫～euthanasia～》*, 2007 DL Edition)** — built in Rust, FreeType 2.13.2, `pl_mpeg`, Metal, UIKit, and `AVAudioEngine` based on full Ghidra/REA reverse engineering of Liar-soft's `RScript` / `LiosGameClass` (`KhimeDL.exe`).

This project is a 1:1 faithful iOS 14.4+ (`arm64`) port engine for Liar-soft's classic visual novel **《腐姫～euthanasia～》 (2007 DL Edition)**. Based on function-by-function reverse engineering of the original Japanese executable `KhimeDL.exe` (`RScript` / `LiosGameClass`) and byte-level verification of all assets, it rebuilds all subsystems from scratch in Rust + C11 FFI + Swift/Metal/AVAudioEngine, without an emulator or compatibility layer, accurately reproducing the original graphics, sound, presentation, script logic, and save format.

---

## Core Features

### 1. 100% Faithful Original Engine Reproduction
- **`.xfl` Virtual File System (VFS)**: Full support for original `.xfl` archives, loose-directory overrides, and the `Documents/override/` hot-override mechanism.
- **Image Decoding and Graphics Pipeline**: Complete implementation of `.wcg`, `.lwg` (including embedded multi-layer), `.msk`, `.bmp` (including 32bpp patch textures) decoding, reproducing the `CMath` LUT exact pixel blending algorithm and the full `0..=299` set of transition effects (including zoom / blinds / crossfade / masks, etc.).
- **Pixel-Level Glyph Typesetting (FreeType 2.13.2)**: Statically compiled FreeType 2.13.2 built in, fully reproducing 6 character rendering modes, line-start / line-end kinsoku (prohibition) handling, external character mapping, and `^` control sequences (pause, speed, color, ruby / line break, etc.).
- **Audio and Video**:
  - `vorbis.acm` (Format Tag `0x6771`) Ogg Vorbis packet decoding + native PCM `.wav` decoding;
  - Independent 4-channel mixer (BGM / SE / Voice / Movie audio), accurately reproducing fade-in / fade-out, loop counts, and `AVAudioEngine` hardware output;
  - `pl_mpeg` native software decoding of `0001.mpg` / `0002.mpg` MPEG-1 opening and ending animations and MP2 stereo audio tracks, stretched to `800×600` following the original MCI logic.
- **Complete `.gsc` VM and Dual-Snapshot Rollback**: All 120 bytecode opcodes fully disassembled and execution-verified, supporting the `save` / `rollback` dual-snapshot mechanism, covering all 341 original and Chinese-translated script files.
- **Save Compatibility and All 9 UI Scenes**:
  - Byte-level compatibility with **2007 DL original saves (`KhDLsave.dat`)**, with built-in automatic detection and migration import for **2002 CD saves (`KhDLsave_2002CD.dat`)**;
  - Faithful recreation of all **9 original UI scenes**: title screen, main game screen, Save / Load, Config, Backlog, CG Gallery, BGM Gallery, Scene Recollection, etc.

### 2. Original Japanese & Chinese Translation V1.0 Hot Switching
- Supports both **original Japanese (CP932)** and **Simplified Chinese translation V1.0 (GBK)** at the same time.
- At runtime, **three-finger tap** brings up the native quick menu, where you can hot-switch language mode in one tap. The choice is automatically persisted, so you can keep playing or load a save without restarting.

### 3. Faithful Original Look and Unobstructed Interaction
- Locked landscape **4:3** original logical frame (`800×600`) centered on screen, with pure black letterbox / pillarbox bars on all sides, **zero extra virtual floating buttons**.
- Faithfully recreates the original `compane` (bottom control panel) interaction, with support for system shortcut functions and mouse hover / click feedback.

### 4. Touch Gestures and External Keyboard Controls

#### Touch Gestures
| Gesture | Original Operation | Notes |
|---|---|---|
| **Single-finger tap / drag** | Mouse left-button move and click | Advance story, select options, operate `compane` and UI panels |
| **Two-finger tap** | Mouse right button | Hide / show textbox, cancel, return to previous menu |
| **Two-finger vertical swipe** | Mouse wheel | Swipe up to open / browse Backlog, swipe down to advance / roll back |
| **Single-finger long-press (≥ 0.45s)** | Hold `Ctrl` | Hold to trigger Skip (fast-forward) mode, release to stop |
| **Three-finger tap** | — | Bring up iOS native quick menu (language switch, Save / Load, Config, Auto / Skip, Backlog, Rollback, 2002 CD save import, controls help) |

#### External Hardware Keyboard (iPad / Bluetooth Keyboard)
| Key | Virtual Key Code (`VK_*`) | Function |
|---|---|---|
| `Enter` / `Space` | `0x0D` / `0x20` | Advance text / Confirm |
| `Esc` | `0x1B` | Right-button function (hide textbox / back) |
| `Tab` | `0x09` | Hide / show textbox |
| `Left/Right Ctrl` | `0x11` | Hold for Skip (fast-forward) |
| `Left/Right Shift` | `0x10` | Shift modifier key |
| `↑` / `PageUp` | `0x26` | Scroll up / open Backlog |
| `↓` / `PageDown` | `0x28` | Scroll down / advance |
| `F1` | `0x70` | Original F1 shortcut |

### 5. Built-in Local Desktop Debugger (Local Web Player)
- The project includes a local Web Player built from the Rust engine (`./scripts/run-web-player.sh`), which lets you debug script flow, rendering output, and input events directly in a desktop browser.

### 6. Deep iOS 14.4 Compatibility
- CI pins the **Xcode 16.4** toolchain and `IPHONEOS_DEPLOYMENT_TARGET=14.4` (`Swift 5.0`) to avoid `dyld` crashes on old systems caused by newer iOS SDKs;
- Forces `-Wl,-no_fixup_chains` to disable `LC_DYLD_CHAINED_FIXUPS`, and keeps **zero `Swift Concurrency` (`libswift_Concurrency`)** and **zero non-system `@rpath` dylib** dependencies;
- Every CI build automatically runs **6 Mach-O & `Info.plist` gate checks**: `arm64` architecture, `minos 14.4`, no `LC_DYLD_CHAINED_FIXUPS`, no `libswift_Concurrency`, no `@rpath` external dylibs, `MinimumOSVersion == 14.4`.

---

## Repository Structure at a Glance

```text
.
├── .github/workflows/
│   └── ci-and-ios-ipa.yml      # Rust Core CI + iOS 14.4 arm64 IPA auto-build and 6 gate checks
├── docker/ & docker-compose.yml # Isolated reverse-engineering environment (Ghidra 12.1.4 + REA + JDK 25 + Rust/GCC)
├── docs/
│   ├── formats-and-gfx-spec.md  # Phase 1A: .xfl/.wcg/.lwg/.msk, CMath, transitions and MPEG-1 reverse-engineering spec
│   ├── vm-opcodes-spec.md       # Phase 1B: .gsc format and all 120 VM Opcode reverse-engineering specs
│   ├── text-audio-save-ui-spec.md # Phase 1C: text typesetting, audio mixing, saves and 9 UI scenes reverse-engineering specs
│   ├── ios14.4-porting-plan.md  # iOS 14.4 overall port architecture and gate planning
│   └── game-assets-manifest.sha256 # Original game assets SHA-256 integrity baseline
├── re/
│   ├── evidence/                # Ghidra/REA exported JSON structured reverse-engineering evidence
│   └── verify_*.py              # Full asset (images / scripts / audio / saves) automated verification scripts
├── scripts/
│   ├── docker-up.sh             # Start isolated analysis container
│   ├── rea-docker.sh            # REA CLI wrapper inside container
│   ├── package-game-assets.sh   # Package offline game assets and fonts into tar
│   └── run-web-player.sh        # Start local desktop Web Player debugger
└── src/
    ├── core/                    # Rust core engine (VFS / decoding / compositing / FreeType / audio / MPEG / VM / C11 FFI)
    └── ios/                     # Swift + Metal + UIKit + AVAudioEngine iOS 14.4 shell and XcodeGen config
```

---

## Install and Use (iOS 14.4+)

### 1. Download and Install the IPA
1. Go to this repo's **[GitHub Releases (`v1.0.0`)](https://github.com/turboegg1145/fujihime-ios/releases/tag/v1.0.0)** and download `Fujihime-iOS14.4-arm64.ipa` (includes all game assets and open-source Japanese / Chinese fonts needed to run).
2. Install to an `arm64` device with any common iOS signing / install tool (requires **iOS 14.4 or later**):
   - **TrollStore** (recommended, supports permanent install on specific iOS 14.0–17.0 versions)
   - **AltStore** / **SideStore** / **Sideloadly** / **Esign** and other self-signing install tools

> If you use the empty-shell IPA built by this repo's CI (a `push` build without the asset pack), manually copy the game assets into the app's shared directory after installing (see "File Management" below).

### 2. Import Saves and Override Resources (Files App)
This app enables `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`, so you can manage files directly in iOS **Files App → On My iPhone / iPad → 腐姫**:
- **`Documents/save/`**: Save persistence directory.
  - Original 2007 DL save `KhDLsave.dat` can be placed directly in this directory for two-way PC ↔ iOS compatibility;
  - If you have a **2002 CD save**, rename it to `KhDLsave_2002CD.dat`, put it in `Documents/save/`, then three-finger tap to open the quick menu → select "Import 2002 CD Save" to auto-convert it to 2007 DL format while keeping progress.
- **`Documents/override/`**: Hot-override directory. `.xfl`, `grpe/`, `grpo_ex/`, `grps/`, `mov/`, or font files placed here load with priority over the installed package resources.

---

## Build from Source

### 1. Run Rust Core Unit Tests and Static Checks
```bash
cd src/core
cargo clippy --all-targets -- -D warnings
cargo test --release
gcc -std=c11 -Wall -Wextra -Werror -fsyntax-only include/fujihime_core.h
```

### 2. Start the Local Web Player (Desktop Debugging)
Make sure the original game directory is ready (default `game/腐姫ＤＬ`, or specify via the `FUJIHIME_GAME_DIR` env var) and the container is running, then run:
```bash
./scripts/run-web-player.sh
```
Then open `http://127.0.0.1:8080` in your browser to run the debugger.

### 3. Build the iOS 14.4 `arm64` IPA
- **GitHub Actions auto-build**:
  - Every `push` automatically runs `rust-core-ci` and builds the unsigned empty-shell `Fujihime-iOS14.4-arm64.ipa` (verifying the full Xcode 16.4 + 6 iOS 14.4 gates).
  - When manually triggering `workflow_dispatch`, you can fill the optional `assets_release_tag` (pointing to a private Release tag containing `fujihime-game-assets-v1.tar`, which can be generated with `./scripts/package-game-assets.sh`), and CI will automatically inject the game assets and package the full IPA.
- **Local macOS build**: Requires Xcode 16.4, Rust `aarch64-apple-ios` target, and `xcodegen`:
  ```bash
  cd src/core
  IPHONEOS_DEPLOYMENT_TARGET=14.4 cargo build --release --target aarch64-apple-ios --lib
  mkdir -p ../ios/Vendor && cp target/aarch64-apple-ios/release/libfujihime_core.a ../ios/Vendor/
  cd ../ios && xcodegen generate
  xcodebuild -project Fujihime.xcodeproj -scheme Fujihime -configuration Release -sdk iphoneos -arch arm64 \
    IPHONEOS_DEPLOYMENT_TARGET=14.4 CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY="" build
  ```

---

## Disclaimer and Copyright

- The original engine source code, iOS shell code, build configs, reverse-engineering tool scripts, and technical docs in this repo are open-sourced under **[MIT License](LICENSE)**.
- The game script, character art, CG, backgrounds, voice, music, video, and related trademarks of **《腐姫～euthanasia～》 (Kusarihime ~euthanasia~)** all belong to **Liar-soft**. **This source tree contains no original copyrighted game assets.**
- This project is for computer reverse-engineering research, cross-platform compatibility preservation of old software, and save porting for personally purchased legitimate copies only. Please support the original game.

---

## Android Port

**App:** `腐り姫～euthanasia～` — Package `com.fujihime.euthanasia` — Requires Android 8+ (SDK 26)

Same Rust core engine as the iOS version, with a Kotlin / `GLSurfaceView` + `AudioTrack` shell instead of Swift / Metal. Same 1:1 original experience, same `800×600` 4:3 letterboxed display, same touch gestures (single tap, two-finger tap, two-finger swipe, long-press, three-finger quick menu), same Japanese / Chinese hot-switching, and same byte-compatible saves (`KhDLsave.dat`, including 2002 CD save import).
