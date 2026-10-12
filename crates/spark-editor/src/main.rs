//! Spark Editor.
//!
//! ```text
//! spark-editor               welcome screen: recent projects, new game, open folder
//! spark-editor path/to/game  opens the game (usually started as `spark editor .`)
//! ```
//!
//! Architecture (see also `crates/spark-editor/README.md`):
//!
//! - [`app`]        screens (welcome / workspace), settings, global window state
//! - [`welcome`]    project manager: recent projects, new project, open, settings
//! - [`workspace`]  an open game: dockable panels (explorer, code editor, console, problems, debugger, scene,
//!                  viewport)
//! - [`logcap`]     the logger; copies engine messages of the in-editor game to the console
//! - [`process`]    child processes: the game (`spark --devtools`), `spark check`, `spark build`
//! - devtools       the live link to the running game (crate `spark-devtools`)
//! - [`i18n`]       every UI string, `locales/*.toml`
//!
//! Run (F5) starts the game in its own process, exactly as players get it. The viewport panel also
//! links the engine: it draws the game on the editor's GPU device (fly camera, Play mode in the panel).
#![cfg_attr(all(feature = "gui", windows), windows_subsystem = "windows")]

mod app;
mod i18n;
mod logcap;
mod os;
mod process;
mod settings;
mod syntax;
mod theme;
mod toasts;
mod welcome;
mod workspace;

use std::path::PathBuf;

use eframe::egui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> eframe::Result {
    logcap::init("info,wgpu_core=warn,wgpu_hal=error,naga=warn,egui_wgpu=warn");

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Spark Editor {VERSION}\n\nusage: spark-editor [path/to/game]\n       spark editor [path/to/game]");
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("spark-editor {VERSION}");
        return Ok(());
    }
    let target = args.into_iter().find(|a| !a.starts_with("--")).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Spark Editor")
            .with_app_id("spark-editor")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([820.0, 520.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native("Spark Editor", options, Box::new(move |cc| Ok(Box::new(app::EditorApp::new(cc, target)))))
}
