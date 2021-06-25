// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Modifications Copyright (c) 2021, Foris Limited (licensed under the Apache License, Version 2.0)

use crate::config::{EnclaveOpt, VSockProxyOpt};
use crate::enclave_log_server::LogServer;
use crossbeam_channel::Receiver;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::process::{Command, Output};

/// The information provided by a `describe-enclaves` request.
#[derive(Clone, Serialize, Deserialize)]
pub struct EnclaveDescribeInfo {
    #[serde(rename = "EnclaveID")]
    /// The full ID of the enclave.
    pub enclave_id: String,
    #[serde(rename = "ProcessID")]
    /// The PID of the enclave process which manages the enclave.
    pub process_id: u32,
    #[serde(rename = "EnclaveCID")]
    /// The enclave's CID.
    pub enclave_cid: u64,
    #[serde(rename = "NumberOfCPUs")]
    /// The number of CPUs used by the enclave.
    pub cpu_count: u64,
    #[serde(rename = "CPUIDs")]
    /// The IDs of the CPUs used by the enclave.
    pub cpu_ids: Vec<u32>,
    #[serde(rename = "MemoryMiB")]
    /// The memory provided to the enclave (in MiB).
    pub memory_mib: u64,
    #[serde(rename = "State")]
    /// The current state of the enclave.
    pub state: String,
    #[serde(rename = "Flags")]
    /// The bit-mask which provides the enclave's launch flags.
    pub flags: String,
}

/// The information provided by a `run-enclave` request.
#[derive(Clone, Serialize, Deserialize)]
pub struct EnclaveRunInfo {
    #[serde(rename = "EnclaveID")]
    /// The full ID of the enclave.
    pub enclave_id: String,
    #[serde(rename = "ProcessID")]
    /// The PID of the enclave process which manages the enclave.
    pub process_id: u32,
    #[serde(rename = "EnclaveCID")]
    /// The enclave's CID.
    pub enclave_cid: u64,
    #[serde(rename = "NumberOfCPUs")]
    /// The number of CPUs used by the enclave.
    pub cpu_count: usize,
    #[serde(rename = "CPUIDs")]
    /// The IDs of the CPUs used by the enclave.
    pub cpu_ids: Vec<u32>,
    #[serde(rename = "MemoryMiB")]
    /// The memory provided to the enclave (in MiB).
    pub memory_mib: u64,
}

/// The information provided by a `terminate-enclave` request.
#[derive(Clone, Serialize, Deserialize)]
pub struct EnclaveTerminateInfo {
    #[serde(rename = "EnclaveID")]
    /// The full ID of the enclave.
    pub enclave_id: String,
    #[serde(rename = "Terminated")]
    /// A flag indicating if the enclave has terminated.
    pub terminated: bool,
}

fn parse_output<T: DeserializeOwned>(output: Output) -> Result<T, String> {
    if !output.status.success() {
        return Err(format!(
            "{}, status code: {:?}",
            String::from_utf8_lossy(output.stderr.as_slice()).to_string(),
            output.status.code(),
        ));
    }
    serde_json::from_slice(output.stdout.as_slice())
        .map_err(|_| "command invalid output".to_string())
}

fn run_enclave_daemon(
    image_path: &str,
    cpu_count: usize,
    memory_mib: u64,
) -> Result<EnclaveRunInfo, String> {
    let output = Command::new("nitro-cli")
        .arg("run-enclave")
        .args(&["--eif-path", image_path])
        .args(&["--cpu-count", &format!("{}", cpu_count)])
        .args(&["--memory", &format!("{}", memory_mib)])
        .output()
        .map_err(|e| format!("execute nitro-cli error: {}", e))?;
    parse_output(output)
}

pub fn run_enclave(opt: &EnclaveOpt, stop_receiver: Receiver<()>) -> Result<(), String> {
    // check if the enclave already running
    let enclave_info = describe_enclave()?;
    if !enclave_info.is_empty() {
        let info = serde_json::to_string_pretty(&enclave_info).expect("get invalid enclave info");
        return Err(format!(
            "the following enclave is already active, please stop and try again:\n{:?}",
            info
        ));
    }
    // lauch enclave server
    tracing::info!("start enclave log server at port {}", opt.log_server_port);
    let enclave_log_server = LogServer::new(
        opt.log_server_port,
        opt.log_to_console,
        opt.log_file.clone(),
    )
    .map_err(|e| format!("{:?}", e))?;

    enclave_log_server.launch();
    // run enclave
    let info = run_enclave_daemon(&opt.eif_path, opt.cpu_count, opt.memory_mib)?;
    let s = serde_json::to_string_pretty(&info).unwrap();
    tracing::info!("run enclave success:\n{}", s);
    // waiting for stop signal and stop the enclave
    let _ = stop_receiver.recv();
    let _ = stop_enclave(Some(info.enclave_id));
    Ok(())
}

pub fn stop_enclave(cid: Option<String>) -> Result<EnclaveTerminateInfo, String> {
    let mut cmd = Command::new("nitro-cli");
    cmd.arg("terminate-enclave");
    if let Some(id) = cid {
        cmd.args(&["--enclave-id", &id]);
    } else {
        cmd.arg("--all");
    }
    let output = cmd
        .output()
        .map_err(|e| format!("execute nitro-cli error: {:?}", e))?;
    parse_output(output)
}

pub fn describe_enclave() -> Result<Vec<EnclaveDescribeInfo>, String> {
    let output = Command::new("nitro-cli")
        .arg("describe-enclaves")
        .output()
        .map_err(|e| format!("execute nitro-cli error: {:?}", e))?;
    parse_output(output)
}

pub fn run_vsock_proxy(opt: &VSockProxyOpt) -> Result<(), String> {
    tracing::debug!("run vsock proxy with config: {:?}", opt);
    let _ = Command::new("vsock-proxy")
        .args(&["--num_workers", &format!("{}", opt.num_workers)])
        .args(&["--config", &opt.config_file])
        .arg(opt.local_port.to_string())
        .arg(&opt.remote_addr)
        .arg(opt.remote_port.to_string())
        .output()
        .map_err(|e| format!("execute nitro-cli error: {:?}", e))?;
    Ok(())
}