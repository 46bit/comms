mod transmit;
mod broadcast;
mod receive;

pub use self::transmit::*;
pub use self::broadcast::*;
pub use self::transmit::*;
pub use self::receive::*;

use std::hash::Hash;
use std::collections::{HashMap, HashSet};
use futures::{Sink, Stream, Poll, Async, AsyncSink, StartSend};
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum RoomError<I, T, R> {
    UnknownClient(I),
    DisconnectedClient(I, Disconnect<T, R>),
}

/// Handles connection with multiple server clients.
///
/// It value-adds a lot of consistency by being a well-developed group of clients.
/// In my applications it's being immensely helpful to factor out this logic to here.
pub struct Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    ready_clients: HashMap<I, Client<I, C>>,
    gone_clients: HashMap<I, Client<I, C>>,
}

impl<I, C> Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    /// Construct a new `Room` from a list of `Client`.
    ///
    /// N.B., construct an empty room with `Room::default()`.
    pub fn new(clients: Vec<Client<I, C>>) -> Room<I, C> {
        let mut room = Room::default();
        for client in clients {
            room.insert(client);
        }
        room
    }

    /// Get the IDs of all the clients.
    pub fn ids(&self) -> HashSet<I> {
        &self.ready_ids() | &self.gone_ids()
    }

    /// Get the IDs of all connected clients.
    pub fn ready_ids(&self) -> HashSet<I> {
        self.ready_clients.keys().cloned().collect()
    }

    /// Get the IDs of all disconnected clients.
    pub fn gone_ids(&self) -> HashSet<I> {
        self.gone_clients.keys().cloned().collect()
    }

    #[doc(hidden)]
    pub fn ready_client_mut(&mut self, id: &I) -> Option<&mut Client<I, C>> {
        self.ready_clients.get_mut(id)
    }

    // @TODO: Exists only for `Client::join`. When RFC1422 is stable, make this `pub(super)`.
    #[doc(hidden)]
    pub fn insert(&mut self, client: Client<I, C>) -> bool {
        if self.contains(&client.id()) {
            return false;
        }
        if client.status().is_connected() {
            self.ready_clients.insert(client.id(), client);
        } else {
            self.gone_clients.insert(client.id(), client);
        }
        true
    }

    /// Remove a client by ID.
    pub fn remove(&mut self, id: &I) -> Option<Client<I, C>> {
        self.ready_clients.remove(id).or_else(|| self.gone_clients.remove(id))
    }

    /// Check is a client ID is present
    pub fn contains(&self, id: &I) -> bool {
        self.ready_clients.contains_key(id) || self.gone_clients.contains_key(id)
    }

    /// Broadcast a single message to all connected clients.
    pub fn broadcast_all(self, msg: C::SinkItem) -> Broadcast<I, C>
        where C::SinkItem: Clone
    {
        let ids = self.ready_clients.keys().cloned().collect();
        Broadcast::new(self, msg, ids)
    }

    /// Broadcast a single message to particular connected clients.
    pub fn broadcast(self, msg: C::SinkItem, ids: HashSet<I>) -> Broadcast<I, C>
        where C::SinkItem: Clone
    {
        Broadcast::new(self, msg, ids.into_iter().collect())
    }

    /// Send particular messages to particular connected clients.
    pub fn transmit(self, msgs: HashMap<I, C::SinkItem>) -> Transmit<I, C> {
        Transmit::new(self, msgs.into_iter().collect())
    }

    /// Try to receive a single message from all clients.
    pub fn receive_all(self) -> Receive<I, C> {
        let ids = self.ready_clients.keys().cloned().collect();
        Receive::new(self, ids)
    }

    /// Try to receive a single message from particular clients.
    pub fn receive(self, ids: HashSet<I>) -> Receive<I, C> {
        Receive::new(self, ids.into_iter().collect())
    }

    // @TODO: When client has a method to check its status against the internal rx/tx,
    // update this to use it (or make a new method to.)
    /// Get the status of all the clients.
    pub fn statuses(&self) -> HashMap<I, Status<C::SinkError, C::Error>> {
        let rs = self.ready_clients.iter().map(|(id, client)| (id.clone(), client.status()));
        let gs = self.gone_clients.iter().map(|(id, client)| (id.clone(), client.status()));
        rs.chain(gs).collect()
    }

    /// Force disconnection of particular clients.
    pub fn close(&mut self, ids: HashSet<I>) {
        for id in ids {
            let mut client = match self.ready_clients.remove(&id) {
                Some(client) => client,
                None => continue,
            };
            client.close();
            self.gone_clients.insert(id, client);
        }
    }

    /// Disconnect all clients.
    pub fn close_all(&mut self) {
        self.ready_clients.clear();
        self.ready_clients.shrink_to_fit();
        self.gone_clients.clear();
        self.gone_clients.shrink_to_fit();
    }

    pub fn into_clients(self) -> (HashMap<I, Client<I, C>>, HashMap<I, Client<I, C>>) {
        (self.ready_clients, self.gone_clients)
    }
}

impl<I, C> Default for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    fn default() -> Room<I, C> {
        Room {
            ready_clients: HashMap::new(),
            gone_clients: HashMap::new(),
        }
    }
}

