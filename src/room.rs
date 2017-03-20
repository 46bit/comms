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

    pub fn into_clients(self) -> HashMap<I, Client<I, T, R>> {
        self.clients
    }

    fn client_mut(&mut self, id: &I) -> Option<&mut Client<T, R>> {
        self.clients.get_mut(id)
    }

    pub fn ids(&self) -> Vec<I> {
        self.clients.keys().cloned().collect()
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

    pub fn filter<F>(&self, ids: Vec<&I>) -> FilteredRoom<I, T, R>
        where F: FnMut(&Client<T, R>) -> bool,
              T: Clone,
              R: Clone
    {
        FilteredRoom {
            room_inner: self.inner.clone(),
            client_ids: ids
        }
    }

    pub fn broadcast(self, msg: T::Item) -> BoxFuture<Item = Self, > {
        let client_futures = self.clients.into_iter().map(|(id, client)| {
            client.transmit(msg.clone()).then(|result| (id, result))
        }).collect();
        join_all(client_futures).map(||)
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
