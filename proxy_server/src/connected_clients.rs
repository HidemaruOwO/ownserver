use dashmap::DashMap;
use futures::stream::SplitSink;
use futures::{Sink, StreamExt, SinkExt};
use futures::channel::mpsc::{UnboundedSender, unbounded, UnboundedReceiver, SendError};
pub use magic_tunnel_lib::{ClientId, ControlPacket};
use metrics::gauge;
use warp::hyper::Client;
use warp::ws::{Message, WebSocket};
use std::fmt::Formatter;
use std::sync::Arc;

#[derive(Clone)]
pub struct ConnectedClient {
    pub id: ClientId,
    pub host: String,
    // pub is_anonymous: bool,
    tx: UnboundedSender<ControlPacket>,
}

impl std::fmt::Debug for ConnectedClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectedClient")
            .field("id", &self.id)
            .field("sub", &self.host)
            // .field("anon", &self.is_anonymous)
            .finish()
    }
}

impl ConnectedClient {
    pub fn build<T>(id: ClientId, host: String, mut websocket_tx: T) -> Self where T: Sink<Message> + Unpin + std::marker::Send + 'static, T::Error: std::fmt::Debug {
        let (tx, mut rx) = unbounded::<ControlPacket>();

        tokio::spawn(async move {
            loop {
                match rx.next().await {
                    Some(packet) => {
                        let data = match rmp_serde::to_vec(&packet) {
                            Ok(data) => data,
                            Err(error) => {
                                tracing::warn!(cid = %id, error = ?error, "failed to encode message");
                                // return client;
                                break
                            }
                        };
        
                        let result = websocket_tx.send(Message::binary(data)).await;
                        if let Err(error) = result {
                            tracing::debug!(cid = %id, error = ?error, "client disconnected: aborting");
                            // return client;
                            break
                        }
                    }
                    None => {
                        tracing::debug!(cid = %id, "ending client tunnel");
                        // return client;
                        break
                    }
                };
            }

            // TODO: some cleanup code
            // Connections::remove(conn, &client);
            // remote_cancellers.remove(&client_id)
            tracing::debug!(cid = %id, "cleaning up ws send listener");
        });

        ConnectedClient { id, host, tx }
    }

    pub fn new(id: ClientId, host: String, tx: UnboundedSender<ControlPacket>) -> Self {
        Self {
            id, 
            host, 
            tx
        }
    }

    pub async fn send_to_client(&mut self, packet: ControlPacket) -> Result<(), SendError> {
        self.tx.send(packet).await
    }

    pub fn close_channel(&self) {
        self.tx.close_channel()
    }
}

#[derive(Debug)]
pub struct Connections {
    clients: Arc<DashMap<ClientId, ConnectedClient>>,
    hosts: Arc<DashMap<String, ConnectedClient>>,
}

impl Default for Connections {
    fn default() -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            hosts: Arc::new(DashMap::new()), 
        }
    }
}

