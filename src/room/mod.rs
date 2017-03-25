mod transmit;
mod broadcast;
mod receive;

pub use self::transmit::*;
pub use self::broadcast::*;
pub use self::transmit::*;
pub use self::receive::*;

use std::fmt::Debug;
use std::hash::Hash;
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::vec;
use futures::{Sink, Stream, Poll, Async, AsyncSink, StartSend};
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum RoomError<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static
{
    UnknownClient(I),
    DisconnectedClient(I, Disconnect<C::SinkError, C::Error>),
}

/// Handles connection with multiple server clients.
///
/// It value-adds a lot of consistency by being a well-developed group of clients.
/// In my applications it's being immensely helpful to factor out this logic to here.
pub struct Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    clients: HashMap<I, Client<I, C>>,
}

impl<I, C> Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
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

    /// Get the IDs of all connected clients.
    pub fn ids(&self) -> HashSet<I> {
        self.clients.keys().cloned().collect()
    }

    pub fn client(&self, id: &I) -> Option<&Client<I, C>> {
        self.clients.get(id)
    }

    #[doc(hidden)]
    pub fn client_mut(&mut self, id: &I) -> Option<&mut Client<I, C>> {
        self.clients.get_mut(id)
    }

    /// Insert a `Client` into this room and return whether it was newly added.
    ///
    /// For now this accepts disconnected clients but this may change in future.
    pub fn insert(&mut self, client: Client<I, C>) -> bool {
        if self.contains(&client.id()) {
            return false;
        }
        self.clients.insert(client.id(), client);
        true
    }

    /// Remove a client by ID.
    pub fn remove(&mut self, id: &I) -> Option<Client<I, C>> {
        self.clients.remove(id)
    }

    /// Check is a client ID is present
    pub fn contains(&self, id: &I) -> bool {
        self.clients.contains_key(id)
    }

    /// Broadcast a single message to all connected clients.
    pub fn broadcast_all(self, msg: C::SinkItem) -> Broadcast<I, C>
        where C::SinkItem: Clone
    {
        let ids = self.ids();
        Broadcast::new(self, msg, ids)
    }

    /// Broadcast a single message to particular connected clients.
    pub fn broadcast(self, msg: C::SinkItem, ids: HashSet<I>) -> Broadcast<I, C>
        where C::SinkItem: Clone
    {
        Broadcast::new(self, msg, ids)
    }

    /// Send particular messages to particular connected clients.
    pub fn transmit(self, msgs: HashMap<I, C::SinkItem>) -> Transmit<I, C> {
        Transmit::new(self, msgs.into_iter().collect())
    }

    /// Try to receive a single message from all clients.
    pub fn receive_all(self) -> Receive<I, C> {
        let ids = self.ids();
        Receive::new(self, ids)
    }

    /// Try to receive a single message from particular clients.
    pub fn receive(self, ids: HashSet<I>) -> Receive<I, C> {
        Receive::new(self, ids.into_iter().collect())
    }

    /// Force disconnection of particular clients.
    pub fn close(&mut self, ids: HashSet<I>) {
        for id in ids {
            let client = match self.client_mut(&id) {
                Some(client) => client,
                None => continue,
            };
            client.close();
        }
    }

    /// Disconnect all clients.
    pub fn close_all(&mut self) {
        self.clients.clear();
        self.clients.shrink_to_fit();
    }

    pub fn into_clients(self) -> HashMap<I, Client<I, C>> {
        self.clients
    }

    fn err_for_disconnect(&mut self, id: I) -> RoomError<I, C> {
        let (id, disconnect) = self.clients.remove(&id).unwrap().into_disconnect();
        RoomError::DisconnectedClient(id, disconnect)
    }
}

impl<I, C> Default for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    fn default() -> Room<I, C> {
        Room { clients: HashMap::new() }
    }
}

impl<I, C> Sink for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    type SinkItem = (I, C::SinkItem);
    type SinkError = RoomError<I, C>;

    fn start_send(&mut self,
                  (id, msg): Self::SinkItem)
                  -> StartSend<Self::SinkItem, Self::SinkError> {
        let client_poll = {
            // Check if the client is connected or (having also excluded disconnected) does not.
            let client = match self.client_mut(&id) {
                None => return Err(RoomError::UnknownClient(id)),
                Some(client) => client,
            };
            client.start_send(msg)
        };

        match client_poll {
            Ok(AsyncSink::Ready) => Ok(AsyncSink::Ready),
            Ok(AsyncSink::NotReady(msg)) => Ok(AsyncSink::NotReady((id.clone(), msg))),
            Err(()) => {
                return Err(self.err_for_disconnect(id));
            }
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        for id in self.clients.keys().cloned().collect::<Vec<I>>() {
            let poll_complete = {
                let client = self.clients.get_mut(&id).unwrap();
                client.poll_complete()
            };
            match poll_complete {
                Ok(Async::Ready(())) => {}
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(()) => {
                    return Err(self.err_for_disconnect(id));
                }
            }
        }
        Ok(Async::Ready(()))
    }
}

impl<I, C> Stream for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    type Item = (I, C::Item);
    type Error = RoomError<I, C>;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        for id in self.clients.keys().cloned().collect::<Vec<I>>() {
            let poll = {
                let client = self.clients.get_mut(&id).unwrap();
                client.poll()
            };
            match poll {
                Ok(Async::Ready(Some(msg))) => return Ok(Async::Ready(Some((id, msg)))),
                Ok(Async::Ready(None)) => {
                    return Err(self.err_for_disconnect(id));
                }
                Err(()) => {
                    return Err(self.err_for_disconnect(id));
                }
                Ok(Async::NotReady) => continue,
            }
        }

        Ok(Async::NotReady)
    }
}

impl<I, C> IntoIterator for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    type Item = Client<I, C>;
    type IntoIter = vec::IntoIter<Client<I, C>>;

    fn into_iter(self) -> Self::IntoIter {
        self.clients.into_iter().map(|(_, client)| client).collect::<Vec<_>>().into_iter()
    }
}

impl<I, C> FromIterator<Client<I, C>> for Room<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    fn from_iter<J: IntoIterator<Item = Client<I, C>>>(iter: J) -> Room<I, C> {
        Room::new(iter.into_iter().collect())
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
                assert_eq!(room.into_iter()
                               .map(|client| (client.id(), client.is_connected()))
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
        for id in room.ids() {
            assert!(room.client(&id).unwrap().is_connected());
        }

        let mut msgs = HashMap::new();
        msgs.insert("client0".to_string(), TinyMsg::A);
        room = room.transmit(msgs).wait().unwrap();
        for id in room.ids() {
            let client = room.client(&id).unwrap();
            if id == "client0".to_string() {
                assert!(client.is_disconnected().is_some());
            } else {
                assert!(client.is_connected());
            }
        }

        room = room.broadcast_all(TinyMsg::B("abc".to_string())).wait().unwrap();
        for id in room.ids() {
            let client = room.client(&id).unwrap();
            assert!(client.is_disconnected().is_some());
        }
    }
}
