//! Core engine library for 《腐姫～euthanasia～》.
//!
//! Phase 2A subsystems:
//! - [`vfs`]: `.xfl` (`LB\x01\x00`) archive reader and layered `AssetVfs` resolver
//!   supporting `override/` hot-patching, original Japanese ground-truth archives,
//!   and CHS V1.0 scattered directories / `.bmp` replacements.
//! - [`gfx`]: `CMath` lookup tables, inverted-alpha `Surface`, `.wcg`/`.lwg`/`.msk`/`.bmp`
//!   decoders, `0x436430` 2D software compositor + Porter-Duff `Over` + filters,
//!   and `0x422060` full `0..=299` screen transition system.
//!
//! Phase 2B subsystems:
//! - [`text`]: FreeType 2 font loading, 6 glyph rasterization modes (`0x44f080`),
//!   DBCS-safe `^` control codes (`0x441180`), 58/60-item kinsoku table (`0x441bf0`),
//!   layout engine (`0x441fe0`), and `50 ms` typewriter timer (`0x447a90`).
//! - [`audio`]: RIFF `0x6771` (`vorbis.acm`) demuxer, `0xFFFFFFFF` dummy-page stripper,
//!   pure-Rust Ogg Vorbis PCM decoder, and DirectSound 4-slot software mixer.
//! - [`video`]: Native MPEG-1 video & MP2 audio playback via MIT `pl_mpeg`, plus
//!   `CMpegWindow` (`0x4505f0`..`0x450a60`) state machine.
//!
//! Phase 2C subsystems:
//! - [`save`]: Byte-exact `KhDLsave.dat` (2007 DL & 2002 CD) and `KhDLsave%02d.dat`
//!   (`75,026 B` dual-snapshot) serialization and migration.
//! - [`vm`]: Full `.gsc` parser/disassembler and bytecode VM (`0x1000..0xF000` +
//!   all 120 command opcodes `0x03..0xD5`).
//! - [`engine`]: Integrated game engine coordinating all subsystems and native dialogs.
//! - [`ffi`]: Stable C11 ABI FFI boundary (`include/fujihime_core.h`) for Swift iOS 14.4+.

pub mod audio;
#[cfg(target_os = "android")]
pub mod android_jni;
pub mod engine;
pub mod error;
pub mod ffi;
pub mod gfx;
pub mod save;
pub mod text;
pub mod ui;
pub mod vfs;
pub mod video;
pub mod vm;

pub use error::{FujihimeError, Result};
