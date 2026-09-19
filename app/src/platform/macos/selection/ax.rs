//! Accessibility (AX) API 封装。
//! 用于检测系统辅助功能权限，以及读取前台焦点控件选中的文本。

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
}

/// 检查进程是否已获得系统「辅助功能」授权。
/// 若 `prompt` 为 true 且尚未授权，系统会弹出授权对话框提示用户。
pub fn is_process_trusted(prompt: bool) -> bool {
    let key = CFString::from_str("AXTrustedCheckOptionPrompt");
    let val = CFBoolean::new(prompt);
    let dict = CFDictionary::from_slices(&[&*key], &[val]);
    // SAFETY: AXIsProcessTrustedWithOptions 接受 CFDictionaryRef 指针（或 NULL）；
    // dict 是由 CFDictionary::from_slices 创建的有效 CFDictionary，在调用期间生命周期完好。
    unsafe { AXIsProcessTrustedWithOptions(CFRetained::as_ptr(&dict).as_ptr().cast()) != 0 }
}

/// 通过 Accessibility API 从当前获得焦点的 UI 元素中读取选中的文字。
/// 任何一步失败（无焦点、无选区、类型不匹配、调用超时等）均返回空字符串。
/// 返回空字符串即代表读不到文字，不显示浮标。
///
/// 本函数无单元测试：跨进程读取 AX 焦点选区需要系统辅助功能授权与真实的系统焦点元素，只能由真人手测。
pub fn read_selected_text() -> String {
    // SAFETY: AXUIElementCreateSystemWide 无参数，返回系统范围的 AXUIElementRef（CFType），
    // 拥有 +1 引用计数，失败时返回空指针。
    let sys_ptr = unsafe { AXUIElementCreateSystemWide() };
    let Some(sys_non_null) = NonNull::new(sys_ptr.cast::<CFType>()) else {
        return String::new();
    };
    // SAFETY: sys_non_null 非空且由 AXUIElementCreateSystemWide 生成，拥有 +1 引用计数；
    // 由 CFRetained 接管，在超出作用域 drop 时调用 CFRelease 释放。
    let sys: CFRetained<CFType> = unsafe { CFRetained::from_raw(sys_non_null) };

    let attr_focused = CFString::from_str("AXFocusedUIElement");
    let mut focused_ptr: *const c_void = core::ptr::null();
    // SAFETY: sys 拥有有效的系统元素指针，attr_focused 是有效的 CFString；
    // focused_ptr 用于接收复制出的对象指针。
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            CFRetained::as_ptr(&sys).as_ptr().cast(),
            CFRetained::as_ptr(&attr_focused).as_ptr().cast(),
            &mut focused_ptr,
        )
    };
    if status != 0 {
        return String::new();
    }
    let Some(focused_non_null) = NonNull::new((focused_ptr as *mut c_void).cast::<CFType>()) else {
        return String::new();
    };
    // SAFETY: AXUIElementCopyAttributeValue 成功时返回 +1 引用计数的 CFType；
    // 由 CFRetained 接管，在超出作用域 drop 时调用 CFRelease 释放。
    let focused: CFRetained<CFType> = unsafe { CFRetained::from_raw(focused_non_null) };

    let attr_selected = CFString::from_str("AXSelectedText");
    let mut text_ptr: *const c_void = core::ptr::null();
    // SAFETY: focused 拥有有效的焦点元素指针，attr_selected 是有效的 CFString；
    // text_ptr 用于接收复制出的对象指针。
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            CFRetained::as_ptr(&focused).as_ptr().cast(),
            CFRetained::as_ptr(&attr_selected).as_ptr().cast(),
            &mut text_ptr,
        )
    };
    if status != 0 {
        return String::new();
    }
    let Some(text_non_null) = NonNull::new((text_ptr as *mut c_void).cast::<CFType>()) else {
        return String::new();
    };
    // SAFETY: AXUIElementCopyAttributeValue 成功时返回 +1 引用计数的 CFType；
    // 由 CFRetained 接管，在超出作用域 drop 时调用 CFRelease 释放。
    let text_type: CFRetained<CFType> = unsafe { CFRetained::from_raw(text_non_null) };

    // 某些程序在 AXSelectedText 属性上可能返回非字符串对象；
    // downcast 内部通过 CFGetTypeID 严格核对 CFStringGetTypeID()，
    // 若不匹配则返回 Err(text_type)，并在 drop 时正确释放原对象。
    let Ok(text_cf) = text_type.downcast::<CFString>() else {
        return String::new();
    };

    text_cf.to_string()
}
