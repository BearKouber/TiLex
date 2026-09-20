// release 不带控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use tilex::error::Error;
use tilex::logic::{config, hotkey, wordbook};
use tilex::{logger, platform, ui};

fn main() {
    if let Err(e) = run() {
        // 日志还没初始化时这条会丢；release 没有控制台，别的地方也看不到。
        log::error!("Main: fatal: {e}");
        std::process::exit(1);
    }
}

/// 启动顺序。除此之外不写逻辑（design §1.1）。
fn run() -> Result<(), Error> {
    // 托盘"重启"拉起的新进程带这个参数：旧进程还在退出途中，多等它一会儿。
    let restarting = std::env::args_os().any(|a| a == platform::RESTART_FLAG);
    let wait = if restarting {
        Duration::from_secs(10)
    } else {
        Duration::ZERO
    };
    // 先抢单实例再碰日志：第二个实例只通知老实例，不写日志、不建托盘。
    if !platform::claim_single_instance(wait)? {
        return Ok(());
    }

    let data = platform::data_dir()?;
    logger::init(&data)?;
    // 带上生效的日志级别：排查划词那类只有 debug 行的问题时，一眼能看出 TILEX_LOG 到底吃上没有
    // （用户交上来的第一份日志就是因为环境变量没生效，全是 INFO，白跑一轮）。
    log::info!(
        "============== Start TiLex {} (log={}) ==============",
        env!("CARGO_PKG_VERSION"),
        log::max_level()
    );
    let startup = config::init(&data)?;
    if let Err(e) = wordbook::init(&data) {
        log::error!("Main: wordbook init failed: {e}");
    }

    ui::select_backend()?;
    // 托盘必须先建：它是第一个 Slint 组件，建完才有事件循环和翻译上下文。
    let tray = ui::tray::create()?;
    ui::apply_language(&config::snapshot().general.language);
    hotkey::init(ui::overlay::start);
    let hotkey_str = config::snapshot().screenshot.hotkey;
    if !hotkey_str.is_empty()
        && let Err(e) = hotkey::apply(&hotkey_str)
    {
        log::warn!("Main: hotkey register failed: {e}");
    }
    // 划词浮标启动时就建好、一直不销毁（D12）。
    let pop_button = ui::pop_button::create()?;
    if pop_button.is_some() {
        ui::pop_result::create()?;
    }
    // 截图遮罩也是启动时建好、常驻（design §1.4）：每次现建会走 ShowWindow，
    // Windows 给它播 200ms 的开窗动画。
    if let Err(e) = ui::overlay::create() {
        log::error!("Main: create screenshot overlay failed: {e}");
    }
    // 听不到第二实例的通知只是"再开 exe 不弹设置"，不值得让整个程序起不来（socket bind 可能失败）。
    if let Err(e) = platform::listen_activation(|| {
        if let Err(e) = slint::invoke_from_event_loop(ui::settings::open) {
            log::warn!("Main: open settings from second instance failed: {e}");
        }
    }) {
        log::error!("Main: listen for second instance failed: {e}");
    }
    // 设置窗口要在托盘和浮标之后建（它们先占住事件循环和翻译上下文）。
    match startup {
        config::Startup::Normal => {}
        config::Startup::FirstRun => {
            log::info!("Main: first run, opening settings");
            ui::settings::open();
        }
        config::Startup::Recovered(backup) => ui::settings::open_with_notice(&backup),
    }

    // 没有窗口时事件循环也不退出，只有托盘"退出"才结束（design §1.4）。
    slint::run_event_loop_until_quit()?;
    // 显式 drop：托盘图标在这里从通知区域移除，不留残影。
    drop(tray);
    log::info!("Main: exit");
    Ok(())
}
