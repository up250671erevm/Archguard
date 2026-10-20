//! `archguard-tray`: system tray applet (StatusNotifierItem).
//!
//! Usage: `archguard-tray [--no-retry] [--autostart enable|disable] [--version]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("archguard-tray {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "archguard-tray {}\n\nUsage: archguard-tray [--no-retry] [--autostart enable|disable]\n\n  \
             --no-retry          exit instead of retrying when no D-Bus session is reachable\n  \
             --autostart MODE    enable/disable ~/.config/autostart/archguard-tray.desktop",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--autostart") {
        let enable = match args.get(i + 1).map(String::as_str) {
            Some("enable") => true,
            Some("disable") => false,
            _ => {
                eprintln!("archguard-tray: --autostart needs `enable` or `disable`");
                std::process::exit(2);
            }
        };
        match archguard_tray::gui::set_autostart(enable) {
            Ok(p) => {
                println!(
                    "archguard-tray: autostart {} ({})",
                    if enable { "enabled" } else { "disabled" },
                    p.display()
                );
                return;
            }
            Err(e) => {
                eprintln!("archguard-tray: error: {e:#}");
                std::process::exit(2);
            }
        }
    }
    let opts = archguard_tray::gui::Options {
        retry: !args.iter().any(|a| a == "--no-retry"),
    };
    if let Err(e) = archguard_tray::gui::run(opts) {
        eprintln!("archguard-tray: error: {e:#}");
        std::process::exit(2);
    }
}
