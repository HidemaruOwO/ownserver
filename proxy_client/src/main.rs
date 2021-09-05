use anyhow::Result;
use log::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use magic_tunnel_client::{proxy_client::run, ActiveStreams};

lazy_static::lazy_static! {
    pub static ref ACTIVE_STREAMS: ActiveStreams = Arc::new(RwLock::new(HashMap::new()));
}

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();
    let control_port: u16 = 5000;
    let local_port: u16 = 3000;
    let token_server = "http://localhost:4567";

    let (client_info, handle_client_to_control, handle_control_to_client) =
        run(&ACTIVE_STREAMS, control_port, local_port, token_server).await?;
    info!("client is running under configuration: {:?}", client_info);

    let (client_to_control, control_to_client) =
        futures::join!(handle_client_to_control, handle_control_to_client);

    match client_to_control {
        Err(join_error) => {
            error!("join error {:?} for client_to_control", join_error);
        }
        Ok(_) => {
            info!("client_to_control successfully terminated");
        }
    }

    match control_to_client {
        Err(join_error) => {
            error!("join error {:?} for control_to_client", join_error);
        }
        Ok(Err(error)) => {
            info!("client exited. reason: {:?}", error);
        }
        Ok(Ok(_)) => {
            info!("control_to_client successfully terminated");
        }
    }

    Ok(())
}
