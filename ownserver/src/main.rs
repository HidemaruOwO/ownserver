use anyhow::Result;
use log::*;
use ownserver_lib::Payload;
use tokio_util::sync::CancellationToken;
use structopt::StructOpt;

use ownserver::proxy_client::run;

#[derive(StructOpt, Debug)]
#[structopt(name = "ownserver")]
struct Opt {
    #[structopt(long, default_value = "3000", help = "Port of your local game server listens e.g.) 25565 for Minecraft")]
    local_port: u16,
    #[structopt(long, default_value = "tcp", help = "tcp or udp")]
    payload: String,


    #[structopt(long, default_value = "5000", help = "Advanced settings")]
    control_port: u16,
    #[structopt(long, default_value = "https://auth.ownserver.kumassy.com/v1/request_token", help = "Advanced settings")]
    token_server: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();
    let opt = Opt::from_args();
    debug!("{:?}", opt);

    let payload = match opt.payload.as_str() {
        "udp" => Payload::UDP,
        _ => Payload::Other,
    };
    debug!("{:?}", payload);

    let store = Default::default();
    let cancellation_token = CancellationToken::new();

    let (client_info, mut set) =
        run(store, opt.control_port, opt.local_port, &opt.token_server, payload, cancellation_token).await?;
    info!("client is running under configuration: {:?}", client_info);



    while let Some(res) = set.join_next().await {
        match res {
            Err(join_error) => {
                error!("join error {:?} for client", join_error);
            }
            Ok(_) => {
                info!("client successfully terminated");
            }
        }
    }

    Ok(())
}
