//! Quick Note — sticky-note canvas with pages and Google Drive sync.

mod app;
mod autostart;
mod cli;
mod config;
#[cfg(debug_assertions)]
mod debug_shot;
mod dock;
mod drive;
mod export;
mod model;
mod signals;
mod storage;
mod tray;
mod ui;

use std::sync::Arc;

/// System fonts tried first (Ubuntu 24.04/26.04 ship these); egui's bundled font is the fallback.
const SYSTEM_FONTS: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/ubuntu/Ubuntu[wdth,wght].ttf",
    "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
];

fn main() -> eframe::Result {
    let args = cli::parse(std::env::args().skip(1)).unwrap_or_else(|e| fail(e));
    if args.help {
        print!("{}", cli::HELP);
        return Ok(());
    }
    if args.version {
        println!("quick-note {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.uninstall {
        let data = dirs::config_dir().map(|d| d.join(storage::APP_DIR));
        cli::uninstall(args.purge, data.as_deref()).unwrap_or_else(|e| fail(e));
        return Ok(());
    }

    let dir = storage::app_dir().unwrap_or_else(|e| fail(e));
    if args.drive_check {
        let cfg = config::Config::load(&dir).unwrap_or_else(|e| fail(e));
        match drive::check(&cfg.drive, &dir) {
            Ok(report) => print!("{report}"),
            Err(e) => fail(e),
        }
        return Ok(());
    }
    // `--window` opens a normal window for this run even when dock mode is on.
    let dock = config::Config::load(&dir)
        .map(|c| c.dock.enabled && !args.window)
        .unwrap_or(false);
    let options = native_options(dock);

    eframe::run_native(
        "Quick Note",
        options,
        Box::new(move |cc| {
            install_system_font(&cc.egui_ctx);
            Ok(Box::new(app::QuickNoteApp::new(cc, dir, dock)))
        }),
    )
}

fn fail(e: anyhow::Error) -> ! {
    eprintln!("quick-note: {e:#}");
    std::process::exit(1);
}

fn native_options(dock: bool) -> eframe::NativeOptions {
    let base = egui::ViewportBuilder::default()
        .with_title("Quick Note")
        .with_app_id("quick-note");
    if !dock {
        return eframe::NativeOptions {
            viewport: base
                .with_inner_size([1100.0, 720.0])
                .with_min_inner_size([520.0, 360.0]),
            ..Default::default()
        };
    }
    let handle = config::DockConfig::default().handle_size;
    eframe::NativeOptions {
        viewport: base
            .with_inner_size([handle, handle])
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            // Full-size, mostly see-through window; see dock.rs.
            .with_transparent(true),
        // Wayland forbids self-positioning and always-on-top; XWayland allows both.
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        })),
        ..Default::default()
    }
}

fn install_system_font(ctx: &egui::Context) {
    let Some(bytes) = SYSTEM_FONTS.iter().find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("system".into(), Arc::new(egui::FontData::from_owned(bytes)));
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "system".into());
    }
    ctx.set_fonts(fonts);
}
