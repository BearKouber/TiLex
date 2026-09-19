//! macOS 系统剪贴板（NSPasteboard）快照、恢复、文本读取与序列号。
//! 契约对应 platform-windows.md §4 及 Windows 的 force_copy.rs。
//!
//! 关键行为：
//! - `Snapshot::take()`: 保存所有 item 的所有 type 和二进制数据。超过 32MB 时返回 None 取消复制。
//! - `Snapshot::restore()`: 在恢复前比对当前 changeCount 是否等于我们复制后读到的序号（`should_restore`）。
//!   不等则放弃恢复，保留用户或其它程序写入的新内容。
//! - 写入 `org.nspasteboard.TransientType` 标记（社区约定，多数剪贴板管理器会跳过历史记录）。
//! - `clearContents()` 会使 `changeCount` 递增，因此模拟复制的结果按 AX 结果走 `complete_uia`，绝不走 `Ev::Clip`。

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};

/// 剪贴板快照最大字节数上限（32MB），超过则不执行模拟复制，保护大图等占用。
const MAX_BYTES: usize = 32 << 20;

/// 单个剪贴板项（NSPasteboardItem）的快照内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSnapshot {
    pub types: Vec<(String, Vec<u8>)>,
}

/// 剪贴板整体快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub items: Vec<ItemSnapshot>,
}

impl Snapshot {
    /// 截取当前系统剪贴板快照。
    /// 若剪贴板数据超过 32MB 则返回 None。空剪贴板也是有效快照（恢复时清空）。
    pub fn take() -> Option<Self> {
        let pboard = NSPasteboard::generalPasteboard();
        let Some(pb_items) = pboard.pasteboardItems() else {
            return Some(Snapshot { items: Vec::new() });
        };
        let count = pb_items.count();
        let mut items = Vec::with_capacity(count);
        let mut total_bytes = 0usize;

        for i in 0..count {
            let item = pb_items.objectAtIndex(i);
            let types = item.types();
            let type_count = types.count();
            let mut item_types = Vec::with_capacity(type_count);

            for j in 0..type_count {
                let type_name = types.objectAtIndex(j);
                if let Some(data) = item.dataForType(&type_name) {
                    let len = data.len();
                    total_bytes += len;
                    if total_bytes > MAX_BYTES {
                        log::info!("PopButton: force copy skipped, pasteboard over 32MB");
                        return None;
                    }
                    item_types.push((type_name.to_string(), data.to_vec()));
                }
            }
            items.push(ItemSnapshot { types: item_types });
        }

        Some(Snapshot { items })
    }

    /// 恢复快照内容。
    /// 仅在当前 changeCount 与预期序号一致时才执行恢复（用户未产生新复制）。
    pub fn restore(&self, copied: u32) -> bool {
        let pboard = NSPasteboard::generalPasteboard();
        let current_seq = current_change_count();
        if !should_restore(current_seq, copied) {
            return false;
        }

        // clearContents 使 changeCount +1
        pboard.clearContents();
        if self.items.is_empty() {
            return true;
        }

        let mut restored_items: Vec<Retained<NSPasteboardItem>> =
            Vec::with_capacity(self.items.len());
        for item_snap in &self.items {
            let item = NSPasteboardItem::new();
            for (type_name, bytes) in &item_snap.types {
                let t = NSString::from_str(type_name);
                let d = NSData::with_bytes(bytes);
                let _ = item.setData_forType(&d, &t); // ignore: 恢复个别类型失败不影响其余格式写入
            }
            restored_items.push(item);
        }

        // 社区约定：写入 org.nspasteboard.TransientType 空标记。
        // 多数剪贴板管理器（如 Paste, Maccy, Alfred 等）会识别并忽略带有此类型的剪贴板更新，
        // 避免将恢复的旧内容二次录入历史。注意这是社区约定而非 Apple 官方系统 API，不保证所有管理器均认。
        let transient_type = NSString::from_str("org.nspasteboard.TransientType");
        let empty_data = NSData::with_bytes(&[]);
        if let Some(first_item) = restored_items.first() {
            let _ = first_item.setData_forType(&empty_data, &transient_type); // ignore: transient 标记写入失败不影响内容恢复
        }

        let refs: Vec<&ProtocolObject<dyn NSPasteboardWriting>> = restored_items
            .iter()
            .map(|it| ProtocolObject::from_ref(&**it))
            .collect();
        let array = NSArray::from_slice(&refs);
        pboard.writeObjects(&array)
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }
}

/// 只在我们自己的复制之上恢复。任何别的变化都是别人更新的内容。
pub fn should_restore(now: u32, copied: u32) -> bool {
    now == copied
}

/// 读取剪贴板中的纯文本（去掉首尾空白）。
/// 若无文本类型（如仅复制了文件或图像）则返回 None。
pub fn read_text() -> Option<String> {
    let pboard = NSPasteboard::generalPasteboard();
    // SAFETY: NSPasteboardTypeString 是 AppKit 导出的 extern static 常量。
    let type_string = unsafe { NSPasteboardTypeString };
    let ns_str = pboard.stringForType(type_string)?;
    Some(ns_str.to_string().trim().to_string())
}

/// 获取当前系统剪贴板序号（changeCount 截断为 u32）。
pub fn current_change_count() -> u32 {
    NSPasteboard::generalPasteboard().changeCount() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_restore_only_when_equal() {
        assert!(should_restore(10, 10));
        assert!(!should_restore(10, 11));
        assert!(!should_restore(11, 10));
        assert!(!should_restore(0, 1));
    }

    #[test]
    #[ignore = "会动本机真实剪贴板，仅限真人手测或 CI 指定环境跑"]
    fn test_pasteboard_snapshot_restore_round_trip() {
        let pboard = NSPasteboard::generalPasteboard();
        let before_seq = current_change_count();
        let snapshot = Snapshot::take();
        assert!(snapshot.is_some());
        let _ = pboard.clearContents(); // ignore: 测试中清理剪贴板
        let copied_seq = current_change_count();
        if let Some(s) = snapshot {
            assert!(s.restore(copied_seq));
            assert_ne!(current_change_count(), before_seq);
        }
    }
}
