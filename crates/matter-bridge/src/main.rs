//! `matter-bridge`: run the bridge, or parse an onboarding code.

use std::path::PathBuf;
use std::process::ExitCode;

use matter_bridge::Config;

const USAGE: &str = "\
usage:
  matter-bridge [--config FILE]     run the bridge
  matter-bridge parse CODE          say what a QR code or manual pairing code holds
  matter-bridge --version

FILE defaults to /etc/matter-bridge/config.toml; without it every default applies.";

enum Action {
    Run,
    Parse(String),
    Version,
    Help,
}

fn parse() -> Result<(Action, Option<PathBuf>), lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_env();
    let mut config = None;
    let mut action = Action::Run;
    let mut sub: Option<String> = None;
    let mut positional: Option<String> = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('c') | Long("config") => config = Some(parser.value()?.into()),
            Short('V') | Long("version") => action = Action::Version,
            Short('h') | Long("help") => action = Action::Help,
            Value(v) if sub.is_none() => sub = Some(v.string()?),
            Value(v) if positional.is_none() => positional = Some(v.string()?),
            _ => return Err(arg.unexpected()),
        }
    }
    if matches!(action, Action::Version | Action::Help) {
        return Ok((action, config));
    }
    action = match (sub.as_deref(), positional) {
        (None, None) => Action::Run,
        (Some("parse"), Some(code)) => Action::Parse(code),
        _ => Action::Help,
    };
    Ok((action, config))
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let (action, config_path) = match parse() {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("matter-bridge: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match action {
        Action::Version => {
            println!("matter-bridge {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Action::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Action::Parse(code) => match matter_host::Onboarding::parse(&code) {
            Ok(o) => {
                println!("{}", serde_json::to_string_pretty(&o).unwrap_or_default());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("matter-bridge: {e}");
                ExitCode::FAILURE
            }
        },
        Action::Run => {
            let default = PathBuf::from("/etc/matter-bridge/config.toml");
            let config = match config_path {
                Some(path) => Config::load(&path),
                None if default.exists() => Config::load(&default),
                None => Ok(Config::default()),
            };
            let config = match config {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("matter-bridge: {e}");
                    return ExitCode::from(2);
                }
            };
            let shutdown = async {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("stopping");
            };
            match matter_bridge::run(config, shutdown).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("matter-bridge: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
