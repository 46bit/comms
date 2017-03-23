use std::hash::Hash;
use futures::{Future, Sink, Stream, Poll, Async, AsyncSink};
use super::*;

pub struct Transmit<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    room: Option<Room<I, C>>,
    start_send_list: Vec<(I, C::SinkItem)>,
    poll_complete_list: Vec<I>,
}

impl<I, C> Transmit<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(room: Room<I, C>, msgs: Vec<(I, C::SinkItem)>) -> Transmit<I, C> {
        Transmit {
            room: Some(room),
            start_send_list: msgs,
            poll_complete_list: vec![],
        }
    }

    pub fn into_inner(mut self) -> Room<I, C> {
        self.room.take().unwrap()
    }
}

impl<I, C> Future for Transmit<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = Room<I, C>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut room = self.room.take().unwrap();

        let start_send_list = self.start_send_list.drain(..).collect::<Vec<_>>();
        for (id, msg) in start_send_list {
            let ready_client = match room.ready_client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.start_send(msg) {
                Ok(AsyncSink::NotReady(msg)) => self.start_send_list.push((id, msg)),
                Ok(AsyncSink::Ready) => self.poll_complete_list.push(id),
                Err(_) => {}
            }
        }

        let poll_complete_list = self.poll_complete_list.drain(..).collect::<Vec<_>>();
        for id in poll_complete_list {
            let ready_client = match room.ready_client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.poll_complete() {
                Ok(Async::NotReady) => self.poll_complete_list.push(id),
                Ok(Async::Ready(())) | Err(_) => {}
            }
        }

        if self.start_send_list.is_empty() && self.poll_complete_list.is_empty() {
            Ok(Async::Ready(room))
        } else {
            self.room = Some(room);
            Ok(Async::NotReady)
        }
    }
}
