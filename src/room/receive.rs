use std::hash::Hash;
use std::collections::{HashSet, HashMap};
use futures::{Future, Sink, Stream, Poll, Async};
use super::*;

pub struct Receive<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    room: Option<Room<I, C>>,
    poll_list: HashSet<I>,
    replies: Vec<(I, C::Item)>,
}

impl<I, C> Receive<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    #[doc(hidden)]
    pub fn new(room: Room<I, C>, ids: HashSet<I>) -> Receive<I, C> {
        Receive {
            room: Some(room),
            poll_list: ids,
            replies: vec![],
        }
    }

    // @TODO: Return a struct so the present replies are labelled.
    pub fn into_inner(mut self) -> (Room<I, C>, Vec<(I, C::Item)>) {
        (self.room.take().unwrap(), self.replies)
    }
}

impl<I, C> Future for Receive<I, C>
    where I: Clone + Send + PartialEq + Eq + Hash + Debug + 'static,
          C: Sink + Stream + 'static
{
    type Item = (HashMap<I, C::Item>, Room<I, C>);
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut room = self.room.take().unwrap();

        let poll_list = self.poll_list.drain().collect::<Vec<_>>();
        for id in poll_list {
            let ready_client = match room.client_mut(&id) {
                Some(ready_client) => ready_client,
                None => continue,
            };
            match ready_client.poll() {
                Ok(Async::NotReady) => {
                    self.poll_list.insert(id);
                }
                Ok(Async::Ready(Some(msg))) => self.replies.push((id, msg)),
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
