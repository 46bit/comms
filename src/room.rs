use std::fmt::Debug;
use std::collections::HashMap;
use futures::{future, Future, BoxFuture, Sink, Stream};

use super::*;

#[derive(Clone)]
pub struct Room<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    clients: HashMap<I, Client<I, T, R>>,
}

impl<I, T, R> Room<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    pub fn new(clients: Vec<Client<T, R>>) -> Room<T, R> {
        let clients = clients.into_iter().map(|c| (c.id(), c)).collect();
        Room { clients: clients }
    }

    pub fn ids(&self) -> Vec<I> {
        self.clients.keys().cloned().collect()
    }

    pub fn into_clients(self) -> HashMap<I, Client<I, T, R>> {
        self.clients
    }

    // @TODO: Exists only for `Client::join`. When RFC1422 is stable, make this `pub(super)`.
    #[doc(hidden)]
    pub fn insert(&mut self, client: Client<I, T, R>) -> bool {
        if self.contains(&client.id()) {
            return false;
        }
        self.clients.insert(client.id(), client);
        true
    }

    pub fn remove(&mut self, id: &I) -> Option<Client<I, T, R>> {
        self.clients.remove(id)
    }

    pub fn contains(&self, id: &I) -> bool {
        self.clients.contains_key(id)
    }

    pub fn name_of(&self, id: &I) -> Option<String> {
        self.clients.get(id).and_then(|c| c.name())
    }

    pub fn filter_by_ids<F>(&self, ids: Vec<&I>) -> FilteredRoom<I, T, R>
        where F: FnMut(&Client<T, R>) -> bool,
              T: Clone,
              R: Clone
    {
        FilteredRoom {
            room_inner: self.inner.clone(),
            client_ids: ids
        }
    }

    /// Filter the room
    pub fn filter<F>(&mut self, mut f: F) -> FilteredRoom<T, R>
        where F: FnMut(&Client<T, R>) -> bool,
              T: Clone,
              R: Clone
    {
        WheredRoom {
            room: &mut self,
            f: f
        }
    }

    fn client_mut(&mut self, id: &I) -> Option<&mut Client<T, R>> {
        self.clients.get_mut(id)
    }
}

impl<I, T, R> Default for Room<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    fn default() -> Room<T, R> {
        Room { clients: HashMap::new() }
    }
}

impl<I, T, R> Communicator for Room<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    type Transmit = HashMap<I, T>;
    type Receive = (HashMap<I, ClientStatus>, HashMap<I, R>);
    type Status = HashMap<I, ClientStatus>;
    type Error = ();

    fn transmit(&mut self, msgs: Self::Transmit) -> BoxFuture<Self::Status, ()> {
        let client_futures = msgs.into_iter()
            .filter_map(|(id, msg)| self.client_mut(&id).map(|client| client.transmit(msg)))
            .collect::<Vec<_>>();
        future::join_all(client_futures).map(|results| results.into_iter().collect()).boxed()
    }

    fn receive(&mut self, timeout: ClientTimeout) -> BoxFuture<Self::Receive, ()> {
        self.communicate_with_all_clients(|client| client.receive(timeout))
            .map(|results| {
                let mut statuses = HashMap::new();
                let mut msgs = HashMap::new();
                for (id, status, msg) in results {
                    statuses.insert(id, status);
                    msg.and_then(|msg| msgs.insert(id, msg));
                }
                (statuses, msgs)
            })
            .boxed()
    }

    fn status(&mut self) -> BoxFuture<Self::Status, ()> {
        self.communicate_with_all_clients(|client| client.status())
            .map(|results| results.into_iter().collect())
            .boxed()
    }

    fn close(&mut self) -> BoxFuture<Self::Status, ()> {
        self.communicate_with_all_clients(|client| client.close())
            .map(|results| results.into_iter().collect())
            .boxed()
    }
}

pub trait Broadcasting {
    type M;
    type Status;

    fn broadcast(&mut self, msg: Self::M) -> BoxFuture<(Self::Status, Self::Status), ()>;
}

impl<I, T, R> Broadcasting for (Room<I, T, R>, Room<I, T, R>)
    where I: Clone + Send + Debug + 'static,
          T: Clone + Send + 'static,
          R: Send + 'static
{
    type M = T;
    type Status = <Room<T, R> as Communicator>::Status;

    fn broadcast(&mut self, msg: T) -> BoxFuture<(Self::Status, Self::Status), ()> {
        self.0.broadcast(msg.clone()).join(self.1.broadcast(msg)).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::Stream;

    #[test]
    fn can_transmit() {
        let (rx0, client0) = mock_client_channelled();
        let mut client0_rx = rx0.wait().peekable();
        let client0_id = client0.id();

        let (rx1, client1) = mock_client_channelled();
        let mut client1_rx = rx1.wait().peekable();
        let client1_id = client1.id();

        let mut room = Room::new(vec![client0, client1]);

        let mut msgs = HashMap::new();
        msgs.insert(client0_id, TinyMsg::A);
        msgs.insert(client1_id, TinyMsg::B("entropy".to_string()));
        room.transmit(msgs).wait().unwrap();
        match (client0_rx.next(), client1_rx.next()) {
            (Some(Ok(_)), Some(Ok(_))) => {}
            _ => assert!(false),
        }
    }
}
