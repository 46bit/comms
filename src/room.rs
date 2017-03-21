use std::hash::Hash;
use std::collections::{HashMap, HashSet};
use futures::{future, sink, Future, Sink, Stream, Poll, Async, AsyncSink, StartSend};

use super::*;

pub struct Room<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    timeout: ClientTimeout,
    ready_clients: HashMap<I, Client<I, T, R>>,
    gone_clients: HashMap<I, Client<I, T, R>>,
    // Data for Sink
    start_send_list: Vec<(I, T::SinkItem)>,
    poll_complete_list: Vec<I>,
    // Data for Stream
    poll_list: Vec<I>,
    poll_replies: HashMap<I, R::Item>,
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

    pub fn timeout(&self) -> &ClientTimeout {
        &self.timeout
    }

    pub fn set_timeout(&mut self, timeout: ClientTimeout) {
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

    pub fn broadcast(self, msg: T::SinkItem) -> sink::Send<Self>
        where T::SinkItem: Clone
    {
        // @TODO: Lots of unnecessary allocations, but the alternative is a lot of code.
        let msgs = self.ready_clients.keys().cloned().map(|id| (id, msg.clone())).collect();
        self.send(msgs)
    }

    pub fn transmit(self, msgs: HashMap<I, T::SinkItem>) -> sink::Send<Self> {
        self.send(msgs)
    }

    pub fn receive(self) -> Box<Future<Item = (HashMap<I, R::Item>, Self), Error = ()>> {
        Box::new(self.into_future()
            .map(|(maybe_msgs, self_)| (maybe_msgs.unwrap(), self_))
            .map_err(|_| ()))
    }

    pub fn receive_from(mut self,
                        ids: Vec<I>)
                        -> Option<Box<Future<Item = (HashMap<I, R::Item>, Self), Error = ()>>> {
        if !self.poll_list.is_empty() || !self.poll_replies.is_empty() {
            return None;
        }
        self.poll_list = ids;
        Some(Box::new(self.into_future()
            .map(|(maybe_msgs, self_)| (maybe_msgs.unwrap(), self_))
            .map_err(|_| ())))
    }

    // @TODO: When client has a method to check its status against the internal rx/tx,
    // update this to use it (or make a new method to.)
    pub fn statuses(&self) -> HashMap<I, ClientStatus<T::SinkError, R::Error>> {
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
            timeout: ClientTimeout::None,
            ready_clients: HashMap::new(),
            gone_clients: HashMap::new(),
            start_send_list: vec![],
            poll_complete_list: vec![],
            poll_list: vec![],
            poll_replies: HashMap::new(),
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
    type SinkItem = HashMap<I, T::SinkItem>;
    type SinkError = ();

    fn start_send(&mut self, msgs: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        // To batch replies properly it's easier to do this than do something deeper.
        if !self.start_send_list.is_empty() || !self.poll_complete_list.is_empty() {
            return Ok(AsyncSink::NotReady(msgs));
        }

        for (id, msg) in msgs {
            // @TODO: Error type for clients not in ready_clients+gone_clients.
            if self.ready_clients.contains_key(&id) {
                self.start_send_list.push((id, msg));
            }
        }
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        // @TODO: This uses some very nasty operations to get around borrowing self.
        let start_send_list = self.start_send_list
            .drain(..)
            .collect::<Vec<_>>();
        let start_send_list = start_send_list.into_iter()
            .filter_map(|(id, msg)| match self.ready_clients.get_mut(&id) {
                Some(client) => {
                    match client.start_send(msg) {
                        Ok(AsyncSink::NotReady(msg)) => Some((id, msg)),
                        Ok(AsyncSink::Ready) => {
                            self.poll_complete_list.push(id);
                            None
                        }
                        Err(_) => None,
                    }
                }
                None => None,
            })
            .collect();
        self.start_send_list = start_send_list;

        // @TODO: This uses some very nasty operations to get around borrowing self.
        let poll_complete_list = self.poll_complete_list
            .drain(..)
            .collect::<Vec<_>>();
        let poll_complete_list = poll_complete_list.into_iter()
            .filter(|id| match self.ready_clients.get_mut(id) {
                Some(client) => {
                    match client.poll_complete() {
                        Ok(Async::NotReady) => true,
                        Ok(Async::Ready(())) | Err(_) => false,
                    }
                }
                None => false,
            })
            .collect();
        self.poll_complete_list = poll_complete_list;

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
    type Item = HashMap<I, R::Item>;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // @TODO: The approach here is very brittle. It's as follows:
        // - Polling should return a full set of client responses.
        // - Clients added during the process might mean it never actually finishes.
        // - As such we take the ready_clients existing when first polled, and only
        //   receive from those.
        // This is very brittle and very potentially surprising. It might be best to
        // allow the never actually finishing possibility but very clearly document the
        // potential problem.
        if self.poll_list.is_empty() {
            self.poll_list.extend(self.ready_clients.keys().cloned());
        }

        // @TODO: This uses some very nasty operations to get around borrowing self.
        let poll_list = self.poll_list
            .drain(..)
            .collect::<Vec<_>>();
        let poll_list = poll_list.into_iter()
            .filter(|id| match self.ready_clients.get_mut(id) {
                Some(client) => {
                    match client.poll() {
                        Ok(Async::NotReady) => true,
                        // Client yielded a value or a non-disconnecting timeout was reached.
                        Ok(Async::Ready(Some(maybe_msg))) => {
                            if let Some(msg) = maybe_msg {
                                self.poll_replies.insert(id.clone(), msg);
                            }
                            false
                        }
                        // Client stream ended or temporary error.
                        Ok(Async::Ready(None)) |
                        Err(_) => false,
                    }
                }
                None => false,
            })
            .collect();
        self.poll_list = poll_list;

        if self.poll_list.is_empty() {
            Ok(Async::Ready(Some(self.poll_replies.drain().collect())))
        } else {
            Ok(Async::NotReady)
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

        let mut future = executor::spawn(room.receive().fuse());
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
