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

    pub fn receive(mut self,
                   timeout: ClientTimeout<'static>)
                   -> Box<Future<Item = (HashMap<I, R::Item>, Self), Error = ()>> {
        let futures = self.ready_clients
            .drain()
            .map(|(id, client)| client.receive(timeout).then(|v| future::ok((id, v))))
            .collect::<Vec<_>>();
        Box::new(future::join_all(futures).map(|results| {
            let mut msgs = HashMap::new();
            for (id, result) in results {
                match result {
                    Ok((maybe_msg, ready_client)) => {
                        if let Some(msg) = maybe_msg {
                            msgs.insert(id.clone(), msg);
                        }
                        self.ready_clients.insert(id, ready_client);
                    }
                    Err(gone_client) => {
                        self.gone_clients.insert(id, gone_client);
                    }
                }
            }
            (msgs, self)
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
    use futures::{executor, Future, Stream};

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
    fn can_receive() {
        let (_, tx0, client0) = mock_client("client0", 1);
        let (_, tx1, client1) = mock_client("client1", 1);
        let (id0, id1) = (client0.id(), client1.id());
        let room = Room::new(vec![client0, client1]);

        let timeout = ClientTimeout::None;
        let mut future = executor::spawn(room.receive(timeout).fuse());
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
                               .map(|(id, status)| (id, status.ready()))
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
