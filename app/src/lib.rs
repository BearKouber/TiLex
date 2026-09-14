//! TiLex。分层见 design §1.2：ui → logic → service → platform，由 tests/source_rules.rs 检查。
//! 做成 lib + bin 是为了让 tests/ 下的集成测试（unicode_paths.rs）能调到配置和日志。

pub mod error;
pub mod logger;
pub mod logic;
pub mod platform;
pub mod ui;

/// slint 生成的代码里有 unwrap / expect / todo!，放进单独模块豁免。
/// 这是全项目唯一一处模块级放开 R-1 lint 的地方（source_rules.rs 只允许这里）。
pub mod slint_ui {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::todo,
        reason = "slint 生成的代码，不是手写代码"
    )]
    slint::include_modules!();
}
