//! Metin2 game server binary
//!
//! Main entry point for the game server. Handles:
//! - TOML configuration parsing
//! - Logging initialization
//! - TCP listener setup
//! - Signal handling (SIGTERM, SIGINT)
//! - Graceful shutdown sequence

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use common::config::{parse_game_config, GameConfig};
use common::logging::{init_from_env, init_logging, LogConfig};
use game_server::client_session::{ClientDispatchOutcome, ClientSession};
use game_server::game_loop::{spawn_game_loop, GameLoopConfig, GameLoopHandle};
use game_server::game_loop_messages::GameLoopTerminal;
use game_server::ready_gate::ReadyGate;
use game_server::ServerState;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Default configuration file path (TOML)
const DEFAULT_CONFIG_PATH: &str = "game.toml";

/// Parse command line arguments
struct CliArgs {
    /// Path to configuration file
    config_path: PathBuf,
    /// Override port from command line
    port: Option<u16>,
    /// Verbose logging to stdout
    verbose: bool,
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
            port: None,
            verbose: false,
        }
    }
}

impl CliArgs {
    fn parse() -> Self {
        let mut args = Self::default();
        let mut iter = std::env::args().skip(1);

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-c" | "--config" => {
                    if let Some(path) = iter.next() {
                        args.config_path = PathBuf::from(path);
                    }
                }
                "-p" | "--port" => {
                    if let Some(port_str) = iter.next() {
                        if let Ok(port) = port_str.parse::<u16>() {
                            if port > 1024 {
                                args.port = Some(port);
                            } else {
                                eprintln!("Port must be greater than 1024");
                            }
                        }
                    }
                }
                "-v" | "--verbose" => {
                    args.verbose = true;
                }
                "-h" | "--help" => {
                    Self::print_usage();
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("Unknown argument: {arg}");
                    Self::print_usage();
                }
            }
        }

        args
    }

    fn print_usage() {
        println!(
            "Metin2 Game Server\n\
             \n\
             Usage: game-server [OPTIONS]\n\
             \n\
             Options:\n\
             -c, --config <path>  Path to TOML configuration file (default: game.toml)\n\
             -p, --port <port>    Override listen port (must be > 1024)\n\
             -v, --verbose        Enable verbose logging to stdout\n\
             -h, --help           Show this help message"
        );
    }
}

/// Initialize the server with configuration
fn initialize_server(args: &CliArgs) -> Result<GameConfig, Box<dyn std::error::Error>> {
    // Parse configuration file
    info!("Loading configuration from: {}", args.config_path.display());
    let mut config = parse_game_config(&args.config_path)
        .map_err(|error| format!("Failed to parse config: {error}"))?;

    // Override port from command line if specified
    if let Some(port) = args.port {
        info!("Overriding port from command line: {}", port);
        config.mother_port = port;
    }

    // Initialize logging
    if args.verbose {
        init_from_env();
    } else {
        let log_config = LogConfig::default();
        init_logging(&log_config);
    }

    info!("Configuration loaded successfully");
    info!("  Hostname: {}", config.hostname);
    info!("  Channel: {}", config.channel);
    info!("  Port: {}", config.mother_port);
    info!("  P2P Port: {}", config.p2p_port);

    Ok(config)
}

/// Set up TCP listener on the configured port
async fn setup_tcp_listener(port: u16) -> Result<TcpListener, Box<dyn std::error::Error>> {
    let bind_addr = format!("0.0.0.0:{port}");
    info!("Binding TCP listener to: {}", bind_addr);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|error| format!("Failed to bind TCP listener to {bind_addr}: {error}"))?;

    info!("TCP listener bound successfully");
    Ok(listener)
}

