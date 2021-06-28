pub mod launch_all;
pub mod nitro_enclave;

use std::{fs, path::PathBuf};
use sysinfo::{ProcessExt, SystemExt};
use tendermint::net;
use tmkms_light::utils::write_u16_payload;
use tmkms_light::utils::{print_pubkey, PubkeyDisplay};
use vsock::SockAddr;

use crate::config::{Config, EnclaveOpt, NitroSignOpt, VSockProxyOpt};
use crate::key_utils::{credential, generate_key};
use crate::proxy::Proxy;
use crate::shared::{NitroConfig, NitroRequest};
use crate::state::StateSyncer;

/// write tmkms.toml + tmkms.launch_all.toml + generate keys
pub fn init(
    config_path: PathBuf,
    pubkey_display: Option<PubkeyDisplay>,
    bech32_prefix: Option<String>,
    aws_region: String,
    kms_key_id: String,
    cid: Option<u32>,
) -> Result<(), String> {
    let file_stem = config_path
        .file_stem()
        .map(|s| s.to_str().unwrap_or("tmkms"))
        .unwrap_or("tmkms");
    let file_name_launch_all = format!("{}.launch_all.toml", file_stem);
    let cp_launch_all = config_path.with_file_name(file_name_launch_all);

    let nitro_sign_opt = NitroSignOpt {
        aws_region: aws_region.clone(),
        ..Default::default()
    };
    let enclave_opt = EnclaveOpt::default();
    let proxy_opt = VSockProxyOpt {
        remote_addr: format!("kms.{}.amazonaws.com", aws_region),
        ..Default::default()
    };
    let all_config = Config {
        sign_opt: nitro_sign_opt,
        enclave: enclave_opt,
        vsock_proxy: proxy_opt,
    };
    let t = toml::to_string_pretty(&all_config.sign_opt)
        .map_err(|e| format!("failed to create a config in toml: {:?}", e))?;
    let t_launch_all = toml::to_string(&all_config)
        .map_err(|e| format!("failed to create a config in toml: {:?}", e))?;
    fs::write(config_path, t).map_err(|e| format!("failed to write a config: {:?}", e))?;
    fs::write(cp_launch_all, t_launch_all)
        .map_err(|e| format!("failed to write a luanch all config: {:?}", e))?;
    let config = all_config.sign_opt;
    let (cid, port) = if let Some(cid) = cid {
        (cid, config.enclave_config_port)
    } else {
        (config.enclave_config_cid, config.enclave_config_port)
    };
    let credentials = if let Some(credentials) = config.credentials {
        credentials
    } else {
        credential::get_credentials()?
    };
    fs::create_dir_all(
        config
            .sealed_consensus_key_path
            .parent()
            .ok_or_else(|| "cannot create a dir in a root directory".to_owned())?,
    )
    .map_err(|e| format!("failed to create dirs for key storage: {:?}", e))?;
    fs::create_dir_all(
        config
            .state_file_path
            .parent()
            .ok_or_else(|| "cannot create a dir in a root directory".to_owned())?,
    )
    .map_err(|e| format!("failed to create dirs for state storage: {:?}", e))?;
    let (pubkey, attestation_doc) = generate_key(
        cid,
        port,
        config.sealed_consensus_key_path,
        &config.aws_region,
        credentials.clone(),
        kms_key_id.clone(),
    )
    .map_err(|e| format!("failed to generate a key: {:?}", e))?;
    print_pubkey(bech32_prefix, pubkey_display, pubkey);
    let encoded_attdoc = String::from_utf8(subtle_encoding::base64::encode(&attestation_doc))
        .map_err(|e| format!("enconding attestation doc: {:?}", e))?;
    println!("Nitro Enclave attestation:\n{}", &encoded_attdoc);

    if let Some(id_path) = config.sealed_id_key_path {
        generate_key(
            cid,
            port,
            id_path,
            &config.aws_region,
            credentials,
            kms_key_id,
        )
        .map_err(|e| format!("failed to generate a sealed id key: {:?}", e))?;
    }
    Ok(())
}

pub fn check_vsock_proxy() -> bool {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();
    system.get_processes().iter().any(|(_pid, p)| {
        let cmd = p.cmd();
        cmd.contains(&"vsock-proxy".to_string())
    })
}

/// push config to enclave, start up a proxy (if needed) + state syncer
pub fn start(config: &NitroSignOpt, cid: Option<u32>) -> Result<(), String> {
    tracing::debug!("start helper with config: {:?}, cid: {:?}", config, cid);
    let credentials = if let Some(credentials) = &config.credentials {
        credentials.clone()
    } else {
        credential::get_credentials()?
    };
    let peer_id = match config.address {
        net::Address::Tcp { peer_id, .. } => peer_id,
        _ => None,
    };
    let state_syncer = StateSyncer::new(config.state_file_path.clone(), config.enclave_state_port)
        .map_err(|e| format!("failed to get a state syncing helper: {:?}", e))?;
    let sealed_consensus_key = fs::read(config.sealed_consensus_key_path.clone())
        .map_err(|e| format!("failed to read a sealed consensus key: {:?}", e))?;
    let sealed_id_key = if let Some(p) = &config.sealed_id_key_path {
        if let net::Address::Tcp { .. } = config.address {
            Some(
                fs::read(p)
                    .map_err(|e| format!("failed to read a sealed identity key: {:?}", e))?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let enclave_config = NitroConfig {
        chain_id: config.chain_id.clone(),
        max_height: config.max_height,
        sealed_consensus_key,
        sealed_id_key,
        peer_id,
        enclave_state_port: config.enclave_state_port,
        enclave_tendermint_conn: config.enclave_tendermint_conn,
        credentials,
        aws_region: config.aws_region.clone(),
    };
    let addr = if let Some(cid) = cid {
        SockAddr::new_vsock(cid, config.enclave_config_port)
    } else {
        SockAddr::new_vsock(config.enclave_config_cid, config.enclave_config_port)
    };
    let mut socket = vsock::VsockStream::connect(&addr).map_err(|e| {
        format!(
            "failed to connect to the enclave to push its config: {:?}",
            e
        )
    })?;
    let request = NitroRequest::Start(enclave_config);
    let config_raw = serde_json::to_vec(&request)
        .map_err(|e| format!("failed to serialize the config: {:?}", e))?;
    write_u16_payload(&mut socket, &config_raw)
        .map_err(|e| format!("failed to write the config: {:?}", e))?;
    let proxy = match &config.address {
        net::Address::Unix { path } => {
            tracing::debug!(
                "{}: Creating a proxy {}...",
                &config.chain_id,
                &config.address
            );

            Some(Proxy::new(config.enclave_tendermint_conn, path.clone()))
        }
        _ => None,
    };
    if let Some(p) = proxy {
        p.launch_proxy();
    }

    // state syncing runs in an infinite loop (so does the proxy)
    // TODO: check if signal capture + a graceful shutdown would help with anything (given state writing is via "tempfile")
    state_syncer.launch_syncer().join().expect("state syncing");
    Ok(())
}
