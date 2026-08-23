use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use akmux_client_core::{ClientError, Result, credentials::CredentialStore};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "AkironMux";

#[derive(Debug, Default)]
pub struct NativeCredentialStore;

impl CredentialStore for NativeCredentialStore {
    fn store(&self, profile_id: &str, token: &str) -> Result<()> {
        platform_store_credential(profile_id, token)
    }

    fn load(&self, profile_id: &str) -> Result<Zeroizing<String>> {
        platform_load_credential(profile_id)
    }

    fn delete(&self, profile_id: &str) -> Result<()> {
        platform_delete_credential(profile_id)
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_store_credential(profile_id: &str, token: &str) -> Result<()> {
    entry(profile_id)?.set_password(token).map_err(keyring_error)
}

#[cfg(not(target_os = "windows"))]
fn platform_load_credential(profile_id: &str) -> Result<Zeroizing<String>> {
    entry(profile_id)?.get_password().map(Zeroizing::new).map_err(keyring_error)
}

#[cfg(not(target_os = "windows"))]
fn platform_delete_credential(profile_id: &str) -> Result<()> {
    match entry(profile_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(keyring_error(error)),
    }
}

#[cfg(target_os = "windows")]
fn credential_target(profile_id: &str) -> String {
    format!("AkironMux/backend/{profile_id}")
}

#[cfg(target_os = "windows")]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    std::ffi::OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(target_os = "windows")]
fn platform_store_credential(profile_id: &str, token: &str) -> Result<()> {
    use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW};

    let target = wide(&credential_target(profile_id));
    let mut bytes = token.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_ptr() as *mut _,
        Comment: std::ptr::null_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: std::ptr::null_mut(),
        UserName: std::ptr::null_mut(),
    };
    let success = unsafe { CredWriteW(&credential, 0) };
    bytes.fill(0);
    if success == 0 {
        Err(ClientError::Platform("Windows Credential Manager could not store the credential".into()))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn platform_load_credential(profile_id: &str) -> Result<Zeroizing<String>> {
    use windows_sys::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW};

    let target = wide(&credential_target(profile_id));
    let mut pointer: *mut CREDENTIALW = std::ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 || pointer.is_null() {
        return Err(ClientError::Authentication("Backend credential is unavailable; re-authentication is required".into()));
    }
    let credential = unsafe { &*pointer };
    let size = credential.CredentialBlobSize as usize;
    if credential.CredentialBlob.is_null() || size == 0 || size > 16 * 1024 {
        unsafe { CredFree(pointer.cast()) };
        return Err(ClientError::Authentication("Backend credential is invalid".into()));
    }
    let mut bytes = unsafe { std::slice::from_raw_parts(credential.CredentialBlob, size) }.to_vec();
    unsafe { CredFree(pointer.cast()) };
    let token = String::from_utf8(bytes.clone()).map_err(|_| ClientError::Authentication("Backend credential is invalid".into()));
    bytes.fill(0);
    token.map(Zeroizing::new)
}

#[cfg(target_os = "windows")]
fn platform_delete_credential(profile_id: &str) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Security::Credentials::{CRED_TYPE_GENERIC, CredDeleteW},
    };

    let target = wide(&credential_target(profile_id));
    let success = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    let error = if success == 0 { unsafe { GetLastError() } } else { 0 };
    if success != 0 || error == 1168 {
        Ok(())
    } else {
        Err(ClientError::Platform("Windows Credential Manager could not delete the credential".into()))
    }
}
#[cfg(not(target_os = "windows"))]
fn entry(profile_id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("backend/{profile_id}")).map_err(keyring_error)
}

