use anomaly::format_err;
use std::{io, net::TcpStream};
use tmkms_light::{
    chain::state::{consensus, PersistStateSync, State, StateError, StateErrorKind},
    utils::{read_u16_payload, write_u16_payload},
};
use tracing::debug;

/// holds the connection for persiting the state outside of the enclave
pub struct StateHolder {
    state_conn: TcpStream,
}

impl StateHolder {
    /// tries to connect to "state" address which is provided
    /// as "usercall extension" in the runner
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            state_conn: TcpStream::connect("state")?,
        })
    }
}

impl PersistStateSync for StateHolder {
    fn load_state(&mut self) -> Result<State, StateError> {
        // TODO: currently unused as the initial state is now provided/loaded via "args"
        // so `PersistStateSync` is to be revisited
        let json_raw = read_u16_payload(&mut self.state_conn)
            .map_err(|e| format_err!(StateErrorKind::SyncError, "error reading state: {}", e))?;
        let consensus_state: consensus::State = serde_json::from_slice(&json_raw)
            .map_err(|e| format_err!(StateErrorKind::SyncError, "error parsing state: {}", e))?;
        Ok(State::from(consensus_state))
    }

    fn persist_state(&mut self, new_state: &consensus::State) -> Result<(), StateError> {
        debug!("writing new consensus state to state conn");

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
