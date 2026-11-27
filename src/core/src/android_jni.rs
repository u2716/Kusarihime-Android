//! Android JNI bridge for `com.fujihime.euthanasia.EngineBridge`.
//! Thin wrapper over the stable C11 FFI in `ffi/mod.rs` / `fujihime_core.h`.
//! Only compiled on Android: `[target.'cfg(target_os = "android")'.dependencies] jni`.
use jni::objects::{JByteArray, JByteBuffer, JObject, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;
use std::ffi::CString;
use crate::ffi::*;

fn get_string(env: &mut JNIEnv, s: &JString) -> String {
    // jni 0.21: get_string -> JavaStr, to_str() -> Result<&str, Utf8Error>.
    // to_string_lossy() is the documented 0.21 way (see jni 0.21 migration guide).
    env.get_string(s).map(|v| v.to_string_lossy().into_owned()).unwrap_or_default()
}
fn lang(v: jint) -> FujihimeLanguageMode {
    if v == 1 { FujihimeLanguageMode::ChineseV1 } else { FujihimeLanguageMode::Japanese }
}
fn role(v: jint) -> FujihimeFontRole {
    match v { 1 => FujihimeFontRole::Mincho, 2 => FujihimeFontRole::Fallback, _ => FujihimeFontRole::Gothic }
}
fn action(v: jint) -> FujihimeSystemAction {
    match v {
        1 => FujihimeSystemAction::Load, 2 => FujihimeSystemAction::Config,
        3 => FujihimeSystemAction::Backlog, 4 => FujihimeSystemAction::Skip,
        5 => FujihimeSystemAction::Auto, 6 => FujihimeSystemAction::HideTextbox,
        7 => FujihimeSystemAction::Rollback, 8 => FujihimeSystemAction::Title,
        9 => FujihimeSystemAction::Exit, 10 => FujihimeSystemAction::Advance,
        11 => FujihimeSystemAction::ReplayVoice, _ => FujihimeSystemAction::Save,
    }
}
fn cstr(s: &str) -> CString { CString::new(s).unwrap_or_default() }
fn cstr_or_null(s: &str) -> (*const std::ffi::c_char, CString) {
    if s.is_empty() { (std::ptr::null(), cstr("")) } else { let c = cstr(s); (c.as_ptr(), c) }
}

#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeCreate<'local>(
    mut env: JNIEnv<'local>, _this: JObject<'local>,
    game_root: JString<'local>, save_dir: JString<'local>, override_dir: JString<'local>,
    orig_backup_dir: JString<'local>, language_mode: jint, repair: jboolean,
) -> jlong {
    let gr = get_string(&mut env, &game_root);
    let sd = get_string(&mut env, &save_dir);
    let od = get_string(&mut env, &override_dir);
    let ob = get_string(&mut env, &orig_backup_dir);
    let gr_c = cstr(&gr); let sd_c = cstr(&sd);
    let (od_p, _od_c) = cstr_or_null(&od);
    let (ob_p, _ob_c) = cstr_or_null(&ob);
    let cfg = FujihimeEngineConfig {
        game_root: gr_c.as_ptr(), save_dir: sd_c.as_ptr(), override_dir: od_p,
        orig_backup_dir: ob_p, language_mode: lang(language_mode),
        repair_chs_recall_labels: repair != 0,
    };
    fujihime_engine_create(&cfg) as jlong
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeDestroy<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong,
) {
    if handle != 0 { fujihime_engine_destroy(handle as *mut FujihimeEngine); }
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeLoadFont<'local>(
    env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, role_v: jint,
    data: JByteArray<'local>, face_index: jint,
) -> jint {
    if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; }
    let bytes = env.convert_byte_array(&data).unwrap_or_default();
    fujihime_engine_load_font(handle as *mut FujihimeEngine, role(role_v), bytes.as_ptr(), bytes.len(), face_index)
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeStartTitle<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_start_title(handle as *mut FujihimeEngine) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeStartGame<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, script_id: jint,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_start_game(handle as *mut FujihimeEngine, script_id as u16) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeTick<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, delta_ms: jint,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_tick(handle as *mut FujihimeEngine, delta_ms as u32) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativePointerEvent<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, kind: jint, x: jint, y: jint,
) -> jint {
    if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; }
    let k = match kind { 1 => FujihimePointerKind::Down, 2 => FujihimePointerKind::Up, 3 => FujihimePointerKind::RightUp, 4 => FujihimePointerKind::MiddleUp, 5 => FujihimePointerKind::WheelUp, 6 => FujihimePointerKind::WheelDown, _ => FujihimePointerKind::Move };
    fujihime_engine_pointer_event(handle as *mut FujihimeEngine, k, x, y)
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeKeyEvent<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, vk: jint, down: jboolean,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_key_event(handle as *mut FujihimeEngine, vk as u16, down != 0) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativePerformAction<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, act: jint,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_perform_action(handle as *mut FujihimeEngine, action(act)) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeSaveGlobal<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_save_global(handle as *mut FujihimeEngine) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeSaveSlot<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, slot: jint, y: jint, mo: jint, d: jint, h: jint, mi: jint,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_save_slot(handle as *mut FujihimeEngine, slot as u16, y as u16, mo as u16, d as u16, h as u16, mi as u16) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeLoadSlot<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, slot: jint,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_load_slot(handle as *mut FujihimeEngine, slot as u16) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeRollback<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_rollback(handle as *mut FujihimeEngine) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeSetLanguageMode<'local>(
    mut env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, mode: jint,
    orig_backup_dir: JString<'local>, repair: jboolean,
) -> jint {
    if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; }
    let ob = get_string(&mut env, &orig_backup_dir);
    let (ob_p, _c) = cstr_or_null(&ob);
    fujihime_engine_set_language_mode(handle as *mut FujihimeEngine, lang(mode), ob_p, repair != 0)
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeConfirmDialog<'local>(
    _env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, dialog_id: jlong, accepted: jboolean,
) -> jint { if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; } fujihime_engine_confirm_dialog_response(handle as *mut FujihimeEngine, dialog_id as u64, accepted != 0) }
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeImport2002CdSave<'local>(
    env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, data: JByteArray<'local>,
) -> jint {
    if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; }
    let bytes = env.convert_byte_array(&data).unwrap_or_default();
    fujihime_engine_import_2002_cd_save(handle as *mut FujihimeEngine, bytes.as_ptr(), bytes.len())
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativePollDialog<'local>(
    env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong,
) -> jstring {
    if handle == 0 { return std::ptr::null_mut(); }
    let mut dlg = FujihimePendingDialog::default();
    if !fujihime_engine_poll_pending_dialog(handle as *mut FujihimeEngine, &mut dlg) { return std::ptr::null_mut(); }
    let title = std::ffi::CStr::from_bytes_until_nul(&dlg.title_utf8).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    let msg = std::ffi::CStr::from_bytes_until_nul(&dlg.message_utf8).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    let s = format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", dlg.dialog_id, dlg.kind as i32, title, msg);
    env.new_string(s).map(|j| j.into_raw()).unwrap_or(std::ptr::null_mut())
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeGetState<'local>(
    env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong,
) -> jstring {
    if handle == 0 { return std::ptr::null_mut(); }
    let mut st = FujihimeEngineState::default();
    if fujihime_engine_get_state(handle as *mut FujihimeEngine, &mut st) != FUJIHIME_OK { return std::ptr::null_mut(); }
    let s = format!("{}|{}|{}|{}|{}|{}|{}|{}|{}", st.scene as i32, st.language_mode as i32, st.is_playing_video as i32, st.is_waiting_click as i32, st.is_waiting_select as i32, st.is_transitioning as i32, st.exit_requested as i32, st.current_script_id, st.current_pc);
    env.new_string(s).map(|j| j.into_raw()).unwrap_or(std::ptr::null_mut())
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeCopyPresent<'local>(
    env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, buf: JByteBuffer<'local>,
) -> jint {
    if handle == 0 { return FUJIHIME_ERR_INVALID_ARG; }
    let addr = match env.get_direct_buffer_address(&buf) { Ok(p) => p, Err(_) => return FUJIHIME_ERR_INVALID_ARG };
    let cap = env.get_direct_buffer_capacity(&buf).unwrap_or(0);
    if cap < FUJIHIME_PRESENT_BGRA8_LEN { return FUJIHIME_ERR_INVALID_ARG; }
    let slice = std::slice::from_raw_parts_mut(addr, FUJIHIME_PRESENT_BGRA8_LEN);
    fujihime_engine_copy_present_bgra8(handle as *mut FujihimeEngine, slice.as_mut_ptr(), slice.len())
}
#[no_mangle]
pub unsafe extern "system" fn Java_com_fujihime_euthanasia_EngineBridge_nativeRenderAudio<'local>(
    env: JNIEnv<'local>, _this: JObject<'local>, handle: jlong, buf: JByteBuffer<'local>, frames: jint,
) -> jint {
    if handle == 0 || frames <= 0 { return 0; }
    let addr = match env.get_direct_buffer_address(&buf) { Ok(p) => p as *mut f32, Err(_) => return 0 };
    let n = frames as usize;
    let slice = std::slice::from_raw_parts_mut(addr, n * 2);
    fujihime_engine_render_audio_f32(handle as *mut FujihimeEngine, slice.as_mut_ptr(), n) as jint
}
