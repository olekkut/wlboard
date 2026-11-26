//! `wlboard` — Lightweight, customizable native Wayland floating virtual keyboard & Panel Editor.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use wlboard_platform::{IpcCommand, try_send_ipc_command};
use wlboard_ui::{LaunchOptions, generate_preview_suite, run_app};

/// Lightweight, customizable native Wayland & X11 floating virtual keyboard.
#[derive(Debug, Parser)]
#[command(name = "wlboard", version, about, long_about = None)]
struct Cli {
    /// Toggle keyboard visibility on a running wlboard instance (or start if not running).
    #[arg(long)]
    toggle: bool,

    /// Show the keyboard on a running wlboard instance (or start visible).
    #[arg(long)]
    show: bool,

    /// Hide the keyboard on a running wlboard instance.
    #[arg(long)]
    hide: bool,

    /// Open the built-in Customize wlboard Settings & Presets window.
    #[arg(long)]
    settings: bool,

    /// Open the built-in 3-pane visual Panel Editor window.
    #[arg(long)]
    panel_editor: bool,

    /// Run as a Wayland Input Method (zwp_input_method_v1) for KDE Plasma Lockscreen & SDDM Login Screen.
    #[arg(long)]
    input_method: bool,

    /// Switch the active keyboard layout (en, pl, uk, ru, ar, or any XKB code) on a running instance.
    #[arg(long, value_name = "CODE")]
    layout: Option<String>,

    /// Switch the active 2D panel (standard, compact_tv, numpad_nav, shortcuts_media, or custom ID).
    #[arg(long, value_name = "ID")]
    panel: Option<String>,

    /// Quit a running wlboard instance cleanly.
    #[arg(long)]
    quit: bool,

    /// Render offline visual verification PNG previews into a directory and exit.
    #[arg(long, value_name = "DIR", hide = true)]
    render_previews: Option<PathBuf>,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("wlboard=info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    if let Some(dir) = cli.render_previews {
        return match generate_preview_suite(&dir) {
            Ok(files) => {
                for f in files {
                    println!("{f}");
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Failed generating previews: {err}");
                ExitCode::FAILURE
            }
        };
    }

    // Wayland Input Method mode (launched by KWin on Lockscreen / SDDM)
    // runs independently of any floating desktop instance.
    if !cli.input_method {
        if cli.quit {
            return forward_only(IpcCommand::Quit);
        }
        if cli.hide {
            return forward_only(IpcCommand::Hide);
        }
        if let Some(code) = cli.layout {
            return forward_only(IpcCommand::SetLayout(code));
        }
        if let Some(ref pid) = cli.panel
            && let Ok(true) = try_send_ipc_command(IpcCommand::SetPanel(pid.clone()))
        {
            return ExitCode::SUCCESS;
        }
        if cli.toggle {
            match try_send_ipc_command(IpcCommand::Toggle) {
                Ok(true) => return ExitCode::SUCCESS,
                Ok(false) => info!("No running wlboard instance found; launching fresh instance"),
                Err(err) => info!("IPC unavailable ({err}); launching fresh instance"),
            }
        } else if cli.show {
            if let Ok(true) = try_send_ipc_command(IpcCommand::Show) {
                return ExitCode::SUCCESS;
            }
        } else if cli.panel_editor {
            if let Ok(true) = try_send_ipc_command(IpcCommand::OpenPanelEditor) {
                return ExitCode::SUCCESS;
            }
        } else if cli.settings {
            if let Ok(true) = try_send_ipc_command(IpcCommand::OpenSettings) {
                return ExitCode::SUCCESS;
            }
        } else if let Ok(true) = try_send_ipc_command(IpcCommand::Show) {
            info!("wlboard is already running — brought keyboard overlay to front");
            return ExitCode::SUCCESS;
        }
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown));

    // Zero-overhead self-pipe for instant SIGINT/SIGTERM wakeup inside blocking epoll_pwait (0 wakeups/sec!).
    let signal_pipe = UnixStream::pair().ok().and_then(|(rx, tx)| {
        let tx_term = tx.try_clone().ok()?;
        let _ = signal_hook::low_level::pipe::register(signal_hook::consts::SIGINT, tx);
        let _ = signal_hook::low_level::pipe::register(signal_hook::consts::SIGTERM, tx_term);
        Some(rx)
    });

    let launch = LaunchOptions {
        open_settings: cli.settings,
        open_panel_editor: cli.panel_editor,
        input_method_mode: cli.input_method,
        force_show: cli.show || cli.toggle,
        initial_panel: cli.panel,
    };

    if let Err(err) = run_app(launch, shutdown, signal_pipe) {
        error!("{err}");
        eprintln!("wlboard error: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn forward_only(cmd: IpcCommand) -> ExitCode {
    match try_send_ipc_command(cmd) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("wlboard is not currently running.");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("Failed communicating with wlboard over D-Bus: {err}");
            ExitCode::FAILURE
        }
    }
}
