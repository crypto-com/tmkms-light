use anomaly::format_err;
use std::io;
use std::os::unix::io::AsRawFd;
use tmkms_light::chain::state::{consensus, PersistStateSync, State, StateError, StateErrorKind};
use tmkms_light::utils::{read_u16_payload, write_u16_payload};
use tmkms_nitro_helper::VSOCK_HOST_CID;
use tracing::{debug, trace};
use vsock::{SockAddr, VsockStream};

/// as the state needs to be persisted outside of NE,
/// this is a helper that communicates with the host to load the latest state
/// on the start up + to update it after each signing
#[derive(Debug, Clone)]
pub struct StateHolder {
    state_conn: VsockStream,
}

impl StateHolder {
    /// connects to the host via the vsock port specified in the configuration
    pub fn new(vsock_port: u32) -> io::Result<Self> {
        let addr = SockAddr::new_vsock(VSOCK_HOST_CID, vsock_port);
        let state_conn = vsock::VsockStream::connect(&addr)?;
        trace!("state vsock port: {}", vsock_port);
        trace!("state peer addr: {:?}", state_conn.peer_addr());
        trace!("state local addr: {:?}", state_conn.local_addr());
        trace!("state fd: {}", state_conn.as_raw_fd());
        Ok(Self { state_conn })
    }
}

impl PersistStateSync for StateHolder {
    /// loads the initial state
    fn load_state(&mut self) -> Result<State, StateError> {
        let json_raw = read_u16_payload(&mut self.state_conn)
            .map_err(|e| format_err!(StateErrorKind::SyncError, "error reading state: {}", e))?;
        let consensus_state: consensus::State = serde_json::from_slice(&json_raw)
            .map_err(|e| format_err!(StateErrorKind::SyncError, "error parsing state: {}", e))?;
        Ok(State::from(consensus_state))
    }

    /// sends the update state to be persisted on the host
    fn persist_state(&mut self, new_state: &consensus::State) -> Result<(), StateError> {
        trace!("writing new consensus state to state conn");
        trace!("state peer addr: {:?}", self.state_conn.peer_addr());
        trace!("state local addr: {:?}", self.state_conn.local_addr());
        trace!("state fd: {}", self.state_conn.as_raw_fd());
        let json_raw = serde_json::to_vec(&new_state).map_err(|e| {
            format_err!(StateErrorKind::SyncError, "error serializing state: {}", e)
        })?;

        write_u16_payload(&mut self.state_conn, &json_raw).map_err(|e| {
            format_err!(
                StateErrorKind::SyncError,
                "error state writting to socket {}",
                e
            )
        })?;

        debug!("successfully wrote new consensus state to state connection");

        Ok(())
    }
}