#[cfg(not(target_os = "windows"))]
fn keyring_error(error: keyring::Error) -> ClientError {
    ClientError::Platform(format!("Operating-system credential storage is unavailable: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseRequest {
    HideToTray,
    Quit,
}

pub trait DesktopPlatform: Send + Sync {
    fn notify(&self, title: &str, body: &str) -> Result<()>;
    fn open_url(&self, url: &str) -> Result<()>;
    fn request_attention(&self) -> Result<()>;
    fn apply_material(&self, dark: bool, transparency: u8) -> Result<()>;
    fn update_tray_sessions(&self, sessions: &[(String, String)]) -> Result<()>;
}

const INSTANCE_ADDRESS: &str = "127.0.0.1:17322";

pub struct SingleInstance {
    _listener: TcpListener,
    activations: async_channel::Receiver<()>,
}

pub enum InstanceOutcome {
    Primary(SingleInstance),
    Secondary,
}

impl SingleInstance {
    pub fn acquire() -> Result<InstanceOutcome> {
        match TcpListener::bind(INSTANCE_ADDRESS) {
            Ok(listener) => {
                listener
                    .set_nonblocking(true)
                    .map_err(|error| ClientError::Platform(format!("Failed to configure single-instance listener: {error}")))?;
                let worker = listener
                    .try_clone()
                    .map_err(|error| ClientError::Platform(format!("Failed to clone single-instance listener: {error}")))?;
                let (sender, receiver) = async_channel::unbounded();
                thread::Builder::new()
                    .name("akmux-instance-ipc".into())
                    .spawn(move || {
                        loop {
                            match worker.accept() {
                                Ok((mut stream, _)) => {
                                    let mut command = [0_u8; 8];
                                    if stream.read(&mut command).is_ok() && command.starts_with(b"ACTIVATE") {
                                        let _ = sender.send_blocking(());
                                    }
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(100)),
                                Err(_) => return,
                            }
                        }
                    })
                    .map_err(|error| ClientError::Platform(format!("Failed to start single-instance listener: {error}")))?;
                Ok(InstanceOutcome::Primary(Self {
                    _listener: listener,
                    activations: receiver,
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                let mut stream = TcpStream::connect_timeout(&INSTANCE_ADDRESS.parse().expect("static instance address is valid"), Duration::from_secs(1))
                    .map_err(|error| ClientError::Platform(format!("Another process owns the desktop activation endpoint: {error}")))?;
                stream
                    .write_all(b"ACTIVATE")
                    .map_err(|error| ClientError::Platform(format!("Failed to activate the running desktop client: {error}")))?;
                Ok(InstanceOutcome::Secondary)
            }
            Err(error) => Err(ClientError::Platform(format!("Failed to create single-instance listener: {error}"))),
        }
    }

    pub fn activations(&self) -> async_channel::Receiver<()> {
        self.activations.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    Activate,
    SelectSession(String),
    Quit,
}

pub struct DesktopTray {
    menu: Menu,
    _tray: TrayIcon,
}

impl DesktopTray {
    pub fn new() -> Result<(Self, async_channel::Receiver<TrayAction>)> {
        #[cfg(target_os = "linux")]
        gtk::init().map_err(|error| ClientError::Platform(format!("Failed to initialize the Linux tray integration: {error}")))?;

        let menu = Menu::new();
        rebuild_menu(&menu, &[], false)?;
        let icon = tray_icon()?;
        let tray = TrayIconBuilder::new()
            .with_id("akmux-desktop")
            .with_tooltip("AkironMux")
            .with_icon(icon)
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|error| ClientError::Platform(format!("Failed to create system tray icon: {error}")))?;
        let (sender, receiver) = async_channel::unbounded();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id.0.as_str();
            let action = match id {
                "activate" => Some(TrayAction::Activate),
                "quit" => Some(TrayAction::Quit),
                _ => id.strip_prefix("session:").map(|id| TrayAction::SelectSession(id.to_owned())),
            };
            if let Some(action) = action {
                let _ = sender.send_blocking(action);
            }
        }));
        Ok((Self { menu, _tray: tray }, receiver))
    }

    pub fn update_sessions(&mut self, sessions: &[(String, String)], simplified_chinese: bool) -> Result<()> {
        rebuild_menu(&self.menu, sessions, simplified_chinese)
    }

    pub fn pump_native_events(&self) {
        #[cfg(target_os = "linux")]
        while gtk::glib::MainContext::default().pending() {
            gtk::glib::MainContext::default().iteration(false);
        }
    }
}

fn rebuild_menu(menu: &Menu, sessions: &[(String, String)], simplified_chinese: bool) -> Result<()> {
    while menu.remove_at(0).is_some() {}
    menu.append(&MenuItem::with_id(
        "activate",
        if simplified_chinese { "打开 AkironMux" } else { "Open AkironMux" },
        true,
        None,
    ))
    .map_err(|error| ClientError::Platform(format!("Failed to update system tray menu: {error}")))?;
    for (id, title) in sessions {
        menu.append(&MenuItem::with_id(format!("session:{id}"), title, true, None))
            .map_err(|error| ClientError::Platform(format!("Failed to update system tray menu: {error}")))?;
    }
    menu.append(&MenuItem::with_id("quit", if simplified_chinese { "退出" } else { "Quit" }, true, None))
        .map_err(|error| ClientError::Platform(format!("Failed to update system tray menu: {error}")))
}

fn tray_icon() -> Result<Icon> {
    let size = 32_u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let inside = (4..28).contains(&x) && (4..28).contains(&y);
            let bright = inside && (x == 9 || x == 22 || (10..=21).contains(&x) && (y == 9 || y == 22));
            rgba.extend_from_slice(if bright {
                &[240, 249, 255, 255]
            } else if inside {
                &[14, 165, 233, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    Icon::from_rgba(rgba, size, size).map_err(|error| ClientError::Platform(format!("Failed to build system tray icon: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tray_icon_is_valid() {
        assert!(tray_icon().is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credential_adapter_preserves_the_tauri_target_name() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let profile_id = format!("credential-smoke-{suffix}");
        let token = format!("secret-{suffix}");
        let store = NativeCredentialStore;
        store.store(&profile_id, &token).unwrap();
        let loaded = store.load(&profile_id).unwrap();
        let delete_result = store.delete(&profile_id);
        assert_eq!(loaded.as_str(), token);
        delete_result.unwrap();
    }
}
