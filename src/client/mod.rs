use std::hash::Hash;
use std::fmt::{self, Debug};
use futures::{future, Future, Sink, Stream, Poll, Async, AsyncSink, StartSend};
use tokio_timer;
use super::*;

struct ClientInner<T, R>
    where T: Sink + 'static,
          R: Stream + 'static
{
    tx: T,
    rx: Peekable<R>,
    sleep: Option<tokio_timer::Sleep>,
}

impl<T, R> Debug for ClientInner<T, R>
    where T: Sink + 'static,
          R: Stream + 'static
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "ClientInner {{ tx: Sink, rx: Stream, sleep: tokio_timer::Sleep }}")
    }
}

#[derive(Debug)]
pub struct Client<I, T, R>
    where I: Clone + Send + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    id: I,
    timeout: Timeout,
    inner: Result<ClientInner<T, R>, Error<T::SinkError, R::Error>>,
}

impl<I, T, R> Client<I, T, R>
    where I: Clone + Send + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    pub fn new(id: I, timeout: Timeout, tx: T, rx: R) -> Client<I, T, R> {
        Client {
            id: id,
            timeout: timeout,
            inner: Ok(ClientInner {
                tx: tx,
                rx: rx,
                sleep: None,
            }),
        }
    }

    pub fn id(&self) -> I {
        self.id.clone()
    }

    pub fn rename<J>(self, new_id: J) -> Client<J, T, R>
        where J: Clone + Send + 'static
    {
        Client {
            id: new_id,
            timeout: self.timeout,
            inner: self.inner,
        }
    }

    pub fn timeout(&self) -> &Timeout {
        &self.timeout
    }

    pub fn set_timeout(&mut self, timeout: Timeout) {
        self.timeout = timeout;
    }

    pub fn join(self, room: &mut Room<I, T, R>) -> bool
        where I: PartialEq + Eq + Hash
    {
        room.insert(self)
    }

    pub fn transmit(self, msg: T::SinkItem) -> Box<Future<Item = Self, Error = Self>> {
        // N.B. This does ensure the inner is set, but it's a bit of a hack.
        let id = self.id();
        let timeout = self.timeout().clone();
        Box::new(self.send(msg).map_err(|e| {
            Client {
                id: id,
                timeout: timeout,
                inner: Err(e.clone()),
            }
        }))
    }

    // We\re mapping everything to Ok.
    // But if no message is provided, Timeout::DisconnectAfter will be Ok but closed.
    // Deliberate aim: no more abstraction; if anything the crate will remove abstraction.
    // Or we map to Ok if the client is still connected. Yes, this.
    // if msg received
    //     Ok((Some(msg), client))
    // elsif Timeout::None
    //     does not terminate
    // elsif Timeout::DisconnectAfter
    //     Err(client_without_inner)
    // elsif Timeout::KeepAliveAfter
    //     Ok((None, client))
    // else
    //     unreachable!()
    pub fn receive(self) -> Box<Future<Item = (Option<R::Item>, Self), Error = Self>> {
        Box::new(self.into_future()
            .then(|result| match result {
                Ok((Some(maybe_msg), client)) => future::ok((maybe_msg, client)),
                Ok((None, client)) |
                Err((_, client)) => future::err(client),
            }))
    }

    pub fn status(&self) -> Status<T::SinkError, R::Error> {
        if let Err(ref e) = self.inner {
            Status::Gone(e.clone())
        } else {
            Status::Ready
        }
    }

    // @TODO: Implement a method to check that neither stream has been dropped. This can be
    // done with just two poll calls, but sometimes poll requires to be on a task. I really
    // want to hide abstractions, so maybe returning a Future is best to make clear it might
    // require being on a task.
    //
    // It doesn't seem obviously possible to `executor::spawn().poll_future()`. There's an
    // `Unpark` argument to `poll_future` that we don't have a good value for.
    #[doc(hidden)]
    pub fn update_status(self) -> Box<Future<Item = (Option<R::Item>, Self), Error = Self>> {
        unimplemented!();
    }

    // N.B., Erases any existing error. This seems the least-odd thing to do.
    pub fn close(&mut self) {
        self.inner = Err(Error::Closed);
    }

    pub fn into_inner(self) -> (I, Result<(T, R), Error<T::SinkError, R::Error>>) {
        (self.id, self.inner.map(|inner| (inner.tx, inner.rx)))
    }
}

