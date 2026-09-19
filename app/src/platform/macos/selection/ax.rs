//! Accessibility (AX) API 封装。
//! 用于检测系统辅助功能权限，以及读取前台焦点控件选中的文本。

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementCopyAttributeNames(element: *mut c_void, names: *mut *const c_void) -> i32;
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

/// 探测当前焦点 UI 元素是否具有 `AXSelectedText` 属性。
///
/// 判据与 fallback 策略（对应 Windows 的 TextPattern 契约）：
/// - 有 `AXSelectedText` 属性：说明程序有能力报告选区；若选区文本为空则是真的没选中，禁止模拟复制（返回 `true`）。
/// - 没有该属性，或者根本拿不到焦点元素：说明程序为自绘文本（如终端、代码编辑器），允许模拟复制（返回 `false`）。
/// - 任何一步失败（拿不到 names、调用出错、空指针等）：一律当成「有这个属性」（拿不准一律报有），禁止模拟复制（返回 `true`）。
pub fn has_selected_text_attribute() -> bool {
    // SAFETY: AXUIElementCreateSystemWide 返回系统级 AXUIElementRef（CFType），失败时返回空指针。
    let sys_ptr = unsafe { AXUIElementCreateSystemWide() };
    let Some(sys_non_null) = NonNull::new(sys_ptr.cast::<CFType>()) else {
        return true; // 异常：拿不准一律报有（禁止复制）
    };
    // SAFETY: sys_non_null 非空且拥有 +1 引用计数，由 CFRetained 接管并在 drop 时释放。
    let sys: CFRetained<CFType> = unsafe { CFRetained::from_raw(sys_non_null) };

    let attr_focused = CFString::from_str("AXFocusedUIElement");
    let mut focused_ptr: *const c_void = core::ptr::null();
    // SAFETY: sys 为有效的系统元素指针，attr_focused 为有效 CFString，focused_ptr 用于接收对象指针。
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            CFRetained::as_ptr(&sys).as_ptr().cast(),
            CFRetained::as_ptr(&attr_focused).as_ptr().cast(),
            &mut focused_ptr,
        )
    };
    if status != 0 || focused_ptr.is_null() {
        // 根本拿不到焦点元素：程序自绘文本，允许模拟复制
        return false;
    }
    let Some(focused_non_null) = NonNull::new((focused_ptr as *mut c_void).cast::<CFType>()) else {
        return false;
    };
    // SAFETY: AXUIElementCopyAttributeValue 成功时返回 +1 引用计数的 CFType，由 CFRetained 接管并在 drop 时释放。
    let focused: CFRetained<CFType> = unsafe { CFRetained::from_raw(focused_non_null) };

    let mut names_ptr: *const c_void = core::ptr::null();
    // SAFETY: focused 为有效的焦点元素指针，names_ptr 用于接收生成的属性名数组指针。
    let status = unsafe {
        AXUIElementCopyAttributeNames(CFRetained::as_ptr(&focused).as_ptr().cast(), &mut names_ptr)
    };
    if status != 0 || names_ptr.is_null() {
        // 拿不到属性列表或调用出错：拿不准一律报有（禁止复制）
        return true;
    }
    let Some(names_non_null) = NonNull::new((names_ptr as *mut c_void).cast::<CFArray>()) else {
        return true;
    };
    // SAFETY: AXUIElementCopyAttributeNames 成功时返回 +1 引用计数的 CFArrayRef，由 CFRetained 接管并在 drop 时释放。
    let names: CFRetained<CFArray> = unsafe { CFRetained::from_raw(names_non_null) };

    let count = names.count();
    for i in 0..count {
        // SAFETY: i 处于 0..count 有效索引范围内。
        let val_ptr = unsafe { names.value_at_index(i) };
        let Some(val_non_null) = NonNull::new((val_ptr as *mut c_void).cast::<CFType>()) else {
            continue;
        };
        // SAFETY: val_non_null 属于 names 持有的元素指针，在 names 生命周期内有效。
        let item_ref: &CFType = unsafe { val_non_null.as_ref() };
        if let Some(item_str) = item_ref.downcast_ref::<CFString>()
            && item_str.to_string() == "AXSelectedText"
        {
            return true;
        }
    }

    false
}
