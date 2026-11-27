//! High-level game engine coordinating VFS, 2D compositor, text rendering, audio mixer,
//! MPEG-1 playback, `.gsc` VM, save persistence, all 9 original UI scenes, and native dialogs.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use encoding_rs::{GBK, SHIFT_JIS};

use crate::audio::{decode_vorbis_acm_wav, AudioMixer, DecodedAudio, LoopMode};
use crate::error::{FujihimeError, Result};
use crate::gfx::compositor::{
    blit, filter_fill_argb, stretch_blit_srccopy, BlitParams, Canvas800x600, Rect,
};
use crate::gfx::decode::{decode_bmp, decode_lwg, decode_wcg, LwgImage};
use crate::gfx::surface::{Surface, GAME_HEIGHT, GAME_WIDTH};
use crate::gfx::transition::TransitionRunner;
use crate::save::{ConfigBlock2007, GlobalSave2002Cd, GlobalSave2007, SlotSave2007};
use crate::text::caret::{parse_caret_bytes, GaijiBank, TextAlign, TextEncoding, TextStyle};
use crate::text::freetype::{FontFamily, FontLibrary, FontSet};
use crate::text::layout::{
    layout_text, layout_text_with_indent, render_layout_to_surface, TypewriterState,
    TBOX00_TEXT_RECT, TEXT_TIMER_PERIOD_MS,
};
use crate::text::rasterizer::RasterMode;
use crate::ui::widgets::render_ascii_or_utf8_label;
use crate::ui::{
    blit_fullscreen_srccopy, confirm_action_text, find_recall_0399_route,
    find_replay_entry, messagebox_text, BacklogAction, BacklogOverlay,
    CgGalleryState, CompaneButton, CompaneWidget, ConfScrnAction, ConfScrnOverlay, ExtraAction,
    ExtraMenuState, MessageBoxCallsite, MusicModeState, SaveScrnAction, SaveScrnMode,
    SaveScrnOverlay, SceneReplayState, SelMapOverlay, Select6Overlay, SystemPanelAction,
    TitleAction, TitleSceneState, UiRect, CG_GALLERY_TABLE, SCENE_REPLAY_TABLE,
};
use crate::vfs::{AssetCategory, AssetVfs, LanguageMode, VfsConfig, XflArchive};
use crate::video::{MpegWindowController, MpegWindowEvent, MpegWindowPhase};
use crate::vm::{ChoicePrompt, GscScript, HaltReason, VmHost, VmRuntime, VmState, VmYield};

/// Top-level UI scenes matching `KhimeDL.exe` (`0x41d010`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TopScene {
    Title = 1,
    MainGame = 2,
    LoadMenu = 3,
    ConfigMenu = 4,
    CgGallery = 5,
    SceneReplay = 6,
    MusicMode = 7,
    ExtraMenu = 8,
    SubsceneDll = 9,
}

/// Font role passed when injecting TTF/TTC bytes (`AppBundle/Fonts/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FontRole {
    Gothic = 0,
    Mincho = 1,
    Fallback = 2,
}

/// Pointer input kind matching Win32 `WndProc` (`0x41a2a0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PointerKind {
    Move = 0,
    Down = 1,
    Up = 2,
    RightUp = 3,
    MiddleUp = 4,
    WheelUp = 5,
    WheelDown = 6,
}

/// Dialog modality (`MessageBoxA` `MB_OK` vs `0x445e90` `MB_YESNO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DialogKind {
    OkAlert = 0,
    YesNoConfirm = 1,
}

/// Action to run when a `YesNoConfirm` dialog is accepted (`IDYES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    ExitGame,
    ReturnToTitle,
    SaveOverwrite(u16),
    LoadSlot(u16),
    AcknowledgeOnly,
}

/// Pending native dialog surfaced to iOS `UIAlertController`.
#[derive(Debug, Clone)]
pub struct PendingDialog {
    pub id: u64,
    pub kind: DialogKind,
    pub title: String,
    pub message: String,
    pub action: ConfirmAction,
}

/// Engine startup configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub game_root: PathBuf,
    pub save_dir: PathBuf,
    pub override_dir: Option<PathBuf>,
    pub orig_backup_dir: Option<PathBuf>,
    pub language_mode: LanguageMode,
    pub repair_chs_recall_labels: bool,
}

impl EngineConfig {
    pub fn new(
        game_root: impl Into<PathBuf>,
        save_dir: impl Into<PathBuf>,
        language_mode: LanguageMode,
    ) -> Self {
        Self {
            game_root: game_root.into(),
            save_dir: save_dir.into(),
            override_dir: None,
            orig_backup_dir: None,
            language_mode,
            repair_chs_recall_labels: false,
        }
    }
}

/// Snapshot of runtime engine state for UI/FFI queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineStatus {
    pub scene: TopScene,
    pub language_mode: LanguageMode,
    pub is_playing_video: bool,
    pub is_waiting_click: bool,
    pub is_waiting_select: bool,
    pub is_transitioning: bool,
    pub exit_requested: bool,
    pub current_script_id: u16,
    pub current_pc: u32,
}

struct ActiveTransition {
    runner: TransitionRunner,
    accum_ms: u32,
    next_delay_ms: u16,
}

struct ActiveTextRender {
    layout: crate::text::layout::TextLayout,
    typewriter: TypewriterState,
    accum_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleMovieStage {
    None,
    Playing0002,
    Playing0001,
}

/// Host backing resources for [`VmRuntime`].
struct EngineResources {
    game_root: PathBuf,
    override_dir: Option<PathBuf>,
    orig_backup_dir: Option<PathBuf>,
    vfs: AssetVfs,
    orig_scr_backup: Option<XflArchive>,
    repair_chs_recall_labels: bool,
    language_mode: LanguageMode,
    save_dir: PathBuf,
    global_save: GlobalSave2007,
    canvas: Canvas800x600,
    font_lib: FontLibrary,
    fonts: Option<FontSet>,
    pending_mincho: Option<crate::text::freetype::FontFace>,
    gaiji_bank: GaijiBank,
    lwg_cache: HashMap<String, Arc<LwgImage>>,
    surface_cache: HashMap<(AssetCategory, String), Arc<Surface>>,
    audio_cache: HashMap<(AssetCategory, String), Arc<DecodedAudio>>,
    last_loaded_se: Option<Arc<DecodedAudio>>,
    mixer: Arc<Mutex<AudioMixer>>,
    video_audio_fifo: Arc<Mutex<VecDeque<f32>>>,
    mpeg: MpegWindowController,
    transition: Option<ActiveTransition>,
    active_text: Option<ActiveTextRender>,
    textbox_hidden_by_user: bool,
    hovered_vm_layer: Option<usize>,
    show_wait_cursor: bool,
    wait_cursor_ms: u32,
    last_error: Option<String>,
}

#[inline]
fn anchored_top_left(
    x: i32,
    y: i32,
    anchor_x: i32,
    anchor_y: i32,
    width: i32,
    height: i32,
) -> (i32, i32) {
    let ox = if anchor_x != 0 { x - (width >> 1) } else { x };
    let oy = if anchor_y != 0 { y - (height >> 1) } else { y };
    (ox, oy)
}

#[inline]
fn map_spr_blend_mode(blend_mode: u16, blend_param: u16, alpha_or_mode: u16) -> (u32, i32) {
    match blend_mode {
        1 => (2, i32::from(blend_param)),
        2 => (0x0E, i32::from(blend_param)),
        3 => (0x68, i32::from(blend_param)),
        _ if alpha_or_mode != 0 => (2, i32::from(alpha_or_mode)),
        _ => (0, 0),
    }
}

struct TboxGeometry {
    lwg: Option<Arc<LwgImage>>,
    text_rect: (i32, i32, i32, i32),
    wait_xy: (i32, i32),
}

impl EngineResources {
    fn load_surface(&mut self, cat: AssetCategory, name: &str) -> Result<Arc<Surface>> {
        let key = (cat, name.to_ascii_lowercase());
        if let Some(s) = self.surface_cache.get(&key) {
            return Ok(s.clone());
        }
        let resolved = self.vfs.resolve_with_extensions(cat, name, &["wcg", "bmp"])?;
        let surf = if resolved.bytes.starts_with(b"WG") {
            decode_wcg(&resolved.bytes)?
        } else {
            decode_bmp(&resolved.bytes)?
        };
        let arc = Arc::new(surf);
        self.surface_cache.insert(key, arc.clone());
        Ok(arc)
    }

    fn load_lwg(&mut self, name: &str) -> Option<Arc<LwgImage>> {
        let key = name.to_ascii_lowercase();
        if let Some(img) = self.lwg_cache.get(&key) {
            return Some(img.clone());
        }
        let bytes = self.vfs.read(AssetCategory::Grps, name).ok()?;
        let img = Arc::new(decode_lwg(&bytes).ok()?);
        self.lwg_cache.insert(key, img.clone());
        Some(img)
    }

    fn load_audio(&mut self, cat: AssetCategory, name: &str) -> Result<Arc<DecodedAudio>> {
        let key = (cat, name.to_ascii_lowercase());
        if let Some(a) = self.audio_cache.get(&key) {
            return Ok(a.clone());
        }
        let raw = self.vfs.read(cat, name)?;
        let decoded = Arc::new(decode_vorbis_acm_wav(&raw)?);
        self.audio_cache.insert(key, decoded.clone());
        Ok(decoded)
    }

    fn sync_mixer_from_config(&self) {
        if let Ok(mut mixer) = self.mixer.lock() {
            let cfg = &self.global_save.config;
            mixer.set_bgm_enabled(cfg.bgm_enable());
            mixer.set_bgm_volume(cfg.bgm_volume());
            mixer.set_se_enabled(cfg.se_enable());
            mixer.set_se_volume(cfg.se_volume());
            mixer.set_voice_enabled(cfg.voice_enable());
            mixer.set_voice_volume(cfg.voice_volume());
        }
    }

    fn resolve_tbox_geometry(&mut self, state: &VmState) -> TboxGeometry {
        let skin_name = format!("tbox{:02}.lwg", state.tbox.skin_id);
        let lwg = self
            .load_lwg(&skin_name)
            .or_else(|| self.load_lwg("tbox00.lwg"));
        let text_rect = if state.tbox.rect_custom {
            (
                i32::from(state.tbox.rect.0),
                i32::from(state.tbox.rect.1),
                i32::from(state.tbox.rect.2).max(1),
                i32::from(state.tbox.rect.3).max(1),
            )
        } else if let Some(tl) = lwg.as_ref().and_then(|img| img.find_layer("text")) {
            (
                tl.x,
                tl.y,
                tl.surface.width() as i32,
                tl.surface.height() as i32,
            )
        } else {
            TBOX00_TEXT_RECT
        };
        let wait_xy = if state.tbox.icon_pos_custom {
            (i32::from(state.tbox.icon_pos.0), i32::from(state.tbox.icon_pos.1))
        } else if let Some(wl) = lwg.as_ref().and_then(|img| img.find_layer("wait")) {
            (wl.x, wl.y)
        } else {
            (i32::from(state.tbox.icon_pos.0), i32::from(state.tbox.icon_pos.1))
        };
        TboxGeometry {
            lwg,
            text_rect,
            wait_xy,
        }
    }

