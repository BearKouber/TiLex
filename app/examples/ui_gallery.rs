//! 组件展示页，只用于开发手测（父任务 ui.md §13 A3）：`cargo run --example ui_gallery`。

use slint::ComponentHandle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tilex::ui::select_backend()?;
    tilex::slint_ui::Gallery::new()?.run()?;
    Ok(())
}
