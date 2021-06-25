mod command;
mod config;
mod enclave_log_server;
mod key_utils;
mod proxy;
mod shared;
mod state;

use command::helper::{self, check_vsock_proxy};
use command::launch_all::launch_all;
use command::nitro_enclave::{describe_enclave, run_enclave, stop_enclave};
use config::{Config, EnclaveOpt, VSockProxyOpt};

use crate::command::nitro_enclave::run_vsock_proxy;
use crossbeam_channel::bounded;
use std::path::PathBuf;
use structopt::StructOpt;
use tmkms_light::utils::PubkeyDisplay;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

/// Helper sub-commands
#[derive(Debug, StructOpt)]
#[structopt(
    name = "tmkms-nitro-helper",
    about = "helper (proxies etc.) for nitro enclave execution"
)]
enum TmkmsLight {
    #[structopt(flatten)]
    Helper(CommandHelper),
    Enclave(CommandEnclave),
}

/// enclave sub-commands
#[derive(Debug, StructOpt)]
enum CommandEnclave {
    #[structopt(name = "info", about = "get tmkms info")]
    Info,
    #[structopt(name = "run", about = "run enclave")]
    RunEnclave {
        #[structopt(flatten)]
        opt: EnclaveOpt,
        /// log level, default: info, -v: info, -vv: debug, -vvv: trace
        #[structopt(short, parse(from_occurrences))]
        v: u32,
    },
    #[structopt(name = "stop", about = "stop enclave")]
    StopEnclave {
        /// Stop the enclave cid
        #[structopt(long)]
        cid: Option<String>,
    },
    #[structopt(name = "vsock-proxy", about = "launch vsock proxy")]
    RunProxy {
        #[structopt(flatten)]
        opt: VSockProxyOpt,
        /// log level, default: info, -v: info, -vv: debug, -vvv: trace
        #[structopt(short, parse(from_occurrences))]
        v: u32,
    },
}

/// Helper sub-commands
#[derive(Debug, StructOpt)]
enum CommandHelper {
    #[structopt(name = "init", about = "Create config + keygen")]
    /// Create config + keygen
    Init {
        #[structopt(short)]
        config_path: Option<PathBuf>,
        #[structopt(short)]
        pubkey_display: Option<PubkeyDisplay>,
        #[structopt(short)]
        bech32_prefix: Option<String>,
        #[structopt(short)]
        aws_region: String,
        #[structopt(short)]
        kms_key_id: String,
        #[structopt(long)]
        cid: Option<u32>,
    },
    #[structopt(name = "start", about = "start tmkms process")]
    /// start tmkms process (push config + start up proxy and state persistence)
    Start {
        #[structopt(short, default_value = "tmkms.toml")]
        config_path: PathBuf,
        #[structopt(long)]
        cid: Option<u32>,
        #[structopt(short, parse(from_occurrences))]
        /// log level, default: info, -v: info, -vv: debug, -vvv: trace
        #[structopt(short, parse(from_occurrences))]
        v: u32,
    },
    #[structopt(name = "launch-all", about = "launch all")]
    LaunchAll {
        /// config path
        #[structopt(short, default_value = "tmkms.toml")]
        config_path: PathBuf,
        #[structopt(short, parse(from_occurrences))]
        /// log level, default: info, -v: info, -vv: debug, -vvv: trace
        #[structopt(short, parse(from_occurrences))]
        v: u32,
    },
}

fn set_logger(v: u32) -> Result<(), String> {
    let log_level = match v {
        0 | 1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };
    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| format!("setting default subscriber failed: {:?}", e))?;
    Ok(())
}

fn run() -> Result<(), String> {
    let opt = TmkmsLight::from_args();
    match opt {
        TmkmsLight::Helper(CommandHelper::Init {
            config_path,
            pubkey_display,
            bech32_prefix,
            aws_region,
            kms_key_id,
            cid,
        }) => {
            helper::init(
                config_path,
                pubkey_display,
                bech32_prefix,
                aws_region,
                kms_key_id,
                cid,
            )?;
        }
        TmkmsLight::Helper(CommandHelper::Start {
            config_path,
            cid,
            v,
        }) => {
            set_logger(v)?;
            let config = Config::from_file(config_path)?;
            if !check_vsock_proxy() {
                return Err("vsock proxy not started".to_string());
            }
            helper::start(&config.sign_opt, cid)?;
        }
        TmkmsLight::Enclave(CommandEnclave::Info) => {
            let info = describe_enclave()?;
            let s = serde_json::to_string_pretty(&info)
                .map_err(|_| "get invalid enclave info".to_string())?;
            println!("enclave status:\n{}", s);
        }
        TmkmsLight::Enclave(CommandEnclave::RunEnclave { opt, v }) => {
            set_logger(v)?;
            let (sender, receiver) = bounded(1);
            ctrlc::set_handler(move || {
                let _ = sender.send(());
            })
            .map_err(|_| "Error to set Ctrl-C channel".to_string())?;
            run_enclave(&opt, receiver)?;
        }
        TmkmsLight::Enclave(CommandEnclave::StopEnclave { cid }) => {
            stop_enclave(cid)?;
        }
        TmkmsLight::Enclave(CommandEnclave::RunProxy { opt, v }) => {
            set_logger(v)?;
            run_vsock_proxy(&opt)?;
        }
        TmkmsLight::Helper(CommandHelper::LaunchAll { config_path, v }) => {
            set_logger(v)?;
            let config = Config::from_file(config_path)?;
            launch_all(config)?;
        }
    };
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}
