use crate::shared::VSOCK_HOST_CID;
use chrono::offset::Local;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tmkms_nitro_helper::tracing_layer::Log;
use tracing::Level;
use tracing::{debug, error, info, trace, warn};
use vsock::{SockAddr, VsockListener};

/// Configuration parameters for port listening and remote destination
pub struct LogServer {
    cid: u32,
    local_port: u32,
    // put log to console or not
    to_console: bool,
    // put log to file
    log_file: Option<File>,
}

impl LogServer {
    pub fn new(local_port: u32, to_console: bool, log_file: PathBuf) -> std::io::Result<Self> {
        let file = if log_file.file_name().is_none() {
            let f = OpenOptions::new()
                .write(true)
                .append(true)
                .open(&log_file)?;
            Some(f)
        } else {
            None
        };
        Ok(Self {
            cid: VSOCK_HOST_CID,
            local_port,
            to_console,
            log_file: file,
        })
    }

    /// Creates a listening socket
    /// Returns the file descriptor for it or the appropriate error
    fn sock_listen(&self) -> Result<VsockListener, String> {
        info!(
            "binding enclave log server to vsock port: {}",
            self.local_port
        );
        let sockaddr = SockAddr::new_vsock(self.cid, self.local_port);
        let listener = VsockListener::bind(&sockaddr)
            .map_err(|e| format!("Could not bind to {:?}, {:?}", sockaddr, e))?;
        info!("Bound enclave log server to {:?}", sockaddr);
        Ok(listener)
    }

    /// keep listening
    pub fn launch(mut self) {
        thread::spawn(move || loop {
            match self.sock_listen() {
                Ok(listener) => {
                    if let Err(e) = self.sock_accept(&listener) {
                        error!("enclave log server connection failed {}", e);
                        thread::sleep(Duration::new(1, 0));
                    }
                }
                Err(e) => {
                    error!("enclave log server listening failed {}", e);
                    thread::sleep(Duration::new(1, 0));
                }
            }
        });
    }

    /// Accepts an incoming connection coming on listener and handles it on a
    /// different thread
    /// Returns the handle for the new thread or the appropriate error
    fn sock_accept(&mut self, listener: &VsockListener) -> Result<(), String> {
        loop {
            let (mut client, client_addr) = listener
                .accept()
                .map_err(|_| "Enclave log server could not accept connection")?;
            trace!("Accepted connection on {:?}", client_addr);

            let mut tmp_buffer = [0u8; 8192];
            let nbytes = client
                .read(&mut tmp_buffer)
                .map_err(|e| format!("{:?}", e))?;
            self.process_log(&tmp_buffer[0..nbytes])?;
        }
    }

    fn process_log(&mut self, raw_log: &[u8]) -> Result<(), String> {
        let log = Log::from_raw(raw_log).map_err(|e| format!("{:?}", e))?;
        let mut s = log.format();
        if self.to_console {
            match log.level {
                Level::TRACE => trace!("{}", s),
                Level::DEBUG => debug!("{}", s),
                Level::INFO => info!("{}", s),
                Level::WARN => warn!("{}", s),
                Level::ERROR => error!("{}", s),
            }
        }

        if let Some(file) = self.log_file.as_mut() {
            let now = Local::now();
            s = format!("{} {:<6}{}\n", now.format("%F %T%.3f"), log.level, s);
            file.write_all(s.as_bytes())
                .map_err(|e| format!("write log to file error: {:?}", e))?;
            let _ = file.flush();
        }
        Ok(())
    }
}