impl Connections {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            hosts: Arc::new(DashMap::new()),
        }
    }

    // pub fn update_host(connection: &mut Self, client: &ConnectedClient) {
    //     connection
    //         .hosts
    //         .insert(client.host.clone(), client.clone());
    // }

    pub fn remove(&self, client: &ConnectedClient) {
        client.tx.close_channel();

        // https://github.com/agrinman/tunnelto/blob/0.1.9/src/server/connected_clients.rs
        self.hosts.remove(&client.host);
        self.clients.remove(&client.id);
        tracing::debug!(cid = %client.id, "rm client");
        gauge!("magic_tunnel_server.control.connections", self.clients.len() as f64);
    }

    pub fn remove_by_id(&self, client_id: ClientId) {
        if let Some(client ) = self.clients.get(&client_id) {
            client.tx.close_channel();

            self.hosts.remove(&client.host);
            self.clients.remove(&client.id);
            tracing::debug!(cid = %client.id, "rm client");
            gauge!("magic_tunnel_server.control.connections", self.clients.len() as f64);
        }
    }

    // pub fn client_for_host(connection: &mut Self, host: &String) -> Option<ClientId> {
    //     connection.hosts.get(host).map(|c| c.id.clone())
    // }

    pub fn get(&self, client_id: &ClientId) -> Option<ConnectedClient> {
        self
            .clients
            .get(client_id)
            .map(|c| c.value().clone())
    }

    pub fn find_by_host(&self, host: &String) -> Option<ConnectedClient> {
        self.hosts.get(host).map(|c| c.value().clone())
    }

    pub fn add(&self, client: ConnectedClient) {
        self.clients.insert(client.id, client.clone());
        self.hosts.insert(client.host.clone(), client);
        gauge!("magic_tunnel_server.control.connections", self.clients.len() as f64);
    }

    pub fn clear(&self) {
        self.clients.clear();
        self.hosts.clear();
    }

    pub fn len_clients(&self) -> usize {
        self.clients.len()
    }

    pub fn len_hosts(&self) -> usize {
        self.hosts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::unbounded;
    use magic_tunnel_lib::StreamId;

    fn setup() -> (Connections, ConnectedClient, ClientId) {
        let conn = Connections::new();
        let (tx, _rx) = unbounded::<ControlPacket>();
        let client_id = ClientId::new();
        let client = ConnectedClient {
            id: client_id,
            host: "foobar".into(),
            tx,
        };

        (conn, client, client_id)
    }

    #[test]
    fn connections_clients_should_be_empty() {
        let (conn, _, client_id) = setup();

        assert!(conn.get(&client_id).is_none());
    }

    #[test]
    fn connections_hosts_should_be_empty() {
        let (conn, _, _) = setup();

        assert!(conn.find_by_host(&"foobar".to_owned()).is_none());
    }

    #[test]
    fn connections_client_should_be_registered() {
        let (conn, client, client_id) = setup();

        conn.add(client);

        assert_eq!(conn.get(&client_id).unwrap().id, client_id);
    }

    #[test]
    fn connections_hosts_should_be_registered() {
        let (conn, client, client_id) = setup();

        conn.add(client);

        assert_eq!(
            conn.find_by_host(&"foobar".to_owned())
                .unwrap()
                .id,
            client_id
        );
    }

    #[test]
    fn connections_client_should_be_empty_after_remove() {
        let (conn, client, client_id) = setup();

        conn.add(client.clone());
        conn.remove(&client);

        assert!(conn.get(&client_id).is_none());
    }

    #[test]
    fn connections_hosts_should_be_empty_after_remove() {
        let (conn, client, _) = setup();

        conn.add(client.clone());
        conn.remove(&client);

        assert!(conn.find_by_host(&"foobar".to_owned()).is_none());
    }

    #[test]
    fn connections_hosts_ignore_multiple_add() {
        let (conn, client, _) = setup();

        conn.add(client.clone());
        conn.add(client.clone());
        conn.add(client.clone());

        assert_eq!(conn.clients.len(), 1);
        assert_eq!(conn.hosts.len(), 1);
    }

    #[test]
    fn connections_hosts_ignore_multiple_remove() {
        let (conn, client, _) = setup();

        conn.add(client.clone());
        conn.remove(&client);
        conn.remove(&client);
        conn.remove(&client);

        assert_eq!(conn.clients.len(), 0);
        assert_eq!(conn.hosts.len(), 0);
    }

    #[test]
    fn connections_should_clear() {
        let (conn, client, _) = setup();

        conn.add(client);
        conn.clear();

        assert_eq!(conn.clients.len(), 0);
        assert_eq!(conn.hosts.len(), 0);
    }

    #[tokio::test]
    async fn forward_control_packet_to_websocket() -> Result<(), Box<dyn std::error::Error>> {
        let client_id = ClientId::new();
        let stream_id = StreamId::new();
        let (ws_tx, mut ws_rx) = unbounded::<Message>();
        let mut client = ConnectedClient::build(client_id, "foobar".into(), ws_tx);

        client.send_to_client(ControlPacket::Init(stream_id)).await.unwrap();
        client.close_channel();

        let payload = ws_rx.next().await.unwrap().into_bytes();

        let packet: ControlPacket = rmp_serde::from_slice(&payload)?;
        assert_eq!(packet, ControlPacket::Init(stream_id));
        assert_eq!(ws_rx.next().await, None);

        Ok(())
    }
}