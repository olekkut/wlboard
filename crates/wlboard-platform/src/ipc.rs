//! Single-instance D-Bus IPC (`io.github.olekkut.wlboard`) powering `wlboard --toggle` and `--settings`.

use zbus::{Connection, interface, proxy};

use crate::PlatformEvent;
use crate::tray::EventCallback;

pub const APP_BUS_NAME: &str = "io.github.olekkut.wlboard";
pub const APP_OBJECT_PATH: &str = "/io/github/olekkut/wlboard";

/// Commands that a secondary CLI invocation can forward to the running `wlboard` instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcCommand {
    Toggle,
    Show,
    Hide,
    OpenSettings,
    SetLayout(String),
    Quit,
}

/// D-Bus service served by the primary `wlboard` process.
pub struct WlboardIpcService {
    on_event: EventCallback,
}

impl WlboardIpcService {
    #[must_use]
    pub fn new(on_event: EventCallback) -> Self {
        Self { on_event }
    }
}

#[interface(name = "io.github.olekkut.wlboard")]
impl WlboardIpcService {
    fn toggle(&self) {
        (self.on_event)(PlatformEvent::ToggleVisible);
    }

    fn show(&self) {
        (self.on_event)(PlatformEvent::Show);
    }

    fn hide(&self) {
        (self.on_event)(PlatformEvent::Hide);
    }

    fn open_settings(&self) {
        (self.on_event)(PlatformEvent::OpenSettings);
    }

    fn set_layout(&self, code: &str) {
        (self.on_event)(PlatformEvent::SetLayout(code.to_owned()));
    }

    fn quit(&self) {
        (self.on_event)(PlatformEvent::Quit);
    }
}

#[proxy(
    interface = "io.github.olekkut.wlboard",
    default_service = "io.github.olekkut.wlboard",
    default_path = "/io/github/olekkut/wlboard"
)]
trait WlboardIpcClient {
    fn toggle(&self) -> zbus::Result<()>;
    fn show(&self) -> zbus::Result<()>;
    fn hide(&self) -> zbus::Result<()>;
    fn open_settings(&self) -> zbus::Result<()>;
    fn set_layout(&self, code: &str) -> zbus::Result<()>;
    fn quit(&self) -> zbus::Result<()>;
}

/// Try forwarding a CLI command to an already-running `wlboard` instance.
///
/// Returns `Ok(true)` if an existing instance handled the command, `Ok(false)` if none is running.
pub fn try_send_ipc_command(cmd: IpcCommand) -> zbus::Result<bool> {
    futures_lite::future::block_on(async move {
        let conn = Connection::session().await?;
        let proxy = WlboardIpcClientProxy::new(&conn).await?;
        let res = match cmd {
            IpcCommand::Toggle => proxy.toggle().await,
            IpcCommand::Show => proxy.show().await,
            IpcCommand::Hide => proxy.hide().await,
            IpcCommand::OpenSettings => proxy.open_settings().await,
            IpcCommand::SetLayout(code) => proxy.set_layout(&code).await,
            IpcCommand::Quit => proxy.quit().await,
        };
        match res {
            Ok(()) => Ok(true),
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown"
                    || name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
            {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    })
}
