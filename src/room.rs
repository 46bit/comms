use std::fmt::Debug;
use std::hash::Hash;
use std::collections::{HashMap, HashSet};
use futures::{future, Future, Sink, Stream};

use super::*;

pub struct Room<I, T, R>
    where I: Clone + Send + Debug + PartialEq + Eq + Hash + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    ready_clients: HashMap<I, Client<I, T, R>>,
    gone_clients: HashMap<I, Client<I, T, R>>,
}

impl<I, T, R> Room<I, T, R>
    where I: Clone + Send + Debug + PartialEq + Eq + Hash + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    pub fn new(clients: Vec<Client<I, T, R>>) -> Room<I, T, R> {
        let mut room = Room::default();
        for client in clients {
            room.insert(client);
        }
        room
    }

    pub fn into_clients(self) -> (HashMap<I, Client<I, T, R>>, HashMap<I, Client<I, T, R>>) {
        (self.ready_clients, self.gone_clients)
    }

    pub fn ids(&self) -> HashSet<I> {
        &self.ready_ids() | &self.gone_ids()
    }

    pub fn ready_ids(&self) -> HashSet<I> {
        self.ready_clients.keys().cloned().collect()
    }

    pub fn gone_ids(&self) -> HashSet<I> {
        self.gone_clients.keys().cloned().collect()
    }

    // @TODO: Exists only for `Client::join`. When RFC1422 is stable, make this `pub(super)`.
    #[doc(hidden)]
    pub fn insert(&mut self, client: Client<I, T, R>) -> bool {
        if self.contains(&client.id()) {
            return false;
        }
        if client.status().ready() {
            self.ready_clients.insert(client.id(), client);
        } else {
            self.gone_clients.insert(client.id(), client);
        }
        true
    }

    pub fn remove(&mut self, id: &I) -> Option<Client<I, T, R>> {
        self.ready_clients.remove(id).or_else(|| self.gone_clients.remove(id))
    }

    pub fn contains(&self, id: &I) -> bool {
        self.ready_clients.contains_key(id) || self.gone_clients.contains_key(id)
    }

    // pub fn filter<F>(&self, ids: Vec<&I>) -> FilteredRoom<I, T, R>
    //     where F: FnMut(&Client<T, R>) -> bool,
    //           T: Clone,
    //           R: Clone
    // {
    //     FilteredRoom {
    //         room_inner: self.inner.clone(),
    //         client_ids: ids,
    //     }
    // }

    pub fn broadcast(mut self, msg: T::SinkItem) -> Box<Future<Item = Self, Error = ()>>
        where T::SinkItem: Clone
    {
        let futures =
            self.ready_clients
                .drain()
                .map(|(id, client)| {
                    client.transmit(msg.clone()).then(|result| future::ok((id, result)))
                })
                .collect::<Vec<_>>();
        Box::new(future::join_all(futures).map(|results| {
            for (id, result) in results {
                match result {
                    Ok(ready_client) => self.ready_clients.insert(id, ready_client),
                    Err(gone_client) => self.gone_clients.insert(id, gone_client),
                };
            }
            self
        }))
    }
}

impl<I, T, R> Default for Room<I, T, R>
    where I: Clone + Send + Debug + PartialEq + Eq + Hash + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    fn default() -> Room<I, T, R> {
        Room {
            ready_clients: HashMap::new(),
            gone_clients: HashMap::new(),
        }
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