/// Handle incoming fixed-frame client connections.
///
/// The loop records only the source-verified keepalive and pong boundaries.
/// Variable packets, descriptor callbacks, and TEA remain explicit unsupported
/// errors; this handler does not claim to implement gameplay or encryption.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: std::net::SocketAddr,
    shutdown_tx: &broadcast::Sender<()>,
) {
    info!(%addr, "New client connection");

    let mut session = ClientSession::new(stream);
    let mut shutdown_rx = shutdown_tx.subscribe();

    loop {
        tokio::select! {
            packet = session.read_dispatch() => {
                match packet {
                    Ok(Some(ClientDispatchOutcome::KeepAlive { .. })) => {
                        info!(%addr, "Received client keepalive");
                    }
                    Ok(Some(ClientDispatchOutcome::Pong { .. })) => {
                        info!(%addr, "Received client pong");
                    }
                    Ok(None) => {
                        info!(%addr, "Client connection closed cleanly");
                        break;
                    }
                    Err(error) => {
                        warn!(%addr, %error, "Client session stopped");
                        break;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!(%addr, "Client session stopping for shutdown");
                break;
            }
        }
    }
}

fn begin_shutdown(state: &ServerState, shutdown_tx: &broadcast::Sender<()>) {
    if state.is_shutting_down() {
        return;
    }
    info!("Initiating graceful shutdown sequence");
    state.initiate_shutdown();
    drop(shutdown_tx.send(()));
}

/// The OS shutdown-signal handlers, already installed.
///
/// Installing a handler is the point of this type. `tokio::signal::unix::signal`
/// registers the handler when the stream is *created*, so [`arm_shutdown_signal`]
/// must run before anything observable, such as a bound listener, can let a
/// caller believe the process is ready.
struct ShutdownSignal {
    /// SIGTERM. Only the unix build has a real one.
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    /// SIGINT, registered the same way rather than through `ctrl_c`.
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
}

/// Install the shutdown-signal handlers without waiting for a signal.
///
/// # Errors
///
/// Returns an error if the runtime refuses to register a handler. This is fatal
/// on purpose: a process that cannot handle SIGTERM cannot shut down cleanly.
fn arm_shutdown_signal() -> Result<ShutdownSignal, Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        let terminate = signal::unix::signal(signal::unix::SignalKind::terminate())
            .map_err(|error| format!("Failed to install SIGTERM handler: {error}"))?;
        let interrupt = signal::unix::signal(signal::unix::SignalKind::interrupt())
            .map_err(|error| format!("Failed to install SIGINT handler: {error}"))?;
        Ok(ShutdownSignal {
            terminate,
            interrupt,
        })
    }

    #[cfg(not(unix))]
    {
        Ok(ShutdownSignal {})
    }
}

impl ShutdownSignal {
    /// Wait for SIGTERM or SIGINT.
    ///
    /// # Errors
    ///
    /// Returns an error if the signal stream fails.
    async fn wait(self) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(unix)]
        {
            let mut terminate = self.terminate;
            let mut interrupt = self.interrupt;
            tokio::select! {
                _ = terminate.recv() => info!("Received SIGTERM"),
                _ = interrupt.recv() => info!("Received SIGINT (Ctrl+C)"),
            }
            Ok(())
        }

        #[cfg(not(unix))]
        {
            signal::ctrl_c()
                .await
                .map_err(|error| format!("Failed to install Ctrl+C handler: {error}"))?;
            info!("Received SIGINT (Ctrl+C)");
            Ok(())
        }
    }
}

struct ServerContext {
    state: Arc<ServerState>,
    shutdown_tx: broadcast::Sender<()>,
}

enum AcceptLoopExit {
    Signal,
    GameLoop(GameLoopTerminal),
}

