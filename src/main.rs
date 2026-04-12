use rairstream::app::SessionCoordinator;
use rairstream::config::AppConfig;
use rairstream::discovery::MdnsDiscoveryService;
use rairstream::ui::run_tray_app;

fn main() {
    let config = AppConfig::default();
    let coordinator = SessionCoordinator::new(MdnsDiscoveryService::default());

    if let Err(error) = run_tray_app(coordinator, config) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
