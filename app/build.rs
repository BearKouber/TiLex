use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() -> Result<(), slint_build::CompileError> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=icon.ico");
        println!("cargo:rerun-if-changed=app.rc");
        match embed_resource::compile("app.rc", embed_resource::NONE) {
            embed_resource::CompilationResult::Ok
            | embed_resource::CompilationResult::NotWindows => {}
            err => {
                println!("cargo:warning=Failed to embed Windows resource app.rc: {err}");
            }
        }
    }

    // 翻译不带上下文：.po 里只有 msgid / msgstr，source_rules.rs 按 msgid 对账（design §2.9）。
    let config = CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("ui/i18n")
        .with_default_translation_context(DefaultTranslationContext::None);
    // slint-build 只追踪 .slint 的依赖，.po 改了也要重编。
    println!("cargo:rerun-if-changed=ui/i18n");
    slint_build::compile_with_config("ui/app.slint", config)
}
