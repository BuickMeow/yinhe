use std::ffi::{CStr, CString};

use super::*;

mod automation;
mod document;
mod edit_ffi;
mod events;
mod gesture;
mod media;
mod playback;
mod settings;

#[cfg(test)]
mod tests;

/// 把扁平 `[tick, velocity, ...]` 解析为点对；非有限值丢弃。
fn parse_velocity_stroke(flat: &[f64]) -> Vec<(f64, f32)> {
    flat.as_chunks::<2>()
        .0
        .iter()
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .map(|p| (p[0], p[1] as f32))
        .collect()
}

/// 解析扁平 `[old_tick, new_tick, value, ...]`；非有限值丢弃。
fn parse_automation_moves(flat: &[f64]) -> Vec<(u32, u32, f32)> {
    flat.as_chunks::<3>()
        .0
        .iter()
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .map(|p| {
            (
                p[0].max(0.0).round() as u32,
                p[1].max(0.0).round() as u32,
                (p[2] as f32).clamp(0.0, 1.0),
            )
        })
        .collect()
}

/// 读取可空 C 字符串；null/非法 UTF-8 当空串。
fn cstr_arg<'a>(p: *const std::ffi::c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
}

/// 释放本库返回的堆字符串。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_free_string(ptr: *mut std::ffi::c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(ptr));
    }
}

fn to_c_string(s: String) -> *mut std::ffi::c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
