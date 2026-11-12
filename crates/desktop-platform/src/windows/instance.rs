//! Same-user named-pipe IPC with an explicit owner-only DACL, remote clients
//! rejected, bounded payload/read deadline and an OS file lock (no idle polling).
use crate::{activation::Activation, Error, Result};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
};

pub enum Launch {
    Primary(Instance),
    Forwarded,
}
pub struct Instance {
    server: NamedPipeServer,
    name: String,
    _lock: File,
}
fn pipe_server(name: &str, first: bool) -> Result<NamedPipeServer> {
    use windows::{
        core::w,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                },
                PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
            },
        },
    };
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // OW is the owner-rights SID: only the object's owner, not Everyone/Users.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w!("D:P(A;;GA;;;OW)"),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    Ok(result?)
}
impl Instance {
    /// `directory` must be the application's per-user LocalAppData directory.
    /// Call before creating any visible window. `Open` is a normal second launch.
    pub async fn launch(directory: &Path, activation: Activation) -> Result<Launch> {
        let link = activation.uri();
        Activation::parse(&link)?;
        std::fs::create_dir_all(directory)?;
        let directory = std::fs::canonicalize(directory)?;
        let name = format!(
            r"\\.\pipe\StickTalk-{:x}",
            Sha256::digest(directory.to_string_lossy().to_lowercase().as_bytes())
        );
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("instance.lock"))?;
        match lock.try_lock_exclusive() {
            Ok(()) => {
                // Cancelled overlapped I/O can briefly retain the last process's
                // pipe after its instance lock is released. Startup retries only.
                let mut server = None;
                for attempt in 0..100 {
                    match pipe_server(&name, true) {
                        Ok(s) => {
                            server = Some(s);
                            break;
                        }
                        Err(Error::Io(e)) if e.raw_os_error() == Some(5) && attempt < 99 => {
                            tokio::time::sleep(Duration::from_millis(20)).await
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(Launch::Primary(Self {
                    _lock: lock,
                    server: server.ok_or(Error::Invalid("instance pipe unavailable"))?,
                    name,
                }))
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(33) =>
            {
                let mut client = None;
                // Startup-race retries only: this does not run while idle.
                for _ in 0..100 {
                    match ClientOptions::new().open(&name) {
                        Ok(c) => {
                            client = Some(c);
                            break;
                        }
                        Err(e) if matches!(e.raw_os_error(), Some(2 | 231)) => {
                            tokio::time::sleep(Duration::from_millis(20)).await
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                let mut client =
                    client.ok_or(Error::Invalid("existing instance IPC unavailable"))?;
                // Transfer the foreground permission of this user-initiated launch
                // to the existing process; SetForegroundWindow otherwise may fail.
                use std::os::windows::io::AsRawHandle;
                use windows::Win32::{
                    Foundation::HANDLE, System::Pipes::GetNamedPipeServerProcessId,
                    UI::WindowsAndMessaging::AllowSetForegroundWindow,
                };
                let mut pid = 0;
                unsafe {
                    GetNamedPipeServerProcessId(HANDLE(client.as_raw_handle()), &mut pid)?;
                    let _ = AllowSetForegroundWindow(pid);
                }
                let result = tokio::time::timeout(Duration::from_secs(2), async {
                    client.write_u32_le(link.len() as u32).await?;
                    client.write_all(link.as_bytes()).await?;
                    if client.read_u8().await? != 1 {
                        return Err(Error::Invalid("IPC acknowledgement"));
                    }
                    Ok(())
                })
                .await
                .map_err(|_| Error::Invalid("IPC deadline"))?;
                result?;
                Ok(Launch::Forwarded)
            }
            Err(e) => Err(e.into()),
        }
    }
    /// The host awaits one activation and dispatches it to the UI reducer; on
    /// Open/Invite/Message it restores/focuses the existing HWND. Call repeatedly.
    pub async fn receive(&mut self) -> Result<Activation> {
        self.server.connect().await?;
        // Tokio/Mio streams retain EOF readiness; do not recycle a disconnected
        // stream for another client. Keep a fresh listener ready before dispatch.
        let next = pipe_server(&self.name, false)?;
        let mut current = std::mem::replace(&mut self.server, next);
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let len = current.read_u32_le().await?;
            if len == 0 || len > 1024 {
                return Err(Error::Invalid("IPC payload size"));
            }
            let mut bytes = vec![0; len as usize];
            current.read_exact(&mut bytes).await?;
            let link = std::str::from_utf8(&bytes).map_err(|_| Error::Invalid("IPC UTF-8"))?;
            let activation = Activation::parse(link)?;
            current.write_u8(1).await?;
            // The client closes only after consuming the acknowledgement. Do not
            // disconnect earlier: DisconnectNamedPipe discards unread bytes.
            let mut end = [0];
            match current.read(&mut end).await {
                Ok(0) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                _ => return Err(Error::Invalid("IPC trailing data")),
            }
            Ok(activation)
        })
        .await
        .map_err(|_| Error::Invalid("IPC deadline"))
        .and_then(|r| r);
        current.disconnect()?;
        result
    }
}
