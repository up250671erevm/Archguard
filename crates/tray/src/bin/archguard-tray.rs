//! `archguard-tray`: system tray applet (StatusNotifierItem).

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("archguard-tray {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = archguard_tray::gui::run() {
        eprintln!("archguard-tray: error: {e:#}");
        std::process::exit(2);
    }
}
