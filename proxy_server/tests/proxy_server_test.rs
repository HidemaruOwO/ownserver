// integrated server test
use std::sync::Arc;
use std::time::Duration;
use dashmap::DashMap;
use lazy_static::lazy_static;
use tokio_tungstenite::{
    connect_async, WebSocketStream, MaybeTlsStream,
    tungstenite::Message,
};
use tokio::net::TcpStream;
use url::Url;
use futures::{StreamExt, SinkExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use magic_tunnel_lib::ControlPacket;
use magic_tunnel_server::{proxy_server::run, active_stream::ActiveStreams, connected_clients::Connections};
use magic_tunnel_client::{proxy_client::{send_client_hello, verify_server_hello}};

#[cfg(test)]
mod tunnel_to_stream_test {
    use super::*;
    use serial_test::serial;

    macro_rules! assert_control_packet_type_matches {
        ($expr:expr, $pat:pat) => {
            let payload = $expr.next().await.unwrap()?.into_data();
            let control_packet = ControlPacket::deserialize(&payload)?;
            assert!(matches!(control_packet, $pat));
        }
    }

    macro_rules! assert_control_packet_matches {
        ($expr:expr, $expected:expr) => {
            let payload = $expr.next().await.unwrap()?.into_data();
            let control_packet = ControlPacket::deserialize(&payload)?;
            assert_eq!(control_packet, $expected);
        }
    }

    macro_rules! assert_remote_bytes_matches {
        ($read:expr, $expected:expr) => {
            let mut buf = [0; 4*1024];
            let n = $read.read(&mut buf).await.expect("failed to read data from socket");
            let data = buf[..n].to_vec();

            assert_eq!(data, $expected);
        }
    }

    async fn setup_proxy_server(control_port: u16, remote_port: u16) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, ActiveStreams), Box<dyn std::error::Error>> {
        lazy_static! {
            pub static ref CONNECTIONS: Connections = Connections::new();
            pub static ref ACTIVE_STREAMS: ActiveStreams = Arc::new(DashMap::new());
        }
        // we must clear CONNECTIONS, ACTIVE_STREAMS
        // because they are shared across test
        Connections::clear(&CONNECTIONS);
        ACTIVE_STREAMS.clear();

        tokio::spawn(async move {
            run(&CONNECTIONS, &ACTIVE_STREAMS, control_port, remote_port).await;
        });

        // setup proxy client
        // --- handshake
        let url = Url::parse(&format!("wss://localhost:{}/tunnel", control_port))?;
        let (mut websocket, _ ) = connect_async(url).await.expect("failed to connect");

        send_client_hello(&mut websocket).await?;
        let _client_info = verify_server_hello(&mut websocket).await?;

        Ok((websocket, ACTIVE_STREAMS.clone()))
    }

    #[tokio::test]
    #[serial]
    async fn forward_remote_traffic_to_client() -> Result<(), Box<dyn std::error::Error>> {
        let control_port: u16 = 5000;
        let remote_port: u16 = 8080;
        let (websoket, active_streams) = setup_proxy_server(control_port, remote_port).await?;
        let (mut _ws_sink, mut ws_stream) = websoket.split();

        // ensure active_streams has cleared
        assert_eq!(active_streams.iter().count(), 0);

        // access remote port
        let mut remote = TcpStream::connect(format!("127.0.0.1:{}", remote_port)).await.expect("Failed to connect to remote port");
        // wait until remote access has registered to ACTIVE_STREAMS
        tokio::time::sleep(Duration::from_secs(3)).await;
        remote.write_all(b"some bytes").await.expect("failed to send client hello");

        // ensure active_streams has successfully registered
        assert_eq!(active_streams.iter().count(), 1);
        let stream_id = active_streams.iter().next().unwrap().id.clone();

        assert_control_packet_matches!(ws_stream, ControlPacket::Init(stream_id.clone()));
        assert_control_packet_matches!(ws_stream, ControlPacket::Data(stream_id.clone(), b"some bytes".to_vec()));
        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn forward_local_traffic_to_remote() -> Result<(), Box<dyn std::error::Error>> {
        let control_port: u16 = 5000;
        let remote_port: u16 = 8080;
        let (websoket, active_streams) = setup_proxy_server(control_port, remote_port).await?;
        let (mut ws_sink, mut _ws_stream) = websoket.split();

        // ensure active_streams has cleared
        assert_eq!(active_streams.iter().count(), 0);

        // access remote port
        let mut remote = TcpStream::connect(format!("127.0.0.1:{}", remote_port)).await.expect("Failed to connect to remote port");
        // wait until remote access has registered to ACTIVE_STREAMS
        tokio::time::sleep(Duration::from_secs(3)).await;
        
        // ensure active_streams has successfully registered
        assert_eq!(active_streams.iter().count(), 1);
        let stream_id = active_streams.iter().next().unwrap().id.clone();
        ws_sink.send(Message::binary(ControlPacket::Data(stream_id, b"foobarbaz".to_vec()).serialize())).await?;

        assert_remote_bytes_matches!(remote, b"foobarbaz");
        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn forward_multiple_remote_traffic_to_client() -> Result<(), Box<dyn std::error::Error>> {
        let control_port: u16 = 5000;
        let remote_port: u16 = 8080;
        let (websoket, active_streams) = setup_proxy_server(control_port, remote_port).await?;
        let (mut _ws_sink, mut ws_stream) = websoket.split();

        // ensure active_streams has cleared
        assert_eq!(active_streams.iter().count(), 0);

        // access remote port
        let mut remote1 = TcpStream::connect(format!("127.0.0.1:{}", remote_port)).await.expect("Failed to connect to remote port");
        // wait until remote access has registered to ACTIVE_STREAMS
        tokio::time::sleep(Duration::from_secs(3)).await;
        // ensure active_streams has successfully registered
        assert_eq!(active_streams.iter().count(), 1);
        let stream_id1 = active_streams.iter().next().unwrap().id.clone();

        let mut remote2 = TcpStream::connect(format!("127.0.0.1:{}", remote_port)).await.expect("Failed to connect to remote port");
        // wait until remote access has registered to ACTIVE_STREAMS
        tokio::time::sleep(Duration::from_secs(3)).await;
        // ensure active_streams has successfully registered
        assert_eq!(active_streams.iter().count(), 2);
        let stream_id2 = active_streams.iter().filter(|sid| sid.key() != &stream_id1).next().unwrap().id.clone();

        assert_ne!(stream_id1, stream_id2);

        remote1.write_all(b"some bytes 1").await.expect("failed to send client hello");
        tokio::time::sleep(Duration::from_secs(3)).await;
        remote2.write_all(b"some bytes 2").await.expect("failed to send client hello");


        assert_control_packet_matches!(ws_stream, ControlPacket::Init(stream_id1.clone()));
        assert_control_packet_matches!(ws_stream, ControlPacket::Init(stream_id2.clone()));
        assert_control_packet_matches!(ws_stream, ControlPacket::Data(stream_id1.clone(), b"some bytes 1".to_vec()));
        assert_control_packet_matches!(ws_stream, ControlPacket::Data(stream_id2.clone(), b"some bytes 2".to_vec()));
        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn forward_local_traffic_to_multiple_remote() -> Result<(), Box<dyn std::error::Error>> {
        let control_port: u16 = 5000;
        let remote_port: u16 = 8080;
        let (websoket, active_streams) = setup_proxy_server(control_port, remote_port).await?;
        let (mut ws_sink, mut _ws_stream) = websoket.split();

        // ensure active_streams has cleared
        assert_eq!(active_streams.iter().count(), 0);

        // access remote port
        let mut remote1 = TcpStream::connect(format!("127.0.0.1:{}", remote_port)).await.expect("Failed to connect to remote port");
        // wait until remote access has registered to ACTIVE_STREAMS
        tokio::time::sleep(Duration::from_secs(3)).await;
        // ensure active_streams has successfully registered
        assert_eq!(active_streams.iter().count(), 1);
        let stream_id1 = active_streams.iter().next().unwrap().id.clone();

        let mut remote2 = TcpStream::connect(format!("127.0.0.1:{}", remote_port)).await.expect("Failed to connect to remote port");
        // wait until remote access has registered to ACTIVE_STREAMS
        tokio::time::sleep(Duration::from_secs(3)).await;
        // ensure active_streams has successfully registered
        assert_eq!(active_streams.iter().count(), 2);
        let stream_id2 = active_streams.iter().filter(|sid| sid.key() != &stream_id1).next().unwrap().id.clone();

        assert_ne!(stream_id1, stream_id2);

        ws_sink.send(Message::binary(ControlPacket::Data(stream_id1, b"some message 1".to_vec()).serialize())).await?;
        ws_sink.send(Message::binary(ControlPacket::Data(stream_id2, b"some message 2".to_vec()).serialize())).await?;

        assert_remote_bytes_matches!(remote1, b"some message 1");
        assert_remote_bytes_matches!(remote2, b"some message 2");
        Ok(())
    }
}