    fn compose_layers_to_new_scene(&mut self, state: &VmState) -> Result<()> {
        filter_fill_argb(&mut self.canvas.new_scene, 0x0000_0000);

        #[derive(Clone, Copy)]
        enum DrawPass {
            Layer(usize),
            Eyes,
            Filter,
            Gauge(usize),
        }
        let mut items: Vec<(i16, u8, usize, DrawPass)> = Vec::with_capacity(state.layers.len() + 12);
        for (idx, l) in state.layers.iter().enumerate() {
            if l.visible {
                items.push((l.z_order, 0, idx, DrawPass::Layer(idx)));
            }
        }
        if state.eyes_overlay.es_id != 0 {
            items.push((state.eyes_overlay.z_order, 1, 0, DrawPass::Eyes));
        }
        if state.filter_overlay.alpha_pct != 0 {
            items.push((state.filter_overlay.z_order, 2, 0, DrawPass::Filter));
        }
        for (gi, g) in state.gauges.iter().enumerate() {
            if g.visible && g.num_id > 0 {
                items.push((100, 3, gi, DrawPass::Gauge(gi)));
            }
        }
        items.sort_by_key(|&(z, pass_rank, sub_idx, _)| (z, pass_rank, sub_idx));

        for &(_, _, _, pass) in &items {
            match pass {
                DrawPass::Layer(idx) => {
                    let l = &state.layers[idx];
                    if l.source_kind == 3 {
                        if idx == 0 {
                            filter_fill_argb(
                                &mut self.canvas.new_scene,
                                l.fill_color & 0x00FF_FFFF,
                            );
                        }
                        continue;
                    }
                    if l.source_kind == 2 {
                        if !l.text_or_path.is_empty() {
                            if let Some(fonts) = self.fonts.as_ref() {
                                let style = TextStyle {
                                    font_family: FontFamily::Gothic,
                                    base_height: l.extra_50.max(8),
                                    color_rgb: 0x00FF_FFFF,
                                    raster_mode: RasterMode::Mode3,
                                    ..TextStyle::default()
                                };
                                let parsed = parse_caret_bytes(
                                    &l.text_or_path,
                                    state.encoding,
                                    style,
                                    0,
                                    Some(&state.vars),
                                    &state.tbox.custom_palette,
                                );
                                if let Ok(layout) = layout_text(
                                    &parsed,
                                    fonts,
                                    Some(&self.vfs),
                                    &mut self.gaiji_bank,
                                    GAME_WIDTH as i32,
                                    GAME_HEIGHT as i32,
                                    false,
                                ) {
                                    if !layout.items.is_empty() {
                                        let mut min_x = i32::MAX;
                                        let mut min_y = i32::MAX;
                                        let mut max_x = i32::MIN;
                                        let mut max_y = i32::MIN;
                                        for it in &layout.items {
                                            min_x = min_x.min(it.x);
                                            min_y = min_y.min(it.y);
                                            max_x = max_x.max(it.x + it.surface.width() as i32);
                                            max_y = max_y.max(it.y + it.surface.height() as i32);
                                        }
                                        let bw = (max_x - min_x).max(0);
                                        let bh = (max_y - min_y).max(0);
                                        let (ax, ay) = anchored_top_left(
                                            l.x, l.y, l.anchor_x, l.anchor_y, bw, bh,
                                        );
                                        render_layout_to_surface(
                                            &mut self.canvas.new_scene,
                                            &layout,
                                            None,
                                            (ax - min_x, ay - min_y),
                                        );
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    let hovered = l.anim_active && self.hovered_vm_layer == Some(idx);
                    if l.source_kind == 0
                        && (l.img_id != 0 || (hovered && l.subframe_img_ids[0] != 0))
                    {
                        let cat = dir_prefix_to_category(&l.dir_prefix);
                        let (img_id, px, py) = if hovered && l.subframe_img_ids[0] != 0 {
                            (
                                l.subframe_img_ids[0],
                                l.subframe_coords[0].0,
                                l.subframe_coords[0].1,
                            )
                        } else {
                            (l.img_id, l.x, l.y)
                        };
                        let stem = format!("{img_id:04}");
                        if let Ok(surf) = self.load_surface(cat, &stem) {
                            let (ax, ay) = if idx == 0 {
                                (px, py)
                            } else {
                                anchored_top_left(
                                    px,
                                    py,
                                    l.anchor_x,
                                    l.anchor_y,
                                    surf.width() as i32,
                                    surf.height() as i32,
                                )
                            };
                            let (mapped_bm, mapped_ga) =
                                map_spr_blend_mode(l.blend_mode, l.blend_param, l.alpha_or_mode);
                            let params = if idx == 0 {
                                BlitParams {
                                    blend_mode: 0,
                                    alpha_channel_mode: 6,
                                    ..Default::default()
                                }
                            } else if hovered {
                                let hover_mode = match l.anim_x[1] {
                                    1 => 0x0D,
                                    2 => 0x04,
                                    3 => 0x17,
                                    _ => mapped_bm,
                                };
                                let hover_alpha = if l.anim_x[1] == 2 {
                                    0x40
                                } else {
                                    mapped_ga
                                };
                                BlitParams {
                                    blend_mode: hover_mode,
                                    alpha_channel_mode: 0,
                                    global_alpha: hover_alpha,
                                    ..Default::default()
                                }
                            } else {
                                BlitParams {
                                    blend_mode: mapped_bm,
                                    alpha_channel_mode: 0,
                                    global_alpha: mapped_ga,
                                    ..Default::default()
                                }
                            };
                            let clip = Rect::new(
                                i32::from(l.clip_rect[0]),
                                i32::from(l.clip_rect[1]),
                                i32::from(l.clip_rect[2]),
                                i32::from(l.clip_rect[3]),
                            );
                            let clip_opt = if clip.right > clip.left && clip.bottom > clip.top {
                                Some(clip)
                            } else {
                                None
                            };
                            self.canvas
                                .compose_to_new_scene(&surf, &params, (ax, ay), clip_opt);
                        }
                    }
                }
                DrawPass::Eyes => {
                    let name = format!("es{:03}", state.eyes_overlay.es_id);
                    if let Ok(surf) = self.load_surface(AssetCategory::Grps, &name) {
                        let mode = if state.eyes_overlay.blend_flag == 1 {
                            0x67
                        } else {
                            0x68
                        };
                        let params = BlitParams {
                            blend_mode: mode,
                            ..Default::default()
                        };
                        self.canvas
                            .compose_to_new_scene(&surf, &params, (0, 0), None);
                    }
                }
                DrawPass::Filter => {
                    if let Ok(white) = Surface::filled(GAME_WIDTH, GAME_HEIGHT, 0x00FF_FFFF) {
                        let ga = ((u32::from(state.filter_overlay.alpha_pct) * 255) / 100) as i32;
                        let mode = if state.filter_overlay.mode_flag == 1 {
                            0x6D
                        } else {
                            0x6C
                        };
                        let params = BlitParams {
                            blend_mode: mode,
                            global_alpha: ga,
                            ..Default::default()
                        };
                        self.canvas
                            .compose_to_new_scene(&white, &params, (0, 0), None);
                    }
                }
                DrawPass::Gauge(gi) => {
                    let g = &state.gauges[gi];
                    let num_name = format!("num{:02}", g.num_id);
                    if let Ok(surf) = self.load_surface(AssetCategory::Grps, &num_name) {
                        let gw = surf.width();
                        let digit_h = surf.height() / 10;
                        if gw > 0 && digit_h > 0 {
                            let clamped = i32::from(g.value)
                                .clamp(i32::from(g.min), i32::from(g.max))
                                .max(0);
                            let digit = (clamped % 10) as u32;
                            let row_bytes = (gw as usize) * 4;
                            let start = (digit as usize) * (digit_h as usize) * row_bytes;
                            let end = start + (digit_h as usize) * row_bytes;
                            if end <= surf.as_bytes().len() {
                                if let Ok(tile) = Surface::from_bgra(
                                    gw,
                                    digit_h,
                                    surf.as_bytes()[start..end].to_vec(),
                                ) {
                                    let params = BlitParams {
                                        blend_mode: 0,
                                        alpha_channel_mode: 0,
                                        ..Default::default()
                                    };
                                    self.canvas.compose_to_new_scene(
                                        &tile,
                                        &params,
                                        (
                                            i32::from(g.x) + i32::from(g.num_x),
                                            i32::from(g.y) + i32::from(g.num_y),
                                        ),
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Textbox + portrait face (`0x30`) + active text + wait cursor overlay
        let has_tbox_content = !state.tbox.current_text_raw.is_empty() || state.tbox.face_id > 0;
        if state.tbox.visible && !self.textbox_hidden_by_user && has_tbox_content {
            let TboxGeometry {
                lwg,
                text_rect,
                wait_xy,
            } = self.resolve_tbox_geometry(state);
            let tx = i32::from(state.tbox.pos.0);
            let ty = i32::from(state.tbox.pos.1);

            let face_surf = if state.tbox.face_id > 0 {
                self.load_surface(AssetCategory::Grpf, &format!("{:04}", state.tbox.face_id))
                    .ok()
            } else {
                None
            };
            let face_pos = (
                i32::from(state.tbox.face_pos.0),
                i32::from(state.tbox.face_pos.1),
            );
            let face_params = BlitParams {
                blend_mode: 0,
                alpha_channel_mode: 0,
                ..Default::default()
            };

            if state.tbox.face_z == 0 {
                if let Some(fs) = &face_surf {
                    self.canvas
                        .compose_to_new_scene(fs, &face_params, face_pos, None);
                }
            }

            if let Some(lwg) = &lwg {
                for layer in &lwg.layers {
                    if layer.name.eq_ignore_ascii_case("back") {
                        let params = BlitParams {
                            blend_mode: 2,
                            alpha_channel_mode: 0,
                            global_alpha: i32::from(state.tbox.bg_alpha),
                            ..Default::default()
                        };
                        self.canvas.compose_to_new_scene(
                            &layer.surface,
                            &params,
                            (tx + layer.x, ty + layer.y),
                            None,
                        );
                    } else if layer.name.eq_ignore_ascii_case("front") {
                        let params = BlitParams {
                            blend_mode: 0,
                            alpha_channel_mode: 0,
                            ..Default::default()
                        };
                        self.canvas.compose_to_new_scene(
                            &layer.surface,
                            &params,
                            (tx + layer.x, ty + layer.y),
                            None,
                        );
                    }
                }
            }

            if state.tbox.face_z != 0 {
                if let Some(fs) = &face_surf {
                    self.canvas
                        .compose_to_new_scene(fs, &face_params, face_pos, None);
                }
            }

            if let Some(active) = &self.active_text {
                render_layout_to_surface(
                    &mut self.canvas.new_scene,
                    &active.layout,
                    Some(active.typewriter.revealed_count()),
                    (tx + text_rect.0, ty + text_rect.1),
                );

                // Animated click-wait cursor (`wait%02d.lwg` at `tbox.pos + wait_xy + wl.xy`)
                if self.show_wait_cursor && active.typewriter.is_finished() {
                    let wait_name = format!("wait{:02}.lwg", state.tbox.wait_id);
                    if let Some(wait_lwg) = self
                        .load_lwg(&wait_name)
                        .or_else(|| self.load_lwg("wait00.lwg"))
                    {
                        let blink_on = (self.wait_cursor_ms / 400).is_multiple_of(2);
                        let layer_name = if blink_on { "grow" } else { "body" };
                        if let Some(wl) = wait_lwg.find_layer(layer_name) {
                            let wx = tx + wait_xy.0 + wl.x;
                            let wy = ty + wait_xy.1 + wl.y;
                            let params = BlitParams {
                                blend_mode: 0,
                                alpha_channel_mode: 0,
                                ..Default::default()
                            };
                            self.canvas
                                .compose_to_new_scene(&wl.surface, &params, (wx, wy), None);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl VmHost for EngineResources {
    fn load_script(&mut self, script_id: u16) -> Result<GscScript> {
        let name = format!("{script_id:04}.gsc");
        let bytes = self.vfs.read(AssetCategory::Scr, &name)?;
        let allow_quirks = self.language_mode == LanguageMode::ChineseV1;
        let mut script = GscScript::parse(script_id, &bytes, allow_quirks)?;
        if self.repair_chs_recall_labels && script.chs_zeroed_labels {
            if let Some(orig_arc) = &self.orig_scr_backup {
                if let Ok(orig_bytes) = orig_arc.read(&name) {
                    if let Ok(orig_script) = GscScript::parse(script_id, &orig_bytes, false) {
                        script.repair_labels_from(&orig_script);
                    }
                }
            }
        }
        Ok(script)
    }

    fn refresh_scene(&mut self, state: &VmState) -> Result<()> {
        self.compose_layers_to_new_scene(state)?;
        if self.transition.is_none() {
            self.canvas.commit_new_scene();
        }
        Ok(())
    }

    fn start_transition(
        &mut self,
        state: &VmState,
        effect_id: u16,
        steps: u16,
        delay_ms: u16,
    ) -> Result<()> {
        self.compose_layers_to_new_scene(state)?;
        if effect_id == 0 || !self.global_save.config.effect_enable() {
            self.transition = None;
            self.canvas.commit_new_scene();
            return Ok(());
        }
        let mut runner = TransitionRunner::with_vfs(effect_id, steps, delay_ms, &self.vfs)?;
        let first_delay = runner.step(&mut self.canvas).unwrap_or(0);
        if runner.is_done() {
            self.transition = None;
        } else {
            self.transition = Some(ActiveTransition {
                runner,
                accum_ms: 0,
                next_delay_ms: first_delay,
            });
        }
        Ok(())
    }

    fn display_text(&mut self, state: &VmState, _slot: u16, raw_expanded: &[u8]) -> Result<()> {
        let TboxGeometry { text_rect, .. } = self.resolve_tbox_geometry(state);
        let Some(fonts) = self.fonts.as_ref() else {
            self.active_text = None;
            self.refresh_scene(state)?;
            return Ok(());
        };
        let align = match state.tbox.text_align {
            1 => TextAlign::Center,
            2 => TextAlign::Right,
            _ => TextAlign::Left,
        };
        let initial_style = TextStyle {
            font_family: if state.tbox.font_family_id == 1 {
                FontFamily::Mincho
            } else {
                FontFamily::Gothic
            },
            base_height: state.tbox.font_size.max(8),
            color_rgb: state.tbox.color_rgb,
            align,
            vertical: state.tbox.layout_mode != 0,
            ..TextStyle::default()
        };
        let speed = if state.fast_skip_active {
            0
        } else {
            self.global_save.config.text_speed()
        };
        let parsed = parse_caret_bytes(
            raw_expanded,
            state.encoding,
            initial_style,
            speed,
            Some(&state.vars),
            &state.tbox.custom_palette,
        );
        let layout = layout_text_with_indent(
            &parsed,
            fonts,
            Some(&self.vfs),
            &mut self.gaiji_bank,
            text_rect.2,
            text_rect.3,
            state.tbox.layout_mode != 0,
            i32::from(state.tbox.indent_first),
            i32::from(state.tbox.indent_cont),
        )?;
        let mut typewriter = TypewriterState::from_layout(&layout);
        if speed == 0 {
            typewriter.reveal_all();
        } else {
            typewriter.tick_50ms();
        }
        self.active_text = Some(ActiveTextRender {
            layout,
            typewriter,
            accum_ms: 0,
        });
        self.refresh_scene(state)?;
        Ok(())
    }

    fn play_bgm(&mut self, bgm_id: u16, loop_flag: bool, fade_ms: u16) {
        if bgm_id == 0 {
            self.stop_bgm(fade_ms);
            return;
        }
        let name = format!("Track{bgm_id:02}.wav");
        if let Ok(audio) = self.load_audio(AssetCategory::Bgm, &name) {
            let steps = if fade_ms == 0 {
                Some(0)
            } else {
                Some((u32::from(fade_ms) / 100).max(1))
            };
            if let Ok(mut mixer) = self.mixer.lock() {
                mixer.play_bgm(bgm_id, audio, loop_flag, fade_ms > 0, steps);
            }
        }
    }

    fn stop_bgm(&mut self, fade_ms: u16) {
        let steps = if fade_ms == 0 {
            Some(0)
        } else {
            Some((u32::from(fade_ms) / 100).max(1))
        };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.stop_bgm(steps);
        }
    }

    fn load_se(&mut self, se_id: u16) {
        if se_id == 0 {
            self.last_loaded_se = None;
            return;
        }
        let name = format!("{se_id:04}.wav");
        self.last_loaded_se = self.load_audio(AssetCategory::Wav, &name).ok();
    }

    fn play_se(&mut self, loop_cnt: i16, fade_flag: u16, fade_50ms: i16) {
        let Some(audio) = self.last_loaded_se.clone() else {
            return;
        };
        let loop_mode = LoopMode::from_script_loop_cnt(i32::from(loop_cnt));
        let fade_steps = if fade_flag != 0 && fade_50ms > 0 {
            Some(((fade_50ms as u32 * 50) / 100).max(1))
        } else {
            Some(0)
        };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.play_se0(audio, loop_mode, fade_steps);
        }
    }

    fn stop_se(&mut self, fade_ms: u16) {
        let steps = if fade_ms == 0 {
            Some(0)
        } else {
            Some((u32::from(fade_ms) / 100).max(1))
        };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.stop_se0(steps);
        }
    }

    fn play_voice(&mut self, voice_id: u16, loop_cnt: i16, _fade_flag: u16, delay_50ms: i16) {
        if voice_id == 0 {
            return;
        }
        let sub = voice_id / 10000;
        let rem = voice_id % 10000;
        let name = if sub == 0 {
            format!("{rem:04}.wav")
        } else {
            format!("{sub}/{rem:04}.wav")
        };
        let audio = self
            .load_audio(AssetCategory::Voice, &name)
            .or_else(|_| self.load_audio(AssetCategory::Voice, &format!("{voice_id:04}.wav")));
        if let Ok(audio) = audio {
            let loop_flag = loop_cnt == 999 || loop_cnt < 0;
            let delay = delay_50ms.max(0) as u32;
            if let Ok(mut mixer) = self.mixer.lock() {
                mixer.play_voice(audio, loop_flag, delay, Some(0));
            }
        }
    }

    fn stop_voice(&mut self, fade_ms: u16) {
        let steps = if fade_ms == 0 {
            Some(0)
        } else {
            Some((u32::from(fade_ms) / 100).max(1))
        };
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.stop_voice(steps);
        }
    }

    fn start_video(&mut self, mpg_id: u16) {
        let name = format!("{mpg_id:04}.mpg");
        if let Ok(bytes) = self.vfs.read(AssetCategory::Mov, &name) {
            self.mpeg.start_bytes(bytes);
        }
    }

    fn is_audio_playing(&self, is_voice: bool) -> bool {
        if let Ok(mixer) = self.mixer.lock() {
            if is_voice {
                mixer.voice().is_playing()
            } else {
                mixer.se0().is_playing()
            }
        } else {
            false
        }
    }

    fn voice_cut_on_click(&self) -> bool {
        self.global_save.config.voice_cut_on_click()
    }

    fn skip_unread(&self) -> bool {
        self.global_save.config.skip_unread()
    }

    fn on_persistent_state_changed(&mut self, state: &VmState) {
        state.save_to_global_save(&mut self.global_save);
    }
}

/// Complete headless + FFI-ready Fujihime engine instance.
pub struct Engine {
    res: EngineResources,
    pub vm: VmRuntime,
    scene: TopScene,
    title_movie_stage: TitleMovieStage,
    last_yield: Option<VmYield>,
    pending_sleep_ms: u32,
    pending_select: Option<ChoicePrompt>,
    pending_dialogs: VecDeque<PendingDialog>,
    next_dialog_id: u64,
    exit_requested: bool,
    // Phase 3 UI scenes & overlays
    pub title_state: TitleSceneState,
    pub compane: CompaneWidget,
    pub conf_scrn: ConfScrnOverlay,
    pub save_scrn: SaveScrnOverlay,
    pub select_6_ui: Select6Overlay,
    pub selmap: SelMapOverlay,
    pub backlog: BacklogOverlay,
    pub cg_gallery: CgGalleryState,
    pub scene_replay: SceneReplayState,
    pub music_mode: MusicModeState,
    pub extra_menu: ExtraMenuState,
    pub auto_mode: bool,
    auto_accum_ms: u32,
}


// --- fix_save_menu_freeze: real save timestamp (was hardcoded 2026-11-20 00:00) ---
static SYNCED_SAVE_TIME: Mutex<(u16, u16, u16, u16, u16)> = Mutex::new((0, 0, 0, 0, 0));
pub fn set_synced_save_time(y: u16, mo: u16, d: u16, h: u16, mi: u16) {
    if let Ok(mut t) = SYNCED_SAVE_TIME.lock() { *t = (y, mo, d, h, mi); }
}
fn system_now_ymdhm() -> (u16, u16, u16, u16, u16) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let hour = (sod / 3600) as u16;
    let minute = ((sod % 3600) / 60) as u16;
    // Howard Hinnant civil_from_days, no chrono needed
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 { y += 1; }
    (y as u16, m as u16, d as u16, hour, minute)
}
pub fn current_save_time() -> (u16, u16, u16, u16, u16) {
    if let Ok(t) = SYNCED_SAVE_TIME.lock() { if t.0 != 0 { return *t; } }
    system_now_ymdhm()
}

impl Engine {
    /// Construct and initialize the engine from [`EngineConfig`].
    pub fn create(config: EngineConfig) -> Result<Self> {
        let vfs = Self::open_vfs(
            &config.game_root,
            config.language_mode,
            config.override_dir.as_deref(),
            config.orig_backup_dir.as_deref(),
        )?;
        let orig_scr_backup = Self::open_orig_scr_backup(
            &config.game_root,
            config.language_mode,
            config.orig_backup_dir.as_deref(),
            config.repair_chs_recall_labels,
        );

        let global_save_path = config.save_dir.join("KhDLsave.dat");
        let global_save = if global_save_path.is_file() {
            GlobalSave2007::load_file(&global_save_path)?
        } else {
            GlobalSave2007::default()
        };

        let mut mixer = AudioMixer::new();
        mixer.set_bgm_enabled(global_save.config.bgm_enable());
        mixer.set_bgm_volume(global_save.config.bgm_volume());
        mixer.set_se_enabled(global_save.config.se_enable());
        mixer.set_se_volume(global_save.config.se_volume());
        mixer.set_voice_enabled(global_save.config.voice_enable());
        mixer.set_voice_volume(global_save.config.voice_volume());

        let mut gaiji_bank = GaijiBank::new();
        let _ = gaiji_bank.preload_all_standard(&vfs);

        let encoding = match config.language_mode {
            LanguageMode::Japanese => TextEncoding::Cp932,
            LanguageMode::ChineseV1 => TextEncoding::Gbk,
        };
        let mut vm = VmRuntime::new(encoding);
        vm.state.load_from_global_save(&global_save);

        let mut music_mode = MusicModeState::default();
        music_mode.probe_from_vfs(&vfs);

        let last_page = (global_save.config.last_save_page() as usize).min(9);

        let res = EngineResources {
            game_root: config.game_root,
            override_dir: config.override_dir,
            orig_backup_dir: config.orig_backup_dir,
            vfs,
            orig_scr_backup,
            repair_chs_recall_labels: config.repair_chs_recall_labels,
            language_mode: config.language_mode,
            save_dir: config.save_dir,
            global_save,
            canvas: Canvas800x600::new(),
            font_lib: FontLibrary::new()?,
            fonts: None,
            pending_mincho: None,
            gaiji_bank,
            lwg_cache: HashMap::new(),
            surface_cache: HashMap::new(),
            audio_cache: HashMap::new(),
            last_loaded_se: None,
            mixer: Arc::new(Mutex::new(mixer)),
            video_audio_fifo: Arc::new(Mutex::new(VecDeque::new())),
            mpeg: MpegWindowController::new(),
            transition: None,
            active_text: None,
            textbox_hidden_by_user: false,
            hovered_vm_layer: None,
            show_wait_cursor: false,
            wait_cursor_ms: 0,
            last_error: None,
        };

        Ok(Self {
            res,
            vm,
            scene: TopScene::Title,
            title_movie_stage: TitleMovieStage::None,
            last_yield: None,
            pending_sleep_ms: 0,
            pending_select: None,
            pending_dialogs: VecDeque::new(),
            next_dialog_id: 1,
            exit_requested: false,
            title_state: TitleSceneState::default(),
            compane: CompaneWidget::default(),
            conf_scrn: ConfScrnOverlay::default(),
            save_scrn: SaveScrnOverlay {
                page: last_page,
                ..SaveScrnOverlay::default()
            },
            select_6_ui: Select6Overlay::default(),
            selmap: SelMapOverlay::default(),
            backlog: BacklogOverlay::default(),
            cg_gallery: CgGalleryState::default(),
            scene_replay: SceneReplayState::default(),
            music_mode,
            extra_menu: ExtraMenuState::default(),
            auto_mode: false,
            auto_accum_ms: 0,
        })
    }

    fn open_vfs(
        game_root: &Path,
        language_mode: LanguageMode,
        override_dir: Option<&Path>,
        orig_backup_dir: Option<&Path>,
    ) -> Result<AssetVfs> {
        let mut vfs_cfg = VfsConfig::new(game_root, language_mode);
        if let Some(ovr) = override_dir {
            vfs_cfg = vfs_cfg.with_override_dir(ovr);
        }
        if let Some(bak) = orig_backup_dir {
            vfs_cfg = vfs_cfg.with_orig_backup_dir(bak);
        }
        AssetVfs::open(vfs_cfg)
    }

    fn open_orig_scr_backup(
        game_root: &Path,
        language_mode: LanguageMode,
        orig_backup_dir: Option<&Path>,
        repair_chs_recall_labels: bool,
    ) -> Option<XflArchive> {
        if repair_chs_recall_labels && language_mode == LanguageMode::ChineseV1 {
            let candidate = orig_backup_dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| game_root.join("补丁").join("原版备份"))
                .join("scr.xfl");
            if candidate.is_file() {
                return XflArchive::open(candidate).ok();
            }
        }
        None
    }

    /// Switch active language/patch mode (`Japanese` vs `ChineseV1`) at runtime.
    pub fn set_language_mode(
        &mut self,
        language_mode: LanguageMode,
        orig_backup_dir: Option<PathBuf>,
        repair_chs_recall_labels: bool,
    ) -> Result<()> {
        if let Some(bak) = orig_backup_dir {
            self.res.orig_backup_dir = Some(bak);
        }
        self.res.repair_chs_recall_labels = repair_chs_recall_labels;
        let vfs = Self::open_vfs(
            &self.res.game_root,
            language_mode,
            self.res.override_dir.as_deref(),
            self.res.orig_backup_dir.as_deref(),
        )?;
        let orig_scr_backup = Self::open_orig_scr_backup(
            &self.res.game_root,
            language_mode,
            self.res.orig_backup_dir.as_deref(),
            repair_chs_recall_labels,
        );
        self.res.vfs = vfs;
        self.res.orig_scr_backup = orig_scr_backup;
        self.res.language_mode = language_mode;
        self.res.surface_cache.clear();
        self.res.lwg_cache.clear();
        self.vm.state.encoding = match language_mode {
            LanguageMode::Japanese => TextEncoding::Cp932,
            LanguageMode::ChineseV1 => TextEncoding::Gbk,
        };
        if self.vm.current_script.is_some() {
            if self.scene == TopScene::Title && self.title_state.script_driven {
                self.enter_title_screen()?;
                self.music_mode.probe_from_vfs(&self.res.vfs);
                return Ok(());
            }
            let _ = self.vm.sync_loaded_script_and_host(&mut self.res);
        }
        self.music_mode.probe_from_vfs(&self.res.vfs);
        self.compose_ui_and_commit()?;
        Ok(())
    }

    /// Import a 206,024-byte 2002 CD `KhDLsave.dat` (e.g. `补丁/全CG档/KhDLsave.dat`),
    /// migrate to 2007 DL `GlobalSave2007`, apply to live VM, and persist `KhDLsave.dat`.
    pub fn import_2002_cd_save(&mut self, bytes: &[u8]) -> Result<()> {
        let cd = GlobalSave2002Cd::from_bytes(bytes)?;
        let migrated = cd.to_2007_dl();
        // Preserve active audio/speed preferences while adopting global flags/CG/replay vars & kidoku.
        let preserved_cfg = self.res.global_save.config.clone();
        self.res.global_save = migrated;
        self.res.global_save.config = preserved_cfg;
        self.vm.state.load_from_global_save(&self.res.global_save);
        self.save_global()?;
        self.compose_ui_and_commit()?;
        Ok(())
    }

    /// Access immutable reference to `GlobalSave2007` (`ConfigBlock2007` + global vars).
    pub fn global_save(&self) -> &GlobalSave2007 {
        &self.res.global_save
    }

    /// Access mutable reference to `ConfigBlock2007` and sync live audio/text state.
    pub fn update_config<F: FnOnce(&mut ConfigBlock2007)>(&mut self, f: F) {
        f(&mut self.res.global_save.config);
        self.res.sync_mixer_from_config();
        self.vm.state.tbox.speed = self.res.global_save.config.text_speed();
    }

    /// Inject a TTF/TTC font from memory (`AppBundle/Fonts/`).
    pub fn load_font_bytes(&mut self, role: FontRole, bytes: Vec<u8>, face_index: i32) -> Result<()> {
        let face = self.res.font_lib.load_face_from_bytes(bytes, face_index)?;
        match role {
            FontRole::Gothic => {
                if let Some(fs) = &mut self.res.fonts {
                    fs.gothic = face;
                } else {
                    self.res.fonts = Some(FontSet::new(face, self.res.pending_mincho.take()));
                }
            }
            FontRole::Mincho => {
                if let Some(fs) = &mut self.res.fonts {
                    fs.mincho = Some(face);
                } else {
                    self.res.pending_mincho = Some(face);
                }
            }
            FontRole::Fallback => {
                if let Some(fs) = &mut self.res.fonts {
                    if fs.mincho.is_none() {
                        fs.mincho = Some(face);
                    }
                } else {
                    self.res.fonts = Some(FontSet::new(face, self.res.pending_mincho.take()));
                }
            }
        }
        Ok(())
    }

    /// Start the original Title OP movie sequence (`0002.mpg` -> `0001.mpg` -> Title, `0x426ce0`).
    pub fn start_title_sequence(&mut self) -> Result<()> {
        self.scene = TopScene::Title;
        self.close_all_modals();
        if let Ok(b) = self.res.vfs.read(AssetCategory::Mov, "0002.mpg") {
            self.title_movie_stage = TitleMovieStage::Playing0002;
            self.res.mpeg.start_bytes(b);
        } else {
            self.enter_title_screen()?;
        }
        Ok(())
    }

    /// Enter Title screen (`0101.gsc` 2007 DL script title if present, or `top_bg` fallback).
    pub fn enter_title_screen(&mut self) -> Result<()> {
        let _ = self.res.mpeg.handle_event(MpegWindowEvent::InternalStop);
        self.scene = TopScene::Title;
        self.title_movie_stage = TitleMovieStage::None;
        self.close_all_modals();
        self.auto_mode = false;
        self.vm.state.fast_skip_active = false;
        self.res.active_text = None;
        self.res.show_wait_cursor = false;
        self.vm.state.tbox.visible = false;
        self.vm.state.tbox.current_text_raw.clear();
        self.vm.state.tbox.face_id = 0;
        self.vm.state.eyes_overlay = Default::default();
        self.vm.state.filter_overlay = Default::default();
        self.compane = CompaneWidget::default();

        if self.res.vfs.exists(AssetCategory::Scr, "0101.gsc")
            && !self.res.vfs.exists(AssetCategory::Grps, "top_bg.wcg")
        {
            if !(1..=4).contains(&self.vm.state.vars[7000]) {
                self.vm.state.vars[7000] = 1;
            }
            self.title_state.script_driven = true;
            self.vm.start_script(&mut self.res, 101, 0)?;
            self.advance_vm_until_yield()?;
            self.compose_ui_and_commit()?;
        } else {
            self.enter_static_title_screen()?;
        }
        Ok(())
    }

    /// Enter EXE-layout static Title screen (`top_bg` / `grpo_tp` + `top_me01..07`).
    pub fn enter_static_title_screen(&mut self) -> Result<()> {
        let _ = self.res.mpeg.handle_event(MpegWindowEvent::InternalStop);
        self.scene = TopScene::Title;
        self.title_movie_stage = TitleMovieStage::None;
        self.title_state.script_driven = false;
        self.close_all_modals();
        self.compose_ui_and_commit()
    }

    /// Navigate directly to one of the 9 `TopScene`s (`0x41d010`).
    pub fn enter_scene(&mut self, target: TopScene) -> Result<()> {
        match target {
            TopScene::Title => self.enter_title_screen()?,
            TopScene::MainGame => {
                self.scene = TopScene::MainGame;
                self.close_all_modals();
                self.compose_ui_and_commit()?;
            }
            TopScene::LoadMenu => {
                self.scene = TopScene::LoadMenu;
                self.conf_scrn.active = false;
                self.backlog.active = false;
                self.save_scrn.active = true;
                self.save_scrn.mode = SaveScrnMode::Load;
                self.save_scrn.refresh_headers(&self.res.save_dir);
                self.compose_ui_and_commit()?;
            }
            TopScene::ConfigMenu => {
                self.scene = TopScene::ConfigMenu;
                self.save_scrn.active = false;
                self.backlog.active = false;
                self.conf_scrn.active = true;
                self.conf_scrn.return_to_main_game = false;
                self.compose_ui_and_commit()?;
            }
            TopScene::CgGallery => {
                self.scene = TopScene::CgGallery;
                self.close_all_modals();
                self.cg_gallery.fullscreen = None;
                self.compose_ui_and_commit()?;
            }
            TopScene::SceneReplay => {
                self.scene = TopScene::SceneReplay;
                self.close_all_modals();
                self.vm.state.recall_flag = false;
                self.compose_ui_and_commit()?;
            }
            TopScene::MusicMode => {
                self.scene = TopScene::MusicMode;
                self.close_all_modals();
                self.music_mode.probe_from_vfs(&self.res.vfs);
                self.compose_ui_and_commit()?;
            }
            TopScene::ExtraMenu => {
                self.scene = TopScene::ExtraMenu;
                self.close_all_modals();
                self.extra_menu.omake_missing_9000 = false;
                self.compose_ui_and_commit()?;
            }
            TopScene::SubsceneDll => {
                self.scene = TopScene::SubsceneDll;
                self.close_all_modals();
                if let Ok(mut m) = self.res.mixer.lock() {
                    m.stop_all();
                }
                self.compose_ui_and_commit()?;
            }
        }
        Ok(())
    }

    fn close_all_modals(&mut self) {
        self.conf_scrn.active = false;
        self.save_scrn.active = false;
        self.backlog.active = false;
        self.selmap.active = false;
    }

    /// Enter main ADV gameplay starting at `script_id` (e.g. `111`/`1102` for New Game).
    pub fn start_game_script(&mut self, script_id: u16) -> Result<()> {
        let _ = self.res.mpeg.handle_event(MpegWindowEvent::InternalStop);
        self.scene = TopScene::MainGame;
        self.title_movie_stage = TitleMovieStage::None;
        self.title_state.script_driven = false;
        self.close_all_modals();
        self.auto_mode = false;
        self.vm.state.fast_skip_active = false;
        self.last_yield = None;
        self.pending_sleep_ms = 0;
        self.pending_select = None;
        self.res.active_text = None;
        self.res.show_wait_cursor = false;
        self.res.textbox_hidden_by_user = false;
        self.res.hovered_vm_layer = None;
        self.compane = CompaneWidget::default();

        // Clear any leftover layers from Title/Menu scripts before entering gameplay.
        for (idx, layer) in self.vm.state.layers.iter_mut().enumerate() {
            layer.img_id = 0;
            layer.text_or_path.clear();
            layer.source_kind = 0;
            layer.anim_active = false;
            layer.z_order = idx as i16;
            if idx == 0 {
                layer.dir_prefix = "grpe".to_string();
                layer.anchor_x = 0;
                layer.anchor_y = 0;
                layer.img_id = 999;
                layer.visible = true;
            } else {
                layer.dir_prefix = "grpo".to_string();
                layer.anchor_x = 1;
                layer.anchor_y = 1;
            }
        }
        if script_id >= 1000 {
            // Apply `0111.gsc` baseline textbox/overlay initialization when jumping directly to a scene.
            let tb = &mut self.vm.state.tbox;
            tb.skin_id = 0;
            tb.pos = (0, 456);
            tb.rect_custom = false;
            tb.icon_pos_custom = false;
            tb.font_size = 30;
            tb.color_rgb = 0x00FF_FFFF;
            tb.font_family_id = 0;
            tb.text_align = 0;
            tb.indent_first = 0;
            tb.indent_cont = 0;
            tb.master_enable = true;
            tb.compane_pos = Some((753, 485));
            self.vm.state.filter_overlay.z_order = 10;
        }

        self.vm.start_script(&mut self.res, script_id, 0)?;
        self.advance_vm_until_yield()?;
        self.compose_ui_and_commit()?;
        Ok(())
    }

    /// Launch a `0x46db08` Scene Replay entry `(tab, slot)` or fallback if absent in 2007 DL.
    pub fn launch_replay_table_entry(&mut self, tab: u8, slot: u8) -> Result<bool> {
        let Some(entry) = find_replay_entry(tab, slot) else {
            return Ok(false);
        };
        if self.vm.state.vars.get(entry.flag_var as usize).copied().unwrap_or(0) == 0 {
            return Ok(false);
        }
        let sid = entry.script_id;
        let name = format!("{sid:04}.gsc");
        if !self.res.vfs.exists(AssetCategory::Scr, &name) {
            self.scene_replay.last_missing_legacy_script = Some(sid);
            return Ok(false);
        }
        self.vm.state.vars[..crate::save::LOCAL_VARS_COUNT].fill(0);
        self.vm.state.recall_flag = true;
        self.scene_replay.last_launched_script_id = Some(sid);
        self.scene_replay.last_missing_legacy_script = None;
        self.start_game_script(sid)?;
        Ok(true)
    }

    /// Launch a `0399.gsc` dual-track replay route (`route_id in 8010..=8036`, excluding `8030`).
    /// Works identically in JP & CHS modes (CHS repairs the 9 zeroed-label scripts from `orig_backup_dir`).
    pub fn launch_recall_0399_route(&mut self, route_id: u16) -> Result<bool> {
        if find_recall_0399_route(route_id).is_none() {
            return Ok(false);
        }
        self.vm.state.vars[..crate::save::LOCAL_VARS_COUNT].fill(0);
        self.vm.state.vars[999] = route_id as i16;
        self.vm.state.vars[99] = 2;
        self.vm.state.recall_flag = true;
        self.scene_replay.last_launched_route_id = Some(route_id);
        self.scene = TopScene::MainGame;
        self.title_state.script_driven = false;
        self.close_all_modals();
        self.vm.start_script(&mut self.res, 399, 0)?;
        self.advance_vm_until_yield()?;
        self.compose_ui_and_commit()?;
        Ok(true)
    }

    /// Play a track (`3..=23` in 2007 DL) in `MusicMode`; returns `false` cleanly if missing.
    pub fn music_mode_play_track(&mut self, track_id: u16) -> bool {
        let name = format!("Track{track_id:02}.wav");
        if !self.res.vfs.exists(AssetCategory::Bgm, &name) {
            self.music_mode.last_missing_track_id = Some(track_id);
            return false;
        }
        self.res.play_bgm(track_id, true, 0);
        self.music_mode.current_track_id = Some(track_id);
        self.music_mode.is_playing = true;
        self.music_mode.last_missing_track_id = None;
        true
    }

    /// Stop music playback in `MusicMode`.
    pub fn music_mode_stop(&mut self) {
        self.res.stop_bgm(0);
        self.music_mode.is_playing = false;
    }

    /// Open `confscrn` overlay (either from `Title` or `MainGame`).
    pub fn open_config_overlay(&mut self) -> Result<()> {
        let from_main = self.scene == TopScene::MainGame;
        self.save_scrn.active = false;
        self.backlog.active = false;
        self.conf_scrn.active = true;
        self.conf_scrn.return_to_main_game = from_main;
        if !from_main {
            self.scene = TopScene::ConfigMenu;
        }
        self.compose_ui_and_commit()
    }

    /// Open `savescrn` overlay in Save or Load mode.
    pub fn open_save_overlay(&mut self, mode: SaveScrnMode) -> Result<()> {
        if mode == SaveScrnMode::Save && self.scene != TopScene::MainGame {
            return Ok(());
        }
        self.conf_scrn.active = false;
        self.backlog.active = false;
        self.save_scrn.active = true;
        self.save_scrn.mode = mode;
        self.save_scrn.refresh_headers(&self.res.save_dir);
        if mode == SaveScrnMode::Load && self.scene == TopScene::Title {
            self.scene = TopScene::LoadMenu;
        }
        self.compose_ui_and_commit()
    }

    /// Open Backlog viewer overlay (`0x414e40`).
    pub fn open_backlog(&mut self) -> Result<()> {
        self.conf_scrn.active = false;
        self.save_scrn.active = false;
        self.backlog.active = true;
        self.backlog.scroll_from_bottom = 0;
        self.compose_ui_and_commit()
    }

    /// Close Backlog viewer overlay.
    pub fn close_backlog(&mut self) -> Result<()> {
        self.backlog.active = false;
        self.compose_ui_and_commit()
    }

    /// Execute any system panel action (`Save`, `Load`, `Config`, `Backlog`, `Skip`,
    /// `Auto`, `HideTextbox`, `Rollback`, `Title`, `Exit`, `Advance`, `ReplayVoice`).
    pub fn perform_system_action(&mut self, action: SystemPanelAction) -> Result<()> {
        match action {
            SystemPanelAction::Save => self.open_save_overlay(SaveScrnMode::Save)?,
            SystemPanelAction::Load => self.open_save_overlay(SaveScrnMode::Load)?,
            SystemPanelAction::Config => self.open_config_overlay()?,
            SystemPanelAction::Backlog => self.open_backlog()?,
            SystemPanelAction::Skip => {
                self.vm.state.fast_skip_active = !self.vm.state.fast_skip_active;
                self.compose_ui_and_commit()?;
            }
            SystemPanelAction::Auto => {
                self.auto_mode = !self.auto_mode;
                self.auto_accum_ms = 0;
                self.compose_ui_and_commit()?;
            }
            SystemPanelAction::HideTextbox => {
                self.res.textbox_hidden_by_user = !self.res.textbox_hidden_by_user;
                self.compose_ui_and_commit()?;
            }
            SystemPanelAction::Rollback => {
                self.trigger_rollback()?;
            }
            SystemPanelAction::Title => {
                self.request_confirm(ConfirmAction::ReturnToTitle);
            }
            SystemPanelAction::Exit => {
                self.request_confirm(ConfirmAction::ExitGame);
            }
            SystemPanelAction::Advance => {
                if matches!(self.last_yield, Some(VmYield::WaitClick { .. })) {
                    self.advance_dialogue_click()?;
                }
            }
            SystemPanelAction::ReplayVoice => {
                let vid = self.vm.state.tbox.current_voice_id;
                if vid != 0 {
                    self.res.play_voice(vid, 0, 0, 0);
                }
            }
        }
        Ok(())
    }

    /// Trigger a `selmap` (`0xD5`) prompt directly or via VM, rendering `selmap01.lwg`.
    pub fn open_map_select(&mut self, mode: i16, param2: i16, unused: i16) -> Result<()> {
        self.selmap.active = true;
        self.selmap.mode = mode;
        self.selmap.param2 = param2;
        self.selmap.unused = unused;
        self.selmap.hovered = None;
        self.selmap.keyboard_focus = 0;
        self.last_yield = Some(VmYield::WaitMapSelect {
            mode,
            param2,
            unused,
        });
        self.compose_ui_and_commit()
    }

    /// Confirm a `selmap` selection (`0..=4`), writing `vars[0] = idx` and resuming VM if running.
    pub fn choose_map_select(&mut self, idx: usize) -> Result<()> {
        if !self.selmap.active || idx >= 5 {
            return Ok(());
        }
        self.selmap.active = false;
        self.selmap.last_selected = Some(idx);
        self.vm.state.vars[0] = idx as i16;
        if matches!(self.last_yield, Some(VmYield::WaitMapSelect { .. })) {
            self.last_yield = None;
            if self.vm.current_script.is_some() {
                self.advance_vm_until_yield()?;
            }
        }
        self.compose_ui_and_commit()
    }

    /// Save global progress `KhDLsave.dat` to `save_dir`.
    pub fn save_global(&mut self) -> Result<()> {
        self.vm.state.save_to_global_save(&mut self.res.global_save);
        let path = self.res.save_dir.join("KhDLsave.dat");
        self.res.global_save.save_file(path)
    }

    /// Save current slot state to `KhDLsave%02d.dat` (`1..=100`).
    pub fn save_slot(
        &mut self,
        slot_num: u16,
        year: u16,
        month: u16,
        day: u16,
        hour: u16,
        minute: u16,
    ) -> Result<()> {
        let slot = self.vm.state.export_slot_save(year, month, day, hour, minute);
        let path = self.res.save_dir.join(format!("KhDLsave{slot_num:02}.dat"));
        slot.save_file(path)?;
        self.res.global_save.config.set_last_save_slot(slot_num);
        self.save_scrn.refresh_headers(&self.res.save_dir);
        Ok(())
    }

    fn restore_wait_click_after_snapshot(&mut self) -> Result<()> {
        let raw = self.vm.state.tbox.current_text_raw.clone();
        let slot = self.vm.state.tbox.active_slot;
        let voice_id = self.vm.state.tbox.current_voice_id;
        if !raw.is_empty() && self.vm.state.tbox.visible {
            self.res.display_text(&self.vm.state, slot, &raw)?;
            if let Some(active) = &mut self.res.active_text {
                active.typewriter.reveal_all();
            }
            self.res.show_wait_cursor = true;
            self.last_yield = Some(VmYield::WaitClick {
                slot,
                clear_after: false,
                is_dialogue: true,
                voice_id,
            });
        } else {
            self.res.active_text = None;
            self.res.show_wait_cursor = false;
            self.last_yield = Some(VmYield::WaitClick {
                slot: 0,
                clear_after: false,
                is_dialogue: false,
                voice_id: 0,
            });
        }
        Ok(())
    }

    /// Load slot `KhDLsave%02d.dat` (`1..=100`) and restore live + rollback state (`0x41dff0`).
    pub fn load_slot(&mut self, slot_num: u16) -> Result<()> {
        let path = self.res.save_dir.join(format!("KhDLsave{slot_num:02}.dat"));
        let slot = SlotSave2007::load_file(path)?;
        self.vm.state.import_slot_save(&slot);
        self.scene = TopScene::MainGame;
        self.title_state.script_driven = false;
        self.close_all_modals();
        self.last_yield = None;
        self.pending_sleep_ms = 0;
        self.pending_select = None;
        self.res.active_text = None;
        self.res.show_wait_cursor = false;
        self.vm.sync_loaded_script_and_host(&mut self.res)?;
        self.sync_se_from_vm_state();
        self.restore_wait_click_after_snapshot()?;
        self.compose_ui_and_commit()?;
        Ok(())
    }

    /// Trigger one-step rollback restore (`0x424060`).
    pub fn trigger_rollback(&mut self) -> Result<()> {
        self.vm.rollback_restore(&mut self.res)?;
        self.last_yield = None;
        self.pending_sleep_ms = 0;
        self.pending_select = None;
        self.res.active_text = None;
        self.res.show_wait_cursor = false;
        self.sync_se_from_vm_state();
        self.restore_wait_click_after_snapshot()?;
        self.compose_ui_and_commit()?;
        Ok(())
    }

    fn sync_se_from_vm_state(&mut self) {
        if self.vm.state.active_se_id != 0 {
            let name = format!("{:04}.wav", self.vm.state.active_se_id);
            if let Ok(audio) = self.res.load_audio(AssetCategory::Wav, &name) {
                self.res.last_loaded_se = Some(audio.clone());
                if self.vm.state.active_se_loop {
                    if let Ok(mut mixer) = self.res.mixer.lock() {
                        mixer.play_se0(audio, LoopMode::Infinite, Some(0));
                    }
                }
            }
        }
    }

    fn finish_recall_to_menu(&mut self) -> Result<()> {
        self.vm.state.recall_flag = false;
        self.vm.state.fast_skip_active = false;
        self.auto_mode = false;
        self.res.active_text = None;
        self.res.show_wait_cursor = false;
        self.pending_select = None;
        self.last_yield = None;
        if let Ok(mut m) = self.res.mixer.lock() {
            m.stop_all();
        }
        self.scene = TopScene::SceneReplay;
        self.compose_ui_and_commit()
    }

    fn advance_vm_until_yield(&mut self) -> Result<()> {
        for _ in 0..1_000_000 {
            if let Some(y) = self.vm.step_one(&mut self.res)? {
                // If 0399.gsc recall script finished or returned to 0301..0303, exit cleanly to SceneReplay
                let sid = self.vm.state.current_script_id();
                if self.vm.state.recall_flag
                    && (matches!(
                        y,
                        VmYield::Halted {
                            reason: HaltReason::AbortFlag
                                | HaltReason::EndOfScript
                                | HaltReason::ExitOpcode
                        }
                    ) || (301..=303).contains(&sid))
                {
                    return self.finish_recall_to_menu();
                }

                // If 0101.gsc launched a gameplay script (`0111` or `>= 1000`), transition out of Title
                if self.scene == TopScene::Title
                    && self.title_state.script_driven
                    && (sid == 111 || sid >= 1000 || sid == 9001)
                {
                    self.scene = TopScene::MainGame;
                    self.title_state.script_driven = false;
                }

                match &y {
                    VmYield::WaitSleepMs(ms) => {
                        self.pending_sleep_ms = *ms;
                        self.res.show_wait_cursor = false;
                    }
                    VmYield::WaitSelect(cp) => {
                        self.pending_select = Some(cp.clone());
                        self.select_6_ui = Select6Overlay::default();
                        self.res.show_wait_cursor = false;
                    }
                    VmYield::WaitMapSelect {
                        mode,
                        param2,
                        unused,
                    } => {
                        self.selmap.active = true;
                        self.selmap.mode = *mode;
                        self.selmap.param2 = *param2;
                        self.selmap.unused = *unused;
                        self.selmap.hovered = None;
                        self.selmap.keyboard_focus = 0;
                        self.res.show_wait_cursor = false;
                    }
                    VmYield::WaitClick { .. } => {
                        self.res.show_wait_cursor = true;
                    }
                    VmYield::Halted {
                        reason: HaltReason::SubsceneDll,
                    } => {
                        self.last_yield = Some(y);
                        return self.enter_scene(TopScene::SubsceneDll);
                    }
                    _ => {
                        self.res.show_wait_cursor = false;
                    }
                }
                self.last_yield = Some(y);
                return Ok(());
            }

            // Intercept 0399.gsc jumping back to 0301..0303 immediately after replay returns
            let sid = self.vm.state.current_script_id();
            if self.vm.state.recall_flag && (301..=303).contains(&sid) {
                return self.finish_recall_to_menu();
            }
        }
        Ok(())
    }

    fn sync_compane_origin(&mut self) {
        if let Some((cx, cy)) = self.vm.state.tbox.compane_pos {
            self.compane.origin_x = i32::from(cx);
            self.compane.origin_y = i32::from(cy);
        }
    }

    /// Recompose the current UI/scene into the presentation framebuffer.
    pub fn refresh_display(&mut self) -> Result<()> {
        self.compose_ui_and_commit()
    }

    fn compose_ui_and_commit(&mut self) -> Result<()> {
        if self.res.transition.is_some() {
            return Ok(());
        }
        self.sync_compane_origin();
        if self.scene == TopScene::MainGame || self.title_state.script_driven {
            self.res.compose_layers_to_new_scene(&self.vm.state)?;
        } else {
            self.compose_non_vm_scene_to_new_scene()?;
        }

        // Overlays inside MainGame
        if self.scene == TopScene::MainGame {
            if let Some(cp) = &self.pending_select {
                let sel_a01 = self.res.load_lwg("sel_a01.lwg");
                let sel_q01 = self.res.load_lwg("sel_q01.lwg");
                self.select_6_ui.render(
                    &mut self.res.canvas.new_scene,
                    cp,
                    sel_a01.as_deref(),
                    sel_q01.as_deref(),
                    self.res.fonts.as_ref(),
                    Some(&self.res.vfs),
                    &mut self.res.gaiji_bank,
                    self.vm.state.encoding,
                );
            }
            if self.selmap.active {
                let lwg = self.res.load_lwg("selmap01.lwg");
                self.selmap
                    .render(&mut self.res.canvas.new_scene, lwg.as_deref());
            }
            if !self.res.textbox_hidden_by_user || self.compane.pinned {
                let comp = self.res.load_lwg("compane.lwg");
                self.compane.render(
                    &mut self.res.canvas.new_scene,
                    comp.as_deref(),
                    self.auto_mode,
                    self.vm.state.fast_skip_active,
                    self.res.textbox_hidden_by_user,
                );
            }
        }

        // Topmost modal overlays (SaveScrn / ConfScrn / Backlog)
        if self.save_scrn.active {
            let lwg = self.res.load_lwg("savescrn.lwg");
            let saveconf = self.res.load_lwg("saveconf.lwg");
            let nonbl = self.res.load_lwg("nonbl.lwg");
            self.save_scrn.render(
                &mut self.res.canvas.new_scene,
                lwg.as_deref(),
                saveconf.as_deref(),
                nonbl.as_deref(),
                self.res.fonts.as_ref(),
                Some(&self.res.vfs),
                &mut self.res.gaiji_bank,
                self.vm.state.encoding,
            );
        } else if self.conf_scrn.active {
            let lwg = self.res.load_lwg("confscrn.lwg");
            self.conf_scrn.render(
                &mut self.res.canvas.new_scene,
                lwg.as_deref(),
                &self.res.global_save.config,
            );
        } else if self.backlog.active {
            self.backlog.render(
                &mut self.res.canvas.new_scene,
                &self.vm.state.tbox.backlog,
                self.res.fonts.as_ref(),
                Some(&self.res.vfs),
                &mut self.res.gaiji_bank,
                self.vm.state.encoding,
            );
        }

        self.res.canvas.commit_new_scene();
        Ok(())
    }

    fn compose_non_vm_scene_to_new_scene(&mut self) -> Result<()> {
        filter_fill_argb(&mut self.res.canvas.new_scene, 0x0000_0000);
        match self.scene {
            TopScene::Title => {
                if let Ok(bg) = self
                    .res
                    .load_surface(AssetCategory::Grps, "top_bg")
                    .or_else(|_| self.res.load_surface(AssetCategory::GrpoTp, "9011"))
                {
                    blit_fullscreen_srccopy(&mut self.res.canvas.new_scene, &bg);
                }
            }
            TopScene::LoadMenu | TopScene::ConfigMenu | TopScene::MainGame => {}
            TopScene::CgGallery => {
                if let Some(fs) = self.cg_gallery.fullscreen.clone() {
                    if let Ok(surf) = self
                        .res
                        .load_surface(fs.category, &format!("{:04}", fs.current_img_id))
                    {
                        blit_fullscreen_srccopy(&mut self.res.canvas.new_scene, &surf);
                        if let Some(view) = &mut self.cg_gallery.fullscreen {
                            view.last_image_missing_in_dl = false;
                        }
                    } else if let Some(view) = &mut self.cg_gallery.fullscreen {
                        view.last_image_missing_in_dl = true;
                    }
                    return Ok(());
                }
                if let Ok(bg) = self
                    .res
                    .load_surface(AssetCategory::Grps, "cg_bg")
                    .or_else(|_| self.res.load_surface(AssetCategory::Grpe, "9001"))
                {
                    blit_fullscreen_srccopy(&mut self.res.canvas.new_scene, &bg);
                }
                for slot in 0..CgGalleryState::SLOTS_PER_PAGE {
                    if let Some((_idx, entry)) = self.cg_gallery.current_entry(slot) {
                        let unlocked = self
                            .vm
                            .state
                            .vars
                            .get(entry.var_id as usize)
                            .copied()
                            .unwrap_or(0)
                            != 0;
                        let r = CgGalleryState::slot_rect(slot);
                        if unlocked {
                            if let Ok(thumb) = self
                                .res
                                .load_surface(AssetCategory::Grps, &format!("cg_{:04}", entry.thumb_id))
                                .or_else(|_| {
                                    self.res.load_surface(
                                        entry.full_image_category(),
                                        &format!("{:04}", entry.thumb_id),
                                    )
                                })
                            {
                                if thumb.width() == r.w as u32 && thumb.height() == r.h as u32 {
                                    let params = BlitParams {
                                        blend_mode: 0,
                                        alpha_channel_mode: 6,
                                        ..Default::default()
                                    };
                                    blit(
                                        &mut self.res.canvas.new_scene,
                                        &thumb,
                                        &params,
                                        (r.x, r.y),
                                        Some(Rect::new(r.x, r.y, r.x + r.w, r.y + r.h)),
                                    );
                                } else {
                                    stretch_blit_srccopy(
                                        &mut self.res.canvas.new_scene,
                                        (r.x, r.y, r.w, r.h),
                                        &thumb,
                                        (0, 0, thumb.width() as i32, thumb.height() as i32),
                                    );
                                }
                            }
                        } else if let Ok(locked) = self.res.load_surface(AssetCategory::Grps, "cg_0000") {
                            let params = BlitParams {
                                blend_mode: 0,
                                alpha_channel_mode: 6,
                                ..Default::default()
                            };
                            blit(&mut self.res.canvas.new_scene, &locked, &params, (r.x, r.y), None);
                        }
                    }
                }
                if let Some(fonts) = self.res.fonts.as_ref() {
                    let hdr = format!(
                        "CG GALLERY  TAB {}/7  PAGE {}/5  UNLOCKED {}",
                        self.cg_gallery.tab + 1,
                        self.cg_gallery.page + 1,
                        self.unlocked_cg_count()
                    );
                    render_ascii_or_utf8_label(
                        &mut self.res.canvas.new_scene,
                        fonts,
                        Some(&self.res.vfs),
                        &mut self.res.gaiji_bank,
                        &hdr,
                        self.vm.state.encoding,
                        15,
                        0x00FF_DE00,
                        42,
                        12,
                        560,
                        22,
                    );
                }
            }
            TopScene::SceneReplay => {
                if let Ok(bg) = self
                    .res
                    .load_surface(AssetCategory::Grps, "sc_bg")
                    .or_else(|_| self.res.load_surface(AssetCategory::Grpe, "9010"))
                {
                    blit_fullscreen_srccopy(&mut self.res.canvas.new_scene, &bg);
                }
                if let Some(fonts) = self.res.fonts.as_ref() {
                    let unlocked = SCENE_REPLAY_TABLE
                        .iter()
                        .filter(|e| {
                            self.vm
                                .state
                                .vars
                                .get(e.flag_var as usize)
                                .copied()
                                .unwrap_or(0)
                                != 0
                        })
                        .count();
                    let hdr = format!(
                        "SCENE REPLAY  TAB {}/7  PAGE {}/4  UNLOCKED {}/{}",
                        self.scene_replay.tab + 1,
                        self.scene_replay.page + 1,
                        unlocked,
                        SCENE_REPLAY_TABLE.len()
                    );
                    render_ascii_or_utf8_label(
                        &mut self.res.canvas.new_scene,
                        fonts,
                        Some(&self.res.vfs),
                        &mut self.res.gaiji_bank,
                        &hdr,
                        self.vm.state.encoding,
                        16,
                        0x00FF_DE00,
                        36,
                        14,
                        640,
                        24,
                    );
                }
            }
            TopScene::MusicMode => {
                if let Some(lwg) = self.res.load_lwg("muscmenu.lwg") {
                    for layer in &lwg.layers {
                        let params = BlitParams {
                            blend_mode: 0,
                            alpha_channel_mode: 0,
                            ..Default::default()
                        };
                        blit(
                            &mut self.res.canvas.new_scene,
                            &layer.surface,
                            &params,
                            (layer.x, layer.y),
                            None,
                        );
                    }
                } else if let Ok(bg) = self.res.load_surface(AssetCategory::Grpe, "9009") {
                    blit_fullscreen_srccopy(&mut self.res.canvas.new_scene, &bg);
                }
            }
            TopScene::ExtraMenu => {
                if let Ok(bg) = self
                    .res
                    .load_surface(AssetCategory::Grps, "top_exbg")
                    .or_else(|_| self.res.load_surface(AssetCategory::Grpe, "9011"))
                {
                    blit_fullscreen_srccopy(&mut self.res.canvas.new_scene, &bg);
                }
            }
            TopScene::SubsceneDll => {}
        }
        Ok(())
    }

    fn on_video_finished(&mut self) -> Result<()> {
        if let Ok(mut q) = self.res.video_audio_fifo.lock() {
            q.clear();
        }
        match self.title_movie_stage {
            TitleMovieStage::Playing0002 => {
                if let Ok(b) = self.res.vfs.read(AssetCategory::Mov, "0001.mpg") {
                    self.title_movie_stage = TitleMovieStage::Playing0001;
                    self.res.mpeg.start_bytes(b);
                    return Ok(());
                }
                self.enter_title_screen()?;
            }
            TitleMovieStage::Playing0001 => {
                self.enter_title_screen()?;
            }
            TitleMovieStage::None => {
                if matches!(self.last_yield, Some(VmYield::WaitVideo { .. })) {
                    self.last_yield = None;
                    if self.scene == TopScene::MainGame || self.title_state.script_driven {
                        self.advance_vm_until_yield()?;
                        self.compose_ui_and_commit()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Advance the engine simulation by `delta_ms` milliseconds.
    pub fn tick(&mut self, delta_ms: u32) -> Result<()> {
        if !self.pending_dialogs.is_empty() {
            return Ok(());
        }

        // 1. MPEG-1 video playback (`CMpegWindow`)
        if self.res.mpeg.phase() == MpegWindowPhase::Playing {
            let (_pts, done) = self.res.mpeg.step_video_800x600(&mut self.res.canvas.front)?;
            if let Some(player) = self.res.mpeg.player_mut() {
                while let Some(pkt) = player.decode_audio_packet() {
                    if let Ok(mut q) = self.res.video_audio_fifo.lock() {
                        q.extend(pkt.samples_interleaved);
                    }
                }
            }
            if done.is_some() {
                self.on_video_finished()?;
            }
            return Ok(());
        }

        // Pause VM & timers when a modal overlay is open
        if self.conf_scrn.active || self.save_scrn.active || self.backlog.active {
            return Ok(());
        }

        // 2. Active screen transition
        if let Some(mut tr) = self.res.transition.take() {
            tr.accum_ms += delta_ms;
            if tr.accum_ms >= u32::from(tr.next_delay_ms) {
                tr.accum_ms = 0;
                if let Some(next_delay) = tr.runner.step(&mut self.res.canvas) {
                    tr.next_delay_ms = next_delay;
                    if !tr.runner.is_done() {
                        self.res.transition = Some(tr);
                        return Ok(());
                    }
                }
            } else {
                self.res.transition = Some(tr);
                return Ok(());
            }
            if matches!(self.last_yield, Some(VmYield::WaitTransition { .. })) {
                self.last_yield = None;
            }
        }

        // 3. Timed sleep (`0x0D`)
        if self.pending_sleep_ms > 0 {
            if delta_ms < self.pending_sleep_ms {
                self.pending_sleep_ms -= delta_ms;
                return Ok(());
            }
            self.pending_sleep_ms = 0;
            if matches!(self.last_yield, Some(VmYield::WaitSleepMs(_))) {
                self.last_yield = None;
            }
        }

        // 4. Audio wait (`0x44` / `0x45`)
        if let Some(VmYield::WaitAudio { is_voice }) = self.last_yield {
            if self.res.is_audio_playing(is_voice) {
                return Ok(());
            }
            self.last_yield = None;
        }

        // 5. Typewriter 50ms animation, wait cursor blink, and Auto-mode while waiting click
        if matches!(self.last_yield, Some(VmYield::WaitClick { .. })) {
            let mut need_refresh = false;
            if let Some(active) = &mut self.res.active_text {
                if !active.typewriter.is_finished() {
                    active.accum_ms += delta_ms;
                    while active.accum_ms >= TEXT_TIMER_PERIOD_MS && !active.typewriter.is_finished()
                    {
                        active.accum_ms -= TEXT_TIMER_PERIOD_MS;
                        active.typewriter.tick_50ms();
                        need_refresh = true;
                    }
                } else {
                    let prev_blink = self.res.wait_cursor_ms / 400;
                    self.res.wait_cursor_ms = self.res.wait_cursor_ms.wrapping_add(delta_ms);
                    if self.res.wait_cursor_ms / 400 != prev_blink {
                        need_refresh = true;
                    }
                }
            }
            if need_refresh {
                self.compose_ui_and_commit()?;
            }
            if self.vm.state.fast_skip_active {
                self.advance_dialogue_click()?;
                return Ok(());
            }
            let text_done = self
                .res
                .active_text
                .as_ref()
                .is_none_or(|a| a.typewriter.is_finished());
            if self.auto_mode
                && self.scene == TopScene::MainGame
                && text_done
                && !self.res.textbox_hidden_by_user
                && !self.res.is_audio_playing(true)
            {
                self.auto_accum_ms += delta_ms;
                if self.auto_accum_ms >= u32::from(self.res.global_save.config.auto_wait_ms()) {
                    self.auto_accum_ms = 0;
                    self.advance_dialogue_click()?;
                }
            }
            return Ok(());
        }

        // 6. Select / TimerLoop / Menu / Halted stay paused until user event
        if matches!(
            self.last_yield,
            Some(
                VmYield::WaitSelect(_)
                    | VmYield::WaitMapSelect { .. }
                    | VmYield::WaitTimerLoop { .. }
                    | VmYield::MenuPause
                    | VmYield::Halted { .. }
            )
        ) {
            return Ok(());
        }

        if self.scene == TopScene::MainGame || self.title_state.script_driven {
            self.advance_vm_until_yield()?;
            self.compose_ui_and_commit()?;
        }
        Ok(())
    }

    /// Advance dialogue on LeftClick / Enter / Space (`0x415050`).
    pub fn advance_dialogue_click(&mut self) -> Result<()> {
        if let Some(active) = &mut self.res.active_text {
            if !active.typewriter.is_finished() {
                active.typewriter.reveal_all();
                self.compose_ui_and_commit()?;
                return Ok(());
            }
        }
        let voice_cut = self.res.global_save.config.voice_cut_on_click();
        if let Ok(mut mixer) = self.res.mixer.lock() {
            mixer.on_dialogue_advance(voice_cut);
        }
        self.auto_accum_ms = 0;
        self.vm.complete_wait_click(&mut self.res)?;
        self.last_yield = None;
        self.advance_vm_until_yield()?;
        self.compose_ui_and_commit()
    }

    /// Select branch choice `choice_idx` when paused at `0x0E select_6`.
    pub fn choose_select(&mut self, choice_idx: usize) -> Result<()> {
        let Some(cp) = self.pending_select.take() else {
            return Ok(());
        };
        let targets: Vec<u32> = cp.choices.iter().map(|c| c.1).collect();
        self.vm.complete_select(&mut self.res, &targets, choice_idx)?;
        self.last_yield = None;
        self.advance_vm_until_yield()?;
        self.compose_ui_and_commit()
    }

    /// Perform a Title screen button action directly.
    pub fn trigger_title_action(&mut self, action: TitleAction) -> Result<()> {
        match action {
            TitleAction::StartNewGame => {
                let start_sid = if self.res.vfs.exists(AssetCategory::Scr, "0111.gsc") {
                    111
                } else {
                    0
                };
                self.start_game_script(start_sid)?;
            }
            TitleAction::ContinueScript1 => {
                self.start_game_script(1)?;
            }
            TitleAction::OpenLoad => self.enter_scene(TopScene::LoadMenu)?,
            TitleAction::OpenConfig => self.enter_scene(TopScene::ConfigMenu)?,
            TitleAction::OpenExtra => self.enter_scene(TopScene::ExtraMenu)?,
            TitleAction::StartOmake7000 => {
                if self.vm.state.vars[7000] != 0 {
                    let sid = if self.res.vfs.exists(AssetCategory::Scr, "7000.gsc") {
                        7000
                    } else {
                        9001
                    };
                    self.start_game_script(sid)?;
                }
            }
            TitleAction::RequestExit => {
                self.request_confirm(ConfirmAction::ExitGame);
            }
        }
        Ok(())
    }

    /// Hit-test VM hotspot layers (`anim_active`) at `(px, py)` from highest `z_order` downward.
    fn hit_test_vm_hotspot(&mut self, px: i32, py: i32) -> Option<usize> {
        let mut order: Vec<usize> = (0..self.vm.state.layers.len())
            .filter(|&i| self.vm.state.layers[i].visible && self.vm.state.layers[i].anim_active)
            .collect();
        order.sort_by_key(|&i| (self.vm.state.layers[i].z_order, i));
        order.reverse();

        for idx in order {
            let l = &self.vm.state.layers[idx];
            let cat = dir_prefix_to_category(&l.dir_prefix);
            let iid = if l.img_id != 0 {
                l.img_id
            } else {
                l.subframe_img_ids[0]
            };
            if iid == 0 {
                continue;
            }
            let (bx, by) = if l.img_id != 0 {
                (l.x, l.y)
            } else {
                l.subframe_coords[0]
            };
            if let Ok(surf) = self.res.load_surface(cat, &format!("{iid:04}")) {
                let (ax, ay) = anchored_top_left(
                    bx,
                    by,
                    l.anchor_x,
                    l.anchor_y,
                    surf.width() as i32,
                    surf.height() as i32,
                );
                if UiRect::new(ax, ay, surf.width() as i32, surf.height() as i32).contains(px, py) {
                    return Some(idx);
                }
            }
        }
        None
    }

    fn handle_vm_hotspot_click(&mut self, var_idx: i16, layer_idx: usize) -> Result<()> {
        let (kind, btn_id) = {
            let l = &self.vm.state.layers[layer_idx];
            let id = match l.anim_kind {
                1 => l.anim_y[0],
                2 => l.anim_alpha[0],
                _ => l.anim_x[0],
            };
            (l.anim_kind, id)
        };
        if kind == 1 {
            // `0x41b800(btn_id)` system command dispatcher
            match btn_id {
                0 => {
                    self.request_confirm(ConfirmAction::ExitGame);
                }
                1 => {
                    self.request_confirm(ConfirmAction::ReturnToTitle);
                }
                2 => self.open_save_overlay(SaveScrnMode::Load)?,
                3 => self.open_config_overlay()?,
                4 => self.open_save_overlay(SaveScrnMode::Save)?,
                5 => self.open_save_overlay(SaveScrnMode::Load)?,
                6 => self.open_config_overlay()?,
                7 => self.enter_scene(TopScene::MusicMode)?,
                8 => self.enter_scene(TopScene::CgGallery)?,
                9 => self.enter_scene(TopScene::SceneReplay)?,
                10 => self.enter_scene(TopScene::ExtraMenu)?,
                _ => {}
            }
            return Ok(());
        }
        // `0x41b7a0` / `0x41b8f0`: writes `vars[var_idx] = btn_id` and exits WaitTimerLoop
        self.vm.complete_timer_wait_loop(var_idx, btn_id);
        self.res.hovered_vm_layer = None;
        self.last_yield = None;
        self.advance_vm_until_yield()?;
        self.compose_ui_and_commit()
    }

    /// Handle pointer input (`800x600` logical coordinates).
    pub fn pointer_event(&mut self, kind: PointerKind, x: i32, y: i32) -> Result<()> {
        if self.res.mpeg.phase() == MpegWindowPhase::Playing {
            if kind == PointerKind::Up
                && self.res.mpeg.handle_event(MpegWindowEvent::LeftButtonUp).is_some()
            {
                self.on_video_finished()?;
            }
            return Ok(());
        }

        if !self.pending_dialogs.is_empty() {
            return Ok(());
        }

        // RightUp / Esc modal stack (`0x41a2a0`)
        if kind == PointerKind::RightUp {
            return self.handle_cancel_or_right_click();
        }
        if kind == PointerKind::MiddleUp {
            if self.scene == TopScene::MainGame {
                self.perform_system_action(SystemPanelAction::HideTextbox)?;
            }
            return Ok(());
        }
        if kind == PointerKind::WheelUp {
            return self.handle_wheel(true);
        }
        if kind == PointerKind::WheelDown {
            return self.handle_wheel(false);
        }

        // Modal overlays take top priority
        if self.save_scrn.active {
            let lwg = self.res.load_lwg("savescrn.lwg");
            match kind {
                PointerKind::Move => {
                    self.save_scrn.handle_move(lwg.as_deref(), x, y);
                    self.compose_ui_and_commit()?;
                }
                PointerKind::Up => {
                    if let Some(act) = self.save_scrn.handle_up(lwg.as_deref(), x, y) {
                        match act {
                            SaveScrnAction::Close => {
                                self.save_scrn.active = false;
                                if self.scene == TopScene::LoadMenu {
                                    self.enter_title_screen()?;
                                } else {
                                    self.compose_ui_and_commit()?;
                                }
                            }
                            SaveScrnAction::PageChanged(p) => {
                                self.res.global_save.config.set_last_save_page(p as u16);
                                self.save_scrn.refresh_headers(&self.res.save_dir);
                                self.compose_ui_and_commit()?;
                            }
                            SaveScrnAction::SlotSelected { slot_num, occupied } => {
                                match self.save_scrn.mode {
                                    SaveScrnMode::Save => {
                                        if occupied {
                                            self.request_confirm(ConfirmAction::SaveOverwrite(
                                                slot_num,
                                            ));
                                        } else {
                                            let (sy, smo, sd, sh, smi) = current_save_time(); self.save_slot(slot_num, sy, smo, sd, sh, smi)?;
                                            self.save_global()?;
                                            self.compose_ui_and_commit()?;
                                        }
                                    }
                                    SaveScrnMode::Load => {
                                        if occupied {
                                            self.request_confirm(ConfirmAction::LoadSlot(slot_num));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if self.conf_scrn.active {
            let lwg = self.res.load_lwg("confscrn.lwg");
            match kind {
                PointerKind::Move => {
                    let changed = self.conf_scrn.handle_move(
                        lwg.as_deref(),
                        &mut self.res.global_save.config,
                        x,
                        y,
                    );
                    if changed {
                        self.res.sync_mixer_from_config();
                        self.vm.state.tbox.speed = self.res.global_save.config.text_speed();
                    }
                    self.compose_ui_and_commit()?;
                }
                PointerKind::Down => {
                    if self.conf_scrn.handle_down(
                        lwg.as_deref(),
                        &mut self.res.global_save.config,
                        x,
                        y,
                    ) {
                        self.res.sync_mixer_from_config();
                        self.vm.state.tbox.speed = self.res.global_save.config.text_speed();
                        self.compose_ui_and_commit()?;
                    }
                }
                PointerKind::Up => {
                    if let Some(act) = self.conf_scrn.handle_up(
                        lwg.as_deref(),
                        &mut self.res.global_save.config,
                        x,
                        y,
                    ) {
                        self.res.sync_mixer_from_config();
                        self.vm.state.tbox.speed = self.res.global_save.config.text_speed();
                        match act {
                            ConfScrnAction::UpdatedConfig => {
                                self.compose_ui_and_commit()?;
                            }
                            ConfScrnAction::OpenLoad => {
                                self.open_save_overlay(SaveScrnMode::Load)?;
                            }
                            ConfScrnAction::OpenSave => {
                                if self.conf_scrn.return_to_main_game {
                                    self.conf_scrn.active = false;
                                    self.save_scrn.active = true;
                                    self.save_scrn.mode = SaveScrnMode::Save;
                                    self.save_scrn.refresh_headers(&self.res.save_dir);
                                    self.compose_ui_and_commit()?;
                                }
                            }
                            ConfScrnAction::RequestExit => {
                                self.request_confirm(ConfirmAction::ExitGame);
                            }
                            ConfScrnAction::RequestTitle => {
                                self.request_confirm(ConfirmAction::ReturnToTitle);
                            }
                            ConfScrnAction::Close => {
                                self.save_global()?;
                                self.conf_scrn.active = false;
                                if self.scene == TopScene::ConfigMenu {
                                    self.enter_title_screen()?;
                                } else {
                                    self.compose_ui_and_commit()?;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if self.backlog.active {
            if kind == PointerKind::Up {
                if let Some(act) = self.backlog.handle_up(&self.vm.state.tbox.backlog, x, y) {
                    match act {
                        BacklogAction::Close => self.close_backlog()?,
                        BacklogAction::ReplayVoice(vid) => {
                            self.res.play_voice(vid, 0, 0, 0);
                        }
                        BacklogAction::Scrolled => {}
                    }
                } else {
                    self.close_backlog()?;
                }
            }
            return Ok(());
        }

        // Script-driven TimerWaitLoop (e.g. 0101.gsc Title, 0201.gsc, 0301.gsc, 0401.gsc)
        if let Some(VmYield::WaitTimerLoop { var_idx, .. }) = self.last_yield {
            let hit = self.hit_test_vm_hotspot(x, y);
            match kind {
                PointerKind::Move => {
                    if self.res.hovered_vm_layer != hit {
                        self.res.hovered_vm_layer = hit;
                        self.compose_ui_and_commit()?;
                    }
                }
                PointerKind::Up => {
                    if let Some(li) = hit {
                        self.handle_vm_hotspot_click(var_idx, li)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        // Top-level non-MainGame scenes
        match self.scene {
            TopScene::Title => {
                if kind == PointerKind::Move {
                    self.title_state.hovered_exe_btn =
                        self.title_state.hit_test_exe(self.vm.state.vars[7000], x, y);
                } else if kind == PointerKind::Up {
                    if let Some(idx) =
                        self.title_state.hit_test_exe(self.vm.state.vars[7000], x, y)
                    {
                        if let Some(act) = TitleSceneState::exe_button_action(idx) {
                            self.trigger_title_action(act)?;
                        }
                    }
                }
                return Ok(());
            }
            TopScene::CgGallery => {
                if kind == PointerKind::Up {
                    if self.cg_gallery.fullscreen.is_some() {
                        self.cg_gallery.advance_fullscreen_variant();
                        self.compose_ui_and_commit()?;
                        return Ok(());
                    }
                    if CgGalleryState::return_rect().contains(x, y) {
                        return self.enter_scene(TopScene::ExtraMenu);
                    }
                    if CgGalleryState::prev_rect().contains(x, y) {
                        self.cg_gallery.page =
                            (self.cg_gallery.page + CgGalleryState::PAGES_PER_TAB - 1)
                                % CgGalleryState::PAGES_PER_TAB;
                        return self.compose_ui_and_commit();
                    }
                    if CgGalleryState::next_rect().contains(x, y) {
                        self.cg_gallery.page =
                            (self.cg_gallery.page + 1) % CgGalleryState::PAGES_PER_TAB;
                        return self.compose_ui_and_commit();
                    }
                    for t in 0..CgGalleryState::TAB_COUNT {
                        if CgGalleryState::tab_rect(t).contains(x, y) {
                            self.cg_gallery.tab = t;
                            self.cg_gallery.page = 0;
                            return self.compose_ui_and_commit();
                        }
                    }
                    for s in 0..CgGalleryState::SLOTS_PER_PAGE {
                        if CgGalleryState::slot_rect(s).contains(x, y) {
                            if let Some((idx, entry)) = self.cg_gallery.current_entry(s) {
                                let unlocked = self
                                    .vm
                                    .state
                                    .vars
                                    .get(entry.var_id as usize)
                                    .copied()
                                    .unwrap_or(0)
                                    != 0;
                                if unlocked {
                                    self.cg_gallery.open_entry(idx, entry);
                                    return self.compose_ui_and_commit();
                                }
                            }
                        }
                    }
                }
                return Ok(());
            }
            TopScene::ExtraMenu => {
                if kind == PointerKind::Move {
                    self.extra_menu.hovered_btn = self.extra_menu.hit_test(x, y);
                } else if kind == PointerKind::Up {
                    if let Some(idx) = self.extra_menu.hit_test(x, y) {
                        if let Some(act) = ExtraMenuState::action_for_btn(idx) {
                            match act {
                                ExtraAction::CgGallery => self.enter_scene(TopScene::CgGallery)?,
                                ExtraAction::SceneReplay => {
                                    self.enter_scene(TopScene::SceneReplay)?
                                }
                                ExtraAction::MusicMode => self.enter_scene(TopScene::MusicMode)?,
                                ExtraAction::Omake9000 => {
                                    if self.res.vfs.exists(AssetCategory::Scr, "9000.gsc") {
                                        self.start_game_script(9000)?;
                                    } else {
                                        self.extra_menu.omake_missing_9000 = true;
                                    }
                                }
                                ExtraAction::ReturnToTitle => self.enter_title_screen()?,
                            }
                        }
                    }
                }
                return Ok(());
            }
            TopScene::SceneReplay | TopScene::MusicMode | TopScene::SubsceneDll => {
                return Ok(());
            }
            TopScene::LoadMenu | TopScene::ConfigMenu | TopScene::MainGame => {}
        }

        // MainGame: SelMap (`0xD5`), Select6 (`0x0E`), Compane (`compane.lwg`), Dialogue click
        if self.selmap.active {
            let lwg = self.res.load_lwg("selmap01.lwg");
            match kind {
                PointerKind::Move => {
                    let h = self.selmap.hit_test(lwg.as_deref(), x, y);
                    if self.selmap.hovered != h {
                        self.selmap.hovered = h;
                        self.compose_ui_and_commit()?;
                    }
                }
                PointerKind::Up => {
                    if let Some(idx) = self.selmap.hit_test(lwg.as_deref(), x, y) {
                        self.choose_map_select(idx)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if let Some(cp) = &self.pending_select {
            let count = cp.choices.len();
            match kind {
                PointerKind::Move => {
                    let h = self.select_6_ui.hit_test(count, x, y);
                    if self.select_6_ui.hovered != h {
                        self.select_6_ui.hovered = h;
                        self.compose_ui_and_commit()?;
                    }
                }
                PointerKind::Up => {
                    if let Some(idx) = self.select_6_ui.hit_test(count, x, y) {
                        self.choose_select(idx)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        // Check `compane.lwg` controls before advancing dialogue
        self.sync_compane_origin();
        let comp_lwg = self.res.load_lwg("compane.lwg");
        if !self.res.textbox_hidden_by_user || self.compane.pinned {
            match kind {
                PointerKind::Move => {
                    let h = self.compane.hit_test(comp_lwg.as_deref(), x, y);
                    if self.compane.hovered != h {
                        self.compane.hovered = h;
                        self.compose_ui_and_commit()?;
                    }
                }
                PointerKind::Up => {
                    if let Some(btn) = self.compane.hit_test(comp_lwg.as_deref(), x, y) {
                        match btn {
                            CompaneButton::Slide => {
                                self.compane.pinned = !self.compane.pinned;
                                self.compose_ui_and_commit()?;
                            }
                            CompaneButton::Rev => self.perform_system_action(SystemPanelAction::Backlog)?,
                            CompaneButton::Bak => self.perform_system_action(SystemPanelAction::Rollback)?,
                            CompaneButton::Fow => self.perform_system_action(SystemPanelAction::Skip)?,
                            CompaneButton::Next => self.perform_system_action(SystemPanelAction::Advance)?,
                            CompaneButton::Auto => self.perform_system_action(SystemPanelAction::Auto)?,
                            CompaneButton::Hide => self.perform_system_action(SystemPanelAction::HideTextbox)?,
                            CompaneButton::Voc => self.perform_system_action(SystemPanelAction::ReplayVoice)?,
                        }
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        if kind == PointerKind::Up
            && (0..GAME_WIDTH as i32).contains(&x)
            && (0..GAME_HEIGHT as i32).contains(&y)
            && matches!(self.last_yield, Some(VmYield::WaitClick { .. }))
        {
            self.advance_dialogue_click()?;
        }
        Ok(())
    }

    fn handle_cancel_or_right_click(&mut self) -> Result<()> {
        if self.scene == TopScene::CgGallery && self.cg_gallery.fullscreen.is_some() {
            self.cg_gallery.fullscreen = None;
            return self.compose_ui_and_commit();
        }
        if self.save_scrn.active {
            self.save_scrn.active = false;
            if self.scene == TopScene::LoadMenu {
                return self.enter_title_screen();
            }
            return self.compose_ui_and_commit();
        }
        if self.conf_scrn.active {
            self.save_global()?;
            self.conf_scrn.active = false;
            if self.scene == TopScene::ConfigMenu {
                return self.enter_title_screen();
            }
            return self.compose_ui_and_commit();
        }
        if self.backlog.active {
            return self.close_backlog();
        }
        if self.scene == TopScene::MainGame {
            if self.res.textbox_hidden_by_user {
                self.res.textbox_hidden_by_user = false;
                return self.compose_ui_and_commit();
            }
            return self.open_backlog();
        }
        Ok(())
    }

    fn handle_wheel(&mut self, up: bool) -> Result<()> {
        if self.backlog.active {
            if up {
                self.backlog.scroll_up(self.vm.state.tbox.backlog.len());
            } else if self.backlog.scroll_from_bottom == 0 {
                return self.close_backlog();
            } else {
                self.backlog.scroll_down();
            }
            return self.compose_ui_and_commit();
        }
        if self.selmap.active {
            if up {
                self.selmap.keyboard_focus = (self.selmap.keyboard_focus + 4) % 5;
            } else {
                self.selmap.keyboard_focus = (self.selmap.keyboard_focus + 1) % 5;
            }
            return self.compose_ui_and_commit();
        }
        if let Some(cp) = &self.pending_select {
            let n = cp.choices.len().max(1);
            if up {
                self.select_6_ui.keyboard_focus = (self.select_6_ui.keyboard_focus + n - 1) % n;
            } else {
                self.select_6_ui.keyboard_focus = (self.select_6_ui.keyboard_focus + 1) % n;
            }
            return self.compose_ui_and_commit();
        }
        if self.scene == TopScene::MainGame && up && !self.vm.state.tbox.backlog.is_empty() {
            return self.open_backlog();
        }
        Ok(())
    }

    /// Handle keyboard input using Win32 `VK_*` virtual key codes (`0x41a2a0`).
    pub fn key_event(&mut self, vk_code: u16, is_down: bool) -> Result<()> {
        if self.res.mpeg.phase() == MpegWindowPhase::Playing {
            return Ok(()); // `0x450900` ignores all keyboard events during MPG playback
        }
        match vk_code {
            0x70 => {
                // `VK_F1` (`0x41e490`): release builds have DAT_0048318c == NULL -> safe no-op
            }
            0x11 => {
                // `VK_CONTROL`: hold skip
                self.vm.state.fast_skip_active = is_down;
            }
            0x09 if is_down => {
                // `VK_TAB` (`0x415070`): toggle Skip
                if self.scene == TopScene::MainGame {
                    self.vm.state.fast_skip_active = !self.vm.state.fast_skip_active;
                    self.compose_ui_and_commit()?;
                }
            }
            0x10 if is_down => {
                // `VK_SHIFT` (`0x41b980` / `0x415130`)
                if self.scene == TopScene::CgGallery && self.cg_gallery.fullscreen.is_some() {
                    self.cg_gallery.fullscreen = None;
                    self.compose_ui_and_commit()?;
                } else if self.scene == TopScene::MainGame
                    && !self.save_scrn.active
                    && !self.conf_scrn.active
                    && !self.backlog.active
                {
                    self.res.textbox_hidden_by_user = !self.res.textbox_hidden_by_user;
                    self.compose_ui_and_commit()?;
                }
            }
            0x1B if is_down => {
                // `VK_ESCAPE`
                self.handle_cancel_or_right_click()?;
            }
            0x26 if is_down => {
                // `VK_UP`
                self.handle_wheel(true)?;
            }
            0x28 if is_down => {
                // `VK_DOWN`
                self.handle_wheel(false)?;
            }
            0x0D | 0x20 if is_down => {
                // `VK_RETURN` / `VK_SPACE`
                if self.selmap.active {
                    self.choose_map_select(self.selmap.keyboard_focus)?;
                } else if self.pending_select.is_some() {
                    let focus = self.select_6_ui.keyboard_focus;
                    self.choose_select(focus)?;
                } else if matches!(self.last_yield, Some(VmYield::WaitClick { .. })) {
                    self.advance_dialogue_click()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Enqueue one of the 10 `MessageBoxA` alerts/prompts with exact JP/CHS wording.
    pub fn request_messagebox(
        &mut self,
        callsite: MessageBoxCallsite,
        detail: Option<&str>,
    ) -> u64 {
        let (kind, title, message) = messagebox_text(callsite, self.res.language_mode, detail);
        let id = self.next_dialog_id;
        self.next_dialog_id += 1;
        self.pending_dialogs.push_back(PendingDialog {
            id,
            kind,
            title,
            message,
            action: ConfirmAction::AcknowledgeOnly,
        });
        id
    }

    /// Enqueue one of the 7 `0x445e90` confirmation prompts (JP/CHS authentic wording).
    pub fn request_confirm(&mut self, action: ConfirmAction) -> u64 {
        let (title, msg) = confirm_action_text(action, self.res.language_mode);
        let id = self.next_dialog_id;
        self.next_dialog_id += 1;
        self.pending_dialogs.push_back(PendingDialog {
            id,
            kind: DialogKind::YesNoConfirm,
            title: title.to_string(),
            message: msg.to_string(),
            action,
        });
        id
    }

    /// Pop next pending native dialog if any.
    pub fn poll_pending_dialog(&mut self) -> Option<PendingDialog> {
        self.pending_dialogs.front().cloned()
    }

    /// Deliver user response (`accepted == IDYES`) for `dialog_id`.
    pub fn confirm_dialog_response(&mut self, dialog_id: u64, accepted: bool) -> Result<()> {
        let Some(front) = self.pending_dialogs.front() else {
            return Ok(());
        };
        if front.id != dialog_id {
            return Ok(());
        }
        let dlg = self.pending_dialogs.pop_front().unwrap();
        if !accepted {
            return Ok(());
        }
        match dlg.action {
            ConfirmAction::ExitGame => {
                self.save_global()?;
                self.exit_requested = true;
            }
            ConfirmAction::ReturnToTitle => {
                self.vm.state.recall_flag = false;
                if let Ok(mut m) = self.res.mixer.lock() {
                    m.stop_all();
                }
                self.enter_title_screen()?;
            }
            ConfirmAction::SaveOverwrite(slot) => {
                let (sy, smo, sd, sh, smi) = current_save_time(); self.save_slot(slot, sy, smo, sd, sh, smi)?;
                self.save_global()?;
                self.compose_ui_and_commit()?;
            }
            ConfirmAction::LoadSlot(slot) => {
                self.load_slot(slot)?;
            }
            ConfirmAction::AcknowledgeOnly => {}
        }
        Ok(())
    }

    /// Copy `800x600x4` standard presentation BGRA8 (`A = 0xFF`) into `out_bgra8`.
    pub fn copy_present_bgra8(&self, out_bgra8: &mut [u8]) -> Result<()> {
        let expected = (GAME_WIDTH as usize) * (GAME_HEIGHT as usize) * 4;
        if out_bgra8.len() != expected {
            return Err(FujihimeError::InvalidSurface {
                reason: format!(
                    "expected {expected} bytes for 800x600 BGRA8, got {}",
                    out_bgra8.len()
                ),
            });
        }
        self.res.canvas.front.write_present_bgra8(out_bgra8);
        Ok(())
    }

    /// Render interleaved stereo `f32` PCM (`44,100 Hz`) for `AVAudioEngine`.
    pub fn render_audio_f32(&self, out_interleaved: &mut [f32]) -> usize {
        let frames = out_interleaved.len() / 2;
        if frames == 0 {
            return 0;
        }
        if let Ok(mut mixer) = self.res.mixer.lock() {
            mixer.render_stereo_f32(&mut out_interleaved[..frames * 2]);
        } else {
            out_interleaved.fill(0.0);
        }
        if let Ok(mut vq) = self.res.video_audio_fifo.lock() {
            if !vq.is_empty() {
                for dst in out_interleaved[..frames * 2].iter_mut() {
                    if let Some(s) = vq.pop_front() {
                        *dst = (*dst + s).clamp(-1.0, 1.0);
                    } else {
                        break;
                    }
                }
            }
        }
        frames
    }

    /// Query current engine status.
    pub fn status(&self) -> EngineStatus {
        EngineStatus {
            scene: self.scene,
            language_mode: self.res.language_mode,
            is_playing_video: self.res.mpeg.phase() == MpegWindowPhase::Playing,
            is_waiting_click: matches!(self.last_yield, Some(VmYield::WaitClick { .. })),
            is_waiting_select: matches!(self.last_yield, Some(VmYield::WaitSelect(_))),
            is_transitioning: self.res.transition.is_some(),
            exit_requested: self.exit_requested,
            current_script_id: self.vm.state.current_script_id(),
            current_pc: self.vm.state.pc,
        }
    }

    /// Access total unlocked CG count across the 102 sets in `0x4648a8`.
    pub fn unlocked_cg_count(&self) -> usize {
        CG_GALLERY_TABLE
            .iter()
            .filter(|e| {
                self.vm
                    .state
                    .vars
                    .get(e.var_id as usize)
                    .copied()
                    .unwrap_or(0)
                    != 0
            })
            .count()
    }

    /// Record diagnostic error string.
    pub fn set_last_error(&mut self, err: impl Into<String>) {
        self.res.last_error = Some(err.into());
    }

    /// Retrieve last diagnostic error string.
    pub fn last_error(&self) -> Option<&str> {
        self.res.last_error.as_deref()
    }

    /// Access `AssetVfs`.
    pub fn vfs(&self) -> &AssetVfs {
        &self.res.vfs
    }

    /// Access `Canvas800x600`.
    pub fn canvas(&self) -> &Canvas800x600 {
        &self.res.canvas
    }

    /// Access `AudioMixer` mutex.
    pub fn mixer(&self) -> &Arc<Mutex<AudioMixer>> {
        &self.res.mixer
    }
}

fn dir_prefix_to_category(prefix: &str) -> AssetCategory {
    let norm = prefix
        .trim_start_matches(".\\")
        .trim_start_matches("./")
        .trim_start_matches('\\')
        .to_ascii_lowercase();
    match norm.as_str() {
        "grpe" => AssetCategory::Grpe,
        "grpo" => AssetCategory::Grpo,
        "grpp" => AssetCategory::Grpp,
        "grps" => AssetCategory::Grps,
        "grpf" => AssetCategory::Grpf,
        "grpo_ex" => AssetCategory::GrpoEx,
        "grpo_tp" => AssetCategory::GrpoTp,
        _ => AssetCategory::Grpo,
    }
}

/// Decode raw CP932/GBK bytes into UTF-8 losslessly for UI/diagnostics.
pub fn decode_script_bytes_to_utf8(raw: &[u8], enc: TextEncoding) -> String {
    let codec = match enc {
        TextEncoding::Cp932 => SHIFT_JIS,
        TextEncoding::Gbk => GBK,
    };
    codec.decode(raw).0.into_owned()
}