impl<I, T, R> Stream for Client<I, T, R>
    where I: Clone + Send + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type Item = Option<R::Item>;
    type Error = Error<T::SinkError, R::Error>;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let inner_poll = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Ok(Async::Ready(None)),
                Ok(inner) => inner,
            };

            // If there's a timeout specified, ensure we have a Sleep. Sleep will be instantiated
            // at the first poll after a value is yielded, indicating a new read is to take place.
            if inner.sleep.is_none() {
                inner.sleep = self.timeout.to_sleep();
            }

            inner.rx.poll()
        };

        match inner_poll {
            Ok(Async::NotReady) => {}
            // If an item is ready, discard the current Sleep and yield it.
            Ok(Async::Ready(Some(item))) => {
                if let Ok(inner) = self.inner.as_mut() {
                    inner.sleep = None;
                } else {
                    unreachable!();
                }
                return Ok(Async::Ready(Some(Some(item))));
            }
            // If the stream has terminated, discard it. We record the Closed state for the
            // benefit of Status calls, and pass on the stream termination.
            Ok(Async::Ready(None)) => {
                self.inner = Err(Error::Closed);
                return Ok(Async::Ready(None));
            }
            // If the stream yields an error we discard it. This is a limitation vs the
            // current stream error model but it simplifies an already complex model.
            Err(e) => {
                self.inner = Err(Error::Stream(e.clone()));
                return Err(Error::Stream(e));
            }
        }

        // A sleep should only be unavailable if the Timeout does not specify one.
        let sleep_poll = {
            match self.inner.as_mut() {
                Ok(v) => {
                    match v.sleep.as_mut() {
                        Some(sleep) => sleep.poll(),
                        None => return Ok(Async::NotReady),
                    }
                }
                Err(_) => unreachable!(),
            }
        };
        match sleep_poll {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            // When the sleep yields, the timeout is up.
            Ok(Async::Ready(_)) => {
                match self.timeout {
                    // Even if changed between polls, it's handled by `sleep.is_some` above.
                    Timeout::None => unreachable!(),
                    // If keeping alive, merely specify nothing arrived in time.
                    Timeout::KeepAliveAfter(..) => Ok(Async::Ready(Some(None))),
                    // If disconnecting, discard the stream. What to do here is complex.
                    // Yielding a `Err(Error::Timeout)` is more consistent with
                    // matters elsewhere, but the stream model doesn't consider that to
                    // indicate termination. Thus yield for stream termination.
                    Timeout::DisconnectAfter(..) => {
                        self.inner = Err(Error::Timeout);
                        Ok(Async::Ready(None))
                    }
                }
            }
            // If the timer errored we simply pass that through.
            Err(e) => Err(Error::Timer(e)),
        }
    }
}

