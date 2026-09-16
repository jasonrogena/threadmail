use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use threadmail::config::{self, Config};
use threadmail::imap_source::ImapSource;
use threadmail::smtp_sink::SmtpSink;
use threadmail::web::{AppState, router};
use tracing::Level;

/// Static-site comments backed by a mailing list, not a database
#[derive(Debug, Parser)]
#[clap(name = "threadmail")]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// How verbose the log should be
    #[clap(short, long, default_value = "info")]
    log_level: LogLevel,
    /// Path to the configuration file to use
    #[clap(short, long, default_value = "/etc/threadmail/config.toml")]
    config_path: String,
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start the HTTP server
    Serve,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
#[clap(rename_all = "kebab-case")]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for Level {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Error => Level::ERROR,
            LogLevel::Warn => Level::WARN,
            LogLevel::Info => Level::INFO,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Trace => Level::TRACE,
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = Cli::parse();
    tracing_subscriber::fmt()
        .with_max_level(Level::from(args.log_level))
        .init();
    match args.command {
        Commands::Serve => serve(&args.config_path).await,
    }
}

fn required(from_file: &str, env_var: &str, file_field: &str) -> String {
    config::resolve_secret(from_file, std::env::var(env_var).ok()).unwrap_or_else(|| {
        tracing::error!("no value set: set {file_field} in the config file or {env_var}");
        std::process::exit(1);
    })
}

async fn serve(config_path: &str) {
    let config = Config::load(config_path).unwrap_or_else(|err| {
        tracing::error!(%err, path = config_path, "could not load config");
        std::process::exit(1);
    });

    let imap_username = required(
        &config.imap.username,
        "THREADMAIL_IMAP_USERNAME",
        "imap.username",
    );
    let imap_password = required(
        &config.imap.password,
        "THREADMAIL_IMAP_PASSWORD",
        "imap.password",
    );
    let smtp_username = required(
        &config.smtp.username,
        "THREADMAIL_SMTP_USERNAME",
        "smtp.username",
    );
    let smtp_password = required(
        &config.smtp.password,
        "THREADMAIL_SMTP_PASSWORD",
        "smtp.password",
    );

    let source = Arc::new(ImapSource::new(
        config.imap.host,
        config.imap.port,
        imap_username,
        imap_password,
    ));
    let sink = Arc::new(
        SmtpSink::new(
            &config.smtp.host,
            config.smtp.port,
            smtp_username,
            smtp_password,
        )
        .unwrap_or_else(|err| {
            tracing::error!(%err, "could not build the SMTP transport");
            std::process::exit(1);
        }),
    );

    let state = AppState::new(
        source,
        sink,
        config.list.bot_address,
        config.list.posting_address,
        config.list.relay_comments,
        config.list.show_email_link,
        config.limits.max_concurrent_searches,
        config.limits.max_concurrent_submits,
    );

    let listener = tokio::net::TcpListener::bind(&config.server.bind_address)
        .await
        .unwrap_or_else(|err| {
            tracing::error!(%err, address = config.server.bind_address, "could not bind");
            std::process::exit(1);
        });
    tracing::info!(address = config.server.bind_address, "listening");
    axum::serve(listener, router(state))
        .await
        .unwrap_or_else(|err| {
            tracing::error!(%err, "server error");
        });
}
