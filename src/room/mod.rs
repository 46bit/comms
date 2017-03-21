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

pub struct Room<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    timeout: Timeout,
    ready_clients: HashMap<I, Client<I, T, R>>,
    gone_clients: HashMap<I, Client<I, T, R>>,
    // Data for Sink
    start_send_list: Vec<(I, T::SinkItem)>,
    poll_complete_list: HashSet<I>,
}

impl<I, T, R> Room<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    pub fn new(clients: Vec<Client<I, T, R>>) -> Room<I, T, R> {
        let mut room = Room::default();
        for client in clients {
            room.insert(client);
        }
        room
    }

    pub fn timeout(&self) -> &Timeout {
        &self.timeout
    }

    pub fn set_timeout(&mut self, timeout: Timeout) {
        for client in self.ready_clients.values_mut() {
            client.set_timeout(timeout.clone());
        }
        self.timeout = timeout;
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

    #[doc(hidden)]
    pub fn ready_client_mut(&mut self, id: &I) -> Option<&mut Client<I, T, R>> {
        self.ready_clients.get_mut(id)
    }

    // @TODO: Exists only for `Client::join`. When RFC1422 is stable, make this `pub(super)`.
    #[doc(hidden)]
    pub fn insert(&mut self, mut client: Client<I, T, R>) -> bool {
        if self.contains(&client.id()) {
            return false;
        }
        client.set_timeout(self.timeout.clone());
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

    pub fn broadcast(self, msg: T::SinkItem) -> Broadcast<I, T, R>
        where T::SinkItem: Clone
    {
        Broadcast::new(self, msg)
    }

    pub fn transmit(self, msgs: HashMap<I, T::SinkItem>) -> Transmit<I, T, R> {
        Transmit::new(self, msgs.into_iter().collect())
    }

    pub fn receive_all(self) -> Receive<I, T, R> {
        let ids = self.ready_clients.keys().cloned().collect();
        Receive::new(self, ids)
    }

    pub fn receive(self, ids: HashSet<I>) -> Receive<I, T, R> {
        Receive::new(self, ids.into_iter().collect())
    }

    // @TODO: When client has a method to check its status against the internal rx/tx,
    // update this to use it (or make a new method to.)
    pub fn statuses(&self) -> HashMap<I, Status<T::SinkError, R::Error>> {
        let rs = self.ready_clients.iter().map(|(id, client)| (id.clone(), client.status()));
        let gs = self.gone_clients.iter().map(|(id, client)| (id.clone(), client.status()));
        rs.chain(gs).collect()
    }

    pub fn close(&mut self) {
        self.ready_clients.clear();
        self.ready_clients.shrink_to_fit();
        self.gone_clients.clear();
        self.gone_clients.shrink_to_fit();
    }

    pub fn into_clients(self) -> (HashMap<I, Client<I, T, R>>, HashMap<I, Client<I, T, R>>) {
        (self.ready_clients, self.gone_clients)
    }
}

impl<I, T, R> Default for Room<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    fn default() -> Room<I, T, R> {
        Room {
            timeout: Timeout::None,
            ready_clients: HashMap::new(),
            gone_clients: HashMap::new(),
            start_send_list: vec![],
            poll_complete_list: HashSet::new(),
        }
    }
}

impl<I, T, R> Sink for Room<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type SinkItem = Vec<(I, T::SinkItem)>;
    type SinkError = ();

    fn start_send(&mut self, msgs: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.start_send_list.extend(msgs);
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        let start_send_list = self.start_send_list.drain(..).collect::<Vec<_>>();
        for (id, msg) in start_send_list {
            let start_send = match self.ready_client_mut(&id) {
                Some(ready_client) => ready_client.start_send(msg),
                None => continue,
            };
            match start_send {
                Ok(AsyncSink::NotReady(msg)) => {
                    self.start_send_list.push((id, msg));
                }
                Ok(AsyncSink::Ready) => {
                    self.poll_complete_list.insert(id);
                }
                Err(_) => {}
            }
        }

        let poll_complete_list = self.poll_complete_list.drain().collect::<Vec<_>>();
        for id in poll_complete_list {
            let poll_complete = match self.ready_client_mut(&id) {
                Some(ready_client) => ready_client.poll_complete(),
                None => continue,
            };
            match poll_complete {
                Ok(Async::NotReady) => {
                    self.poll_complete_list.insert(id);
                }
                Ok(Async::Ready(())) | Err(_) => {}
            }
        }

        if self.start_send_list.is_empty() && self.poll_complete_list.is_empty() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl<I, T, R> Stream for Room<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type Item = Vec<(I, R::Item)>;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut msgs = vec![];
        for client in self.ready_clients.values_mut() {
            loop {
                match client.poll() {
                    Ok(Async::NotReady) => break,
                    Ok(Async::Ready(Some(Some(msg)))) => msgs.push((client.id(), msg)),
                    Ok(Async::Ready(Some(None))) |
                    Ok(Async::Ready(None)) |
                    Err(_) => {}
                }
            }
        }

        if msgs.is_empty() {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(Some(msgs)))
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
