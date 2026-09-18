use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use threadmail::config::Config;
use threadmail::imap_source::ImapSource;
use threadmail::smtp_sink::SmtpSink;
use threadmail::sqlite_store::SqliteStore;
use threadmail::web::{AppState, router, spawn_outgoing_comment_worker};
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

async fn serve(config_path: &str) {
    let config = Config::load(config_path).unwrap_or_else(|err| {
        tracing::error!(%err, path = config_path, "could not load config");
        std::process::exit(1);
    });

    let source = Arc::new(ImapSource::new(&config.imap).unwrap_or_else(|err| {
        tracing::error!(%err, "could not build the IMAP source");
        std::process::exit(1);
    }));
    let sink = Arc::new(SmtpSink::new(&config.smtp).unwrap_or_else(|err| {
        tracing::error!(%err, "could not build the SMTP transport");
        std::process::exit(1);
    }));
    let store = Arc::new(
        SqliteStore::open(&config.storage.path).unwrap_or_else(|err| {
            tracing::error!(%err, path = config.storage.path, "could not open the database");
            std::process::exit(1);
        }),
    );

    let bind_address = config.web.bind_address.clone();

    let state = AppState::new(source, sink, store.clone(), store, config);

    spawn_outgoing_comment_worker(state.clone());

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .unwrap_or_else(|err| {
            tracing::error!(%err, address = bind_address, "could not bind");
            std::process::exit(1);
        });
    tracing::info!(address = bind_address, "listening");
    axum::serve(listener, router(state))
        .await
        .unwrap_or_else(|err| {
            tracing::error!(%err, "server error");
        });
}