async fn run_accept_loop(
    listener: &TcpListener,
    context: &ServerContext,
    game_loop: &mut GameLoopHandle,
    shutdown_signal: ShutdownSignal,
    ready_gate: &ReadyGate,
) -> Result<AcceptLoopExit, Box<dyn std::error::Error>> {
    // Boxed and pinned once, not per `select!` iteration: the wait future owns
    // the signal streams, so recreating it each pass would drop them and lose a
    // signal that arrived between accepts.
    let mut shutdown = Box::pin(shutdown_signal.wait());

    loop {
        tokio::select! {
            signal_result = &mut shutdown => {
                signal_result?;
                return Ok(AcceptLoopExit::Signal);
            }
            terminal = game_loop.wait_for_terminal() => {
                return Ok(AcceptLoopExit::GameLoop(terminal?));
            }
            result = listener.accept() => match result {
                Ok((stream, addr)) if !context.state.should_accept_connections() => {
                    warn!(%addr, "Rejecting new connection while shutting down");
                    drop(stream);
                }
                // The ready gate is checked before the handler is spawned, not
                // inside it, so an unready server does not allocate a
                // per-connection task for every client that arrives.
                Ok((stream, addr)) if !ready_gate.admit() => {
                    warn!(
                        %addr,
                        refused = ready_gate.refused(),
                        "Refusing client connection before startup has finished"
                    );
                    drop(stream);
                }
                Ok((stream, addr)) => {
                    let connection_shutdown = context.shutdown_tx.clone();
                    tokio::spawn(async move {
                        handle_connection(stream, addr, &connection_shutdown).await;
                    });
                }
                Err(error) => error!(%error, "Failed to accept connection"),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();
    let config = initialize_server(&args)?;
    let state = Arc::new(ServerState::new());
    let (shutdown_tx, _) = broadcast::channel::<()>(16);

    // Arm the shutdown signal BEFORE the listener exists.
    //
    // `tokio::signal::unix::signal` installs its handler when the stream is
    // created, and the kernel keeps the default disposition until then. If the
    // listener were bound first, a process could accept a connection (and a
    // test could observe readiness) in the window before the handler exists, and
    // a SIGTERM in that window would kill the process outright instead of
    // shutting it down cleanly. Arming first makes "the port is open" imply
    // "SIGTERM is handled".
    let shutdown_signal = arm_shutdown_signal()?;

    let listener = setup_tcp_listener(config.mother_port).await?;
    info!("Server initialized successfully");
    info!("Listening for connections on port {}", config.mother_port);

    // The client port opens before the world is ready, as legacy's does
    // (`main.cpp:671`). The ready gate keeps that ordering from admitting
    // clients onto a world whose Game data is not loaded, which legacy does not
    // do. Nothing is loaded yet, so the gate opens once the game loop runs.
    let ready_gate = ReadyGate::closed();

    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), |_| {})?;
    let controller = game_loop.controller();
    info!(thread_id = ?game_loop.thread_id(), "Dedicated game loop started");
    ready_gate.open();

    let context = ServerContext { state, shutdown_tx };
    let exit = run_accept_loop(
        &listener,
        &context,
        &mut game_loop,
        shutdown_signal,
        &ready_gate,
    )
    .await;
    ready_gate.close();
    begin_shutdown(&context.state, &context.shutdown_tx);
    let exit = match exit {
        Ok(exit) => exit,
        Err(run_error) => {
            if let Err(error) = controller.request_stop().await {
                warn!(%error, "Game loop stop command was not accepted during error cleanup");
            }
            match game_loop.join().await {
                Ok(terminal) => {
                    info!(?terminal, "Game loop error cleanup acknowledged");
                    info!("Game loop thread joined during error cleanup");
                }
                Err(error) => error!(%error, "Game loop error cleanup failed"),
            }
            return Err(run_error);
        }
    };
    let terminal = match exit {
        AcceptLoopExit::Signal => {
            if let Err(error) = controller.request_stop().await {
                warn!(%error, "Game loop stop command was not accepted");
            }
            game_loop.wait_for_terminal().await?
        }
        AcceptLoopExit::GameLoop(terminal) => terminal,
    };
    info!(?terminal, "Game loop stop acknowledged");
    let terminal = game_loop.join().await?;
    info!("Game loop thread joined");
    info!("<shutdown> Shutdown complete");
    info!("Server shutdown complete");

    match terminal {
        GameLoopTerminal::Stopped(_) => Ok(()),
        GameLoopTerminal::Failed { reason, .. } => {
            Err(format!("Game loop terminated with failure: {reason:?}").into())
        }
    }
}
