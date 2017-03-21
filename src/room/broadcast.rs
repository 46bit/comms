use std::hash::Hash;
use futures::{Future, Sink, Stream, Poll, Async, AsyncSink};
use super::*;

pub struct Broadcast<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkItem: Clone,
          T::SinkError: Clone,
          R::Error: Clone
{
    room: Option<Room<I, T, R>>,
    msg: T::SinkItem,
    start_send_list: Vec<I>,
    poll_complete_list: Vec<I>,
}

impl<I, T, R> Broadcast<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkItem: Clone,
          T::SinkError: Clone,
          R::Error: Clone
{
    pub fn new(room: Room<I, T, R>, msg: T::SinkItem) -> Broadcast<I, T, R> {
        let ready_client_ids = room.ready_ids().into_iter().collect();
        Broadcast {
            room: Some(room),
            msg: msg,
            start_send_list: ready_client_ids,
            poll_complete_list: vec![],
        }
    }

    pub fn into_inner(mut self) -> Room<I, T, R> {
        self.room.take().unwrap()
    }
}

impl<I, T, R> Future for Broadcast<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkItem: Clone,
          T::SinkError: Clone,
          R::Error: Clone
{
    type Item = Room<I, T, R>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut room = self.room.take().unwrap();

        let start_send_list = self.start_send_list.drain(..).collect::<Vec<_>>();
        for id in start_send_list {
            let ready_client = match room.ready_client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.start_send(self.msg.clone()) {
                Ok(AsyncSink::NotReady(_)) => {
                    self.start_send_list.push(id);
                }
                Ok(AsyncSink::Ready) => {
                    self.poll_complete_list.push(id);
                }
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
                Ok(Async::NotReady) => {
                    self.poll_complete_list.push(id);
                }
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
