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

    pub fn transmit(mut self,
                    msgs: HashMap<I, T::SinkItem>)
                    -> Box<Future<Item = Self, Error = ()>> {
        let futures = msgs.into_iter()
            .filter_map(|(id, msg)| {
                self.ready_clients
                    .remove(&id)
                    .map(|client| client.transmit(msg).then(|result| future::ok((id, result))))
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

    // @TODO: When client has a method to check its status against the internal rx/tx,
    // update this to use it (or make a new method to.)
    pub fn statuses(&self) -> HashMap<I, ClientStatus<T::SinkError, R::Error>> {
        let rs = self.ready_clients.iter().map(|(id, client)| (id.clone(), client.status()));
        let gs = self.gone_clients.iter().map(|(id, client)| (id.clone(), client.status()));
        rs.chain(gs).collect()
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
    fn can_broadcast() {
        let (rx0, _, client0) = mock_client("client0", 1);
        let (rx1, _, client1) = mock_client("client1", 1);
        let room = Room::new(vec![client0, client1]);

        room.broadcast(TinyMsg::A).wait().unwrap();
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
    fn can_statuses() {
        let (_, _, client0) = mock_client("client0", 1);
        let (_, _, client1) = mock_client("client1", 1);

        let mut room = Room::new(vec![client0, client1]);
        for (_, status) in room.statuses() {
            assert!(status.ready());
        }

        let mut msgs = HashMap::new();
        msgs.insert("client0".to_string(), TinyMsg::A);
        room = room.transmit(msgs).wait().unwrap();
        for (id, status) in room.statuses() {
            if id == "client0".to_string() {
                assert!(status.gone().is_some());
            } else {
                assert!(status.ready());
            }
        }

        room = room.broadcast(TinyMsg::B("abc".to_string())).wait().unwrap();
        for (_, status) in room.statuses() {
            assert!(status.gone().is_some());
        }
    }
}
