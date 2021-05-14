/// state persistence helper;
mod state;

use anomaly::format_err;
use ed25519_dalek as ed25519;
use std::io;
use std::os::unix::io::AsRawFd;
use std::thread;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tendermint::node::Id;
use tendermint_p2p::secret_connection::{self, PublicKey, SecretConnection};
use tmkms_light::chain::state::PersistStateSync;
use tmkms_light::config::validator::ValidatorConfig;
use tmkms_light::connection::{Connection, PlainConnection};
use tmkms_light::error::{
    Error,
    ErrorKind::{AccessError, InvalidKey, IoError},
};
use tmkms_light::utils::read_u16_payload;
use tmkms_nitro_helper::{NitroConfig, VSOCK_HOST_CID};
use tracing::{error, info, trace, warn};
use vsock::{SockAddr, VsockStream};
use zeroize::Zeroizing;

fn get_secret_connection(
    vsock_port: u32,
    identity_key: &ed25519::Keypair,
    peer_id: Option<Id>,
) -> io::Result<Box<dyn Connection>> {
    let addr = SockAddr::new_vsock(VSOCK_HOST_CID, vsock_port);
    let socket = vsock::VsockStream::connect(&addr)?;
    info!("KMS node ID: {}", PublicKey::from(identity_key));
    // the `Clone` is not derived for Keypair
    // TODO: https://github.com/dalek-cryptography/ed25519-dalek/issues/76
    let identity_key = ed25519::Keypair::from_bytes(&identity_key.to_bytes()).unwrap();
    let connection = SecretConnection::new(socket, identity_key, secret_connection::Version::V0_34)
        .map_err(|e| {
            error!("secret connection failed: {}", e);
            io::Error::from(io::ErrorKind::Other)
        })?;
    let actual_peer_id = connection.remote_pubkey().peer_id();

    // TODO: https://github.com/informalsystems/tendermint-rs/issues/786
    if let Some(expected_peer_id) = peer_id {
        if expected_peer_id.ct_eq(&actual_peer_id).unwrap_u8() == 0 {
            error!(
                "(vsock port {}): validator peer ID mismatch! (expected {}, got {})",
                vsock_port, expected_peer_id, actual_peer_id
            );
            return Err(io::Error::from(io::ErrorKind::Other));
        }
    }
    info!("connected to validator successfully");

    if peer_id.is_none() {
        // TODO: https://github.com/informalsystems/tendermint-rs/issues/786
        warn!(
            "unverified validator peer ID! ({})",
            connection.remote_pubkey().peer_id()
        );
    }

    Ok(Box::new(connection))
}

/// keeps retrying with approx. 1 sec sleep until it manages to connect to tendermint privval endpoint
pub fn get_connection(
    config: &NitroConfig,
    id_keypair: Option<&ed25519::Keypair>,
) -> Box<dyn Connection> {
    loop {
        let conn: io::Result<Box<dyn Connection>> = if let Some(ikp) = id_keypair {
            get_secret_connection(config.enclave_tendermint_conn, ikp, config.peer_id)
        } else {
            let addr = SockAddr::new_vsock(VSOCK_HOST_CID, config.enclave_tendermint_conn);
            if let Ok(socket) = vsock::VsockStream::connect(&addr) {
                trace!("tendermint vsock port: {}", config.enclave_tendermint_conn);
                trace!("tendermint peer addr: {:?}", socket.peer_addr());
                trace!("tendermint local addr: {:?}", socket.local_addr());
                trace!("tendermint fd: {}", socket.as_raw_fd());
                info!("connected to validator successfully");
                let plain_conn = PlainConnection::new(socket);
                Ok(Box::new(plain_conn))
            } else {
                warn!("vsock failed to connect to validator");
                Err(io::ErrorKind::Other.into())
            }
        };
        if let Err(e) = conn {
            error!("tendermint connection error {:?}", e);
            thread::sleep(Duration::new(1, 0));
        } else {
            return conn.unwrap();
        }
    }
}

/// a simple req-rep handling loop
pub fn entry(mut config_stream: VsockStream) -> Result<(), Error> {
    let json_raw = read_u16_payload(&mut config_stream)
        .map_err(|_e| format_err!(IoError, "failed to read config"))?;
    let mconfig = serde_json::from_slice(&json_raw);
    match mconfig {
        Ok(config) => {
            let config: NitroConfig = config;
            let key_bytes = Zeroizing::new(
                aws_ne_sys::kms_decrypt(
                    config.aws_region.as_bytes(),
                    config.credentials.aws_key_id.as_bytes(),
                    config.credentials.aws_secret_key.as_bytes(),
                    config.credentials.aws_session_token.as_bytes(),
                    config.sealed_consensus_key.as_ref(),
                )
                .map_err(|_e| format_err!(AccessError, "failed to decrypt key"))?,
            );
            let secret = ed25519::SecretKey::from_bytes(&*key_bytes)
                .map_err(|e| format_err!(InvalidKey, "invalid Ed25519 key: {}", e))?;
            let public = ed25519::PublicKey::from(&secret);
            let keypair = ed25519::Keypair { secret, public };
            let id_keypair = if let Some(ref ciphertext) = config.sealed_id_key {
                let id_key_bytes = Zeroizing::new(
                    aws_ne_sys::kms_decrypt(
                        config.aws_region.as_bytes(),
                        config.credentials.aws_key_id.as_bytes(),
                        config.credentials.aws_secret_key.as_bytes(),
                        config.credentials.aws_session_token.as_bytes(),
                        ciphertext.as_ref(),
                    )
                    .map_err(|_e| format_err!(AccessError, "failed to decrypt key"))?,
                );
                let id_secret = ed25519::SecretKey::from_bytes(&*id_key_bytes)
                    .map_err(|e| format_err!(InvalidKey, "invalid Ed25519 key: {}", e))?;
                let id_public = ed25519::PublicKey::from(&id_secret);
                let id_keypair = ed25519::Keypair {
                    secret: id_secret,
                    public: id_public,
                };
                Some(id_keypair)
            } else {
                None
            };
            let mut state_holder = state::StateHolder::new(config.enclave_state_port)
                .map_err(|_e| format_err!(IoError, "failed get state connection"))?;
            let state = state_holder
                .load_state()
                .map_err(|_e| format_err!(IoError, "failed to load initial state"))?;
            let conn: Box<dyn Connection> = get_connection(&config, id_keypair.as_ref());
            let mut session = tmkms_light::session::Session::new(
                ValidatorConfig {
                    chain_id: config.chain_id.clone(),
                    max_height: config.max_height,
                },
                conn,
                keypair,
                state,
                state_holder,
            );
            loop {
                if let Err(e) = session.request_loop() {
                    error!("request error: {}", e);
                }
                let conn: Box<dyn Connection> = get_connection(&config, id_keypair.as_ref());
                session.reset_connection(conn);
            }
        }
        Err(e) => {
            error!("config error: {}", e);
        }
    }

    Ok(())
}