impl<I, T, R> Sink for Client<I, T, R>
    where I: Clone + Send + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type SinkItem = T::SinkItem;
    type SinkError = Error<T::SinkError, R::Error>;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        let inner_start_send = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Err(Error::Closed),
                Ok(inner) => inner,
            };

            inner.tx.start_send(item)
        };

        match inner_start_send {
            Ok(AsyncSink::Ready) => Ok(AsyncSink::Ready),
            Ok(AsyncSink::NotReady(item)) => Ok(AsyncSink::NotReady(item)),
            Err(e) => {
                self.inner = Err(Error::Sink(e.clone()));
                Err(Error::Sink(e))
            }
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        let inner_poll_complete = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Err(Error::Closed),
                Ok(inner) => inner,
            };

            inner.tx.poll_complete()
        };

        match inner_poll_complete {
            Ok(Async::Ready(())) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                self.inner = Err(Error::Sink(e.clone()));
                Err(Error::Sink(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::{executor, Future, Stream};

    #[test]
    fn can_join_room() {
        let client0_id = "client0";
        let client1_id = "client1";

        let (_, _, client0) = mock_client(client0_id.clone(), 1);
        let (_, _, client0_duplicate_name) = mock_client(client0_id, 1);
        let (_, _, client1) = mock_client(client1_id.clone(), 1);

        // Adding of a `Client` to a `Room` returns `true`.
        let mut room = Room::default();
        assert_eq!(room.ids().len(), 0);
        assert!(client0.join(&mut room));
        assert_eq!(room.ids(),
                   vec![client0_id.to_string()].into_iter().collect());

        // Adding a `Client` whose ID was already present returns `false` and doesn't
        // add a duplicate.
        assert!(!client0_duplicate_name.join(&mut room));
        assert_eq!(room.ids(),
                   vec![client0_id.to_string()].into_iter().collect());

        // Adding a different-IDed `Client` to a `Room` works.
        assert!(client1.join(&mut room));
        // Extended comparison necessary because ordering not preserved.
        let client_ids = room.ids();
        assert!(client_ids.len() == 2);
        assert!(client_ids.contains(&client0_id.to_string()));
        assert!(client_ids.contains(&client1_id.to_string()));
    }

    #[test]
    fn can_transmit() {
        let (rx_from_client, _, mut client) = mock_client("client1", 1);
        let mut rx_stream = rx_from_client.wait().peekable();

        for _ in 0..10 {
            let msg = TinyMsg::A;
            client = client.transmit(msg.clone()).wait().unwrap();

            match rx_stream.next() {
                Some(Ok(msg2)) => {
                    assert_eq!(msg, msg2);
                }
                _ => assert!(false),
            }
        }
    }

    #[test]
    fn can_receive() {
        let (_, mut tx_to_client, mut client) = mock_client("client1", 1);
        //let mut rx_stream = rx.wait().peekable();

        for _ in 0..10 {
            let msg = TinyMsg::B("ABC".to_string());
            let receive = client.receive();

            let mut future = executor::spawn(receive.fuse());

            if let Ok(Async::NotReady) = future.poll_future(unpark_noop()) {
            } else {
                assert!(false);
            }

            tx_to_client = tx_to_client.send(msg.clone()).wait().unwrap();

            match future.wait_future() {
                Ok((maybe_msg, client_new)) => {
                    client = client_new;
                    assert_eq!("client1", client.id());
                    assert_eq!(msg, maybe_msg.unwrap());
                }
                _ => {
                    unreachable!();
                }
            };
        }
    }

    #[test]
    fn can_status() {
        let msg = TinyMsg::B("ABC".to_string());

        let (mut rx_from_client, _, client) = mock_client("client1", 1);
        assert!(client.status().ready());
        // Check unfortunate edgecase that a closed channel is not noticed until the next
        // IO action.
        let _ = rx_from_client.close();
        assert!(client.status().ready());

        let (_, mut tx_to_client, client) = mock_client("client2", 1);
        assert!(client.status().ready());
        let _ = tx_to_client.close();
        // Check unfortunate edgecase that a closed channel is not noticed until the next
        // IO action.
        assert!(client.status().ready());

        // Assert that status with dropped channels indicates the client is gone.
        let (_, _, client) = mock_client("client2", 1);
        assert!(client.status().ready());
        match client.transmit(msg.clone()).wait() {
            Ok(_) => unreachable!(),
            Err(client) => assert!(client.status().gone().is_some()),
        };
    }

    #[test]
    fn can_close() {
        let (_, _, mut client) = mock_client("client1", 1);
        assert!(client.status().ready());
        client.close();
        assert!(client.status().gone().is_some());
        // @TODO: Check channels are gone.
    }
}
