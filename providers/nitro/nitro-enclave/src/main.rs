use tracing::{error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{fmt, EnvFilter};
use vsock::{SockAddr, VsockListener};

use tmkms_nitro_helper::tracing_layer::Layer;
use tmkms_nitro_helper::VSOCK_HOST_CID;

mod nitro;

fn main() {
    let mut env_args = std::env::args();
    let port = env_args
        .next()
        .map(|x| x.parse::<u32>().ok())
        .flatten()
        .unwrap_or(5050);

    let log_server_port = env_args
        .next()
        .map(|x| x.parse::<u32>().ok())
        .flatten()
        .unwrap_or(6050);

    let layer = Layer::new(VSOCK_HOST_CID, log_server_port);
    let fmt_layer = fmt::layer().with_target(false);

    // TODO: the log level via a command line arg?
    let level = "info";
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap();

    let layered = tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(layer);

    tracing::subscriber::set_global_default(layered).expect("setting default subscriber failed");

    const VMADDR_CID_ANY: u32 = 0xFFFFFFFF;
    let addr = SockAddr::new_vsock(VMADDR_CID_ANY, port);
    let listener = VsockListener::bind(&addr).expect("bind address");
    info!("waiting for config to be pushed on {}", addr);
    for conn in listener.incoming() {
        if aws_ne_sys::seed_entropy(512).is_err() {
            error!("failed to seed initial entropy!");
            std::process::exit(1);
        }
        match conn {
            Ok(stream) => {
                info!("got connection on {:?}", addr);
                if let Err(e) = nitro::entry(stream) {
                    error!("io error {}", e);
                }
            }
            Err(e) => {
                warn!("connection error {}", e);
            }
        }
    }
}
