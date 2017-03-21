use std::hash::Hash;
use std::collections::HashMap;
use futures::{Future, Sink, Stream, Poll, Async};
use super::*;

pub struct Receive<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    room: Option<Room<I, T, R>>,
    poll_list: Vec<I>,
    replies: Vec<(I, R::Item)>,
}

impl<I, T, R> Receive<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    pub fn new(room: Room<I, T, R>, ids: Vec<I>) -> Receive<I, T, R> {
        Receive {
            room: Some(room),
            poll_list: ids,
            replies: vec![],
        }
    }

    // @TODO: Return a struct so the present replies are labelled.
    pub fn into_inner(mut self) -> (Room<I, T, R>, Vec<(I, R::Item)>) {
        (self.room.take().unwrap(), self.replies)
    }
}

impl<I, T, R> Future for Receive<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type Item = (HashMap<I, R::Item>, Room<I, T, R>);
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut room = self.room.take().unwrap();

        let poll_list = self.poll_list.drain(..).collect::<Vec<_>>();
        for id in poll_list {
            let ready_client = match room.ready_client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.poll() {
                Ok(Async::NotReady) => self.poll_list.push(id),
                Ok(Async::Ready(Some(Some(msg)))) => self.replies.push((id, msg)),
                Ok(Async::Ready(Some(None))) |
                Ok(Async::Ready(None)) |
                Err(_) => {}
            }
        }

        if self.poll_list.is_empty() {
            let replies_hashmap: HashMap<_, _> = self.replies.drain(..).collect();
            Ok(Async::Ready((replies_hashmap, room)))
        } else {
            self.room = Some(room);
            Ok(Async::NotReady)
        }
    }
}
