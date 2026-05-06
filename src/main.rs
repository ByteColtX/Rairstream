use rairstream::cli::{parse_cli, print_error, print_error_message, run_cli};
use tracing_subscriber::EnvFilter;

fn main() {
    let cli = match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(error) => {
            print_error(&error);
            std::process::exit(2);
        }
    };

    if let Err(error) = init_logging(cli.log_filter()) {
        print_error_message("Startup Error", &error);
        std::process::exit(2);
    }

    if let Err(error) = run_cli(cli) {
        print_error(&error);
        std::process::exit(1);
    }
}

fn init_logging(directive: &str) -> Result<(), String> {
    let env_filter = EnvFilter::try_new(directive)
        .map_err(|error| format!("invalid log filter `{directive}`: {error}"))?;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .try_init()
        .map_err(|error| format!("failed to initialize logging: {error}"))
}
