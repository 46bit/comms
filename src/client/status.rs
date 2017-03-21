use std::hash::Hash;
use futures::{Future, Sink, Stream, Poll, Async, AsyncSink};
use super::*;

pub struct Status<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    client: Option<Client<I, T, R>>,
}

impl<I, T, R> Status<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    pub fn new(client: Client<I, T, R>) -> Status<I, T, R> {
        Status {
            client: Some(client),
        }
    }

    pub fn into_inner(mut self) -> Room<I, T, R> {
        self.client.take().unwrap()
    }
}

impl<I, T, R> Future for Status<I, T, R>
    where I: Clone + Send + PartialEq + Eq + Hash + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type Item = (Status, Client<I, T, R>);
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut client = self.client.take().unwrap();

        self.client.

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