impl<I, C> Sink for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type SinkItem = (I, C::SinkItem);
    type SinkError = RoomError<I, C::SinkError, C::Error>;

    fn start_send(&mut self,
                  (id, msg): Self::SinkItem)
                  -> StartSend<Self::SinkItem, Self::SinkError> {
        // Check if the client exists but is disconnected.
        if let Some(gone_client) = self.gone_clients.get_mut(&id) {
            let disconnect = gone_client.status().is_disconnected().unwrap().clone();
            return Err(RoomError::DisconnectedClient(id, disconnect));
        }

        // Check if the client is connected or (having also excluded disconnected) does not.
        let client = match self.ready_client_mut(&id) {
            None => return Err(RoomError::UnknownClient(id)),
            Some(client) => client,
        };

        match client.start_send(msg) {
            Ok(AsyncSink::Ready) => Ok(AsyncSink::Ready),
            Ok(AsyncSink::NotReady(msg)) => Ok(AsyncSink::NotReady((id, msg))),
            Err(e) => Err(RoomError::DisconnectedClient(id, e)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        for client in self.ready_clients.values_mut() {
            match client.poll_complete() {
                Ok(Async::Ready(())) => {}
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => return Err(RoomError::DisconnectedClient(client.id(), e)),
            }
        }
        Ok(Async::Ready(()))
    }
}

impl<I, C> Stream for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (I, C::Item);
    type Error = (I, Disconnect<C::SinkError, C::Error>);

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        for client in self.ready_clients.values_mut() {
            match client.poll() {
                Ok(Async::Ready(Some(msg))) => return Ok(Async::Ready(Some((client.id(), msg)))),
                Ok(Async::Ready(None)) => return Err((client.id(), Disconnect::Dropped)),
                Err(e) => return Err((client.id(), e)),
                Ok(Async::NotReady) => continue,
            }
        }

        Ok(Async::NotReady)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::{executor, Future, Stream};

    #[test]
    fn can_broadcast_all() {
        let (rx0, _, client0) = mock_client("client0", 1);
        let (rx1, _, client1) = mock_client("client1", 1);
        let room = Room::new(vec![client0, client1]);

        room.broadcast_all(TinyMsg::A).wait().unwrap();
        assert_eq!(rx0.into_future().wait().unwrap().0, Some(TinyMsg::A));
        assert_eq!(rx1.into_future().wait().unwrap().0, Some(TinyMsg::A));
    }

    #[test]
    fn can_transmit() {
        let (rx0, _, client0) = mock_client("client0", 1);
        let (rx1, _, client1) = mock_client("client1", 1);

        let mut msgs = HashMap::new();
        msgs.insert(client0.id(), TinyMsg::A);
        msgs.insert(client1.id(), TinyMsg::B("entropy".to_string()));

        let room = Room::new(vec![client0, client1]);
        room.transmit(msgs).wait().unwrap();

        assert_eq!(rx0.into_future().wait().unwrap().0, Some(TinyMsg::A));
        assert_eq!(rx1.into_future().wait().unwrap().0,
                   Some(TinyMsg::B("entropy".to_string())));
    }

    #[test]
    fn can_receive_all() {
        let (_, tx0, client0) = mock_client("client0", 1);
        let (_, tx1, client1) = mock_client("client1", 1);
        let (id0, id1) = (client0.id(), client1.id());
        let room = Room::new(vec![client0, client1]);

        let mut future = executor::spawn(room.receive_all().fuse());
        assert!(future.poll_future(unpark_noop()).unwrap().is_not_ready());

        tx0.send(TinyMsg::A).wait().unwrap();
        tx1.send(TinyMsg::B("abc".to_string())).wait().unwrap();

        let mut exp_msgs = HashMap::new();
        exp_msgs.insert(id0.clone(), TinyMsg::A);
        exp_msgs.insert(id1.clone(), TinyMsg::B("abc".to_string()));

        match future.wait_future() {
            Ok((msgs, room)) => {
                assert_eq!(msgs, exp_msgs);
                assert_eq!(room.statuses()
                               .into_iter()
                               .map(|(id, status)| (id, status.is_ready()))
                               .collect::<HashMap<_, _>>(),
                           vec![(id0, true), (id1, true)].into_iter().collect());
            }
            _ => assert!(false),
        }
    }

    #[test]
    fn can_statuses() {
        let (_, _, client0) = mock_client("client0", 1);
        let (_, _, client1) = mock_client("client1", 1);

        let mut room = Room::new(vec![client0, client1]);
        for (_, status) in room.statuses() {
            assert!(status.is_ready());
        }

        let mut msgs = HashMap::new();
        msgs.insert("client0".to_string(), TinyMsg::A);
        room = room.transmit(msgs).wait().unwrap();
        for (id, status) in room.statuses() {
            if id == "client0".to_string() {
                assert!(status.is_gone().is_some());
            } else {
                assert!(status.is_ready());
            }
        }

        room = room.broadcast_all(TinyMsg::B("abc".to_string())).wait().unwrap();
        for (_, status) in room.statuses() {
            assert!(status.is_gone().is_some());
        }
    }
}
