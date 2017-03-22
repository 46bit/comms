use std::hash::Hash;
use std::fmt::{self, Debug};
use futures::{future, Future, Sink, Stream, Poll, Async, AsyncSink, StartSend};
use tokio_timer;
use super::*;

// @TODO: When breaking `Client` into separate futures, try defining:
// type Connection = Result<Connection, Disconnection>;
// N.B., Would need a better name for the type.
struct ClientInner<T, R>
    where T: Sink + 'static,
          R: Stream + 'static
{
    tx: T,
    rx: R,
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

/// Handles communication with a single server client.
///
/// This is the basic 'unit' around which `comms` is constructed. It handles
/// communication with a single client, e.g., a single TCP socket. It stores
/// an ID and closes the connection if the provided `Sink` or `Stream` errors.
///
/// In addition this supports timeouts on receiving data. The `Timeout` enum
/// allows specifying whether to have a timeout and whether the client should
/// be disconnected should the timeout not be met.
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
    inner: Result<ClientInner<T, R>, Disconnect<T::SinkError, R::Error>>,
}

impl<I, T, R> Client<I, T, R>
    where I: Clone + Send + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    /// Create a new client.
    ///
    /// Created from the ID, a timeout strategy, a `Sink` and a `Stream`.
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

    /// Get a clone of the client ID.
    pub fn id(&self) -> I {
        self.id.clone()
    }

    /// Change the client's ID. The new ID can be of a different type.
    pub fn rename<J>(self, new_id: J) -> Client<J, T, R>
        where J: Clone + Send + 'static
    {
        Client {
            id: new_id,
            timeout: self.timeout,
            inner: self.inner,
        }
    }

    /// Get the current timeout strategy in use.
    pub fn timeout(&self) -> &Timeout {
        &self.timeout
    }

    /// Change the timeout strategy in use.
    pub fn set_timeout(&mut self, timeout: Timeout) {
        self.timeout = timeout;
    }

    /// Join a `Room` of clients with the same type.
    pub fn join(self, room: &mut Room<I, T, R>) -> bool
        where I: PartialEq + Eq + Hash
    {
        room.insert(self)
    }

    /// Future that transmits a single message.
    ///
    /// If this succeeds the Item is a Client. If the transmission fails the Error is a
    /// Client which has dropped its connection.
    ///
    /// ```rust,ignore
    /// # This is extremely inefficient
    /// handle.spawn(client.transmit('h')
    ///     .and_then(|client| client.transmit('e'))
    ///     .and_then(|client| client.transmit('l'))
    ///     .and_then(|client| client.transmit('l'))
    ///     .and_then(|client| client.transmit('o'))
    ///     .map(|client| println!("sent hello to {}.", client.id()); ())
    ///     .map_err(|client| println!("sending hello to {} failed.", client.id()); ()));
    /// ```
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

    /// Future that tries to receive a single message.
    ///
    /// If the Client is disconnected (whether because the connection dropped or because
    /// of the current timeout strategy) the Error value is a Client.
    ///
    /// If the Client is not disconnected, the Item value is a tuple `(Option<msg>, Client)`.
    /// The message option will only be `None` if the timeout was reached but its strategy
    /// specified keeping the Client connected.
    pub fn receive(self) -> Box<Future<Item = (Option<R::Item>, Self), Error = Self>> {
        Box::new(self.into_future()
            .then(|result| match result {
                Ok((Some(maybe_msg), client)) => future::ok((maybe_msg, client)),
                Ok((None, client)) |
                Err((_, client)) => future::err(client),
            }))
    }

    /// Get the current status of this Client.
    ///
    /// Check whether a Client is connected with `client.status().is_ready()`.
    ///
    /// If disconnected you can get the cause of disconnection with
    /// `client.status().is_gone().unwrap()`.
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

    /// Force the Client to disconnect if a connection is active.
    ///
    /// Returns whether an active connection was disconnected.
    pub fn close(&mut self) -> bool {
        if self.inner.is_ok() {
            self.inner = Err(Disconnect::Closed);
            return true;
        }
        false
    }

    /// Retrieve the client ID and the current state of the connection.
    pub fn into_inner(self) -> (I, Result<(T, R), Disconnect<T::SinkError, R::Error>>) {
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
    type Error = Disconnect<T::SinkError, R::Error>;

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
            // If the stream has terminated, discard it. We record the Dropped state for the
            // benefit of Status calls, and pass on the stream termination.
            Ok(Async::Ready(None)) => {
                self.inner = Err(Disconnect::Dropped);
                return Ok(Async::Ready(None));
            }
            // If the stream yields an error we discard it. This is a limitation vs the
            // current stream error model but it simplifies an already complex model.
            Err(e) => {
                self.inner = Err(Disconnect::Stream(e.clone()));
                return Err(Disconnect::Stream(e));
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
                    // Yielding a `Err(Disconnect::Timeout)` is more consistent with
                    // matters elsewhere, but the stream model doesn't consider that to
                    // indicate termination. Thus yield for stream termination.
                    Timeout::DisconnectAfter(..) => {
                        self.inner = Err(Disconnect::Timeout);
                        Ok(Async::Ready(None))
                    }
                }
            }
            // If the timer errored we simply pass that through.
            Err(e) => Err(Disconnect::Timer(e)),
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
    type SinkError = Disconnect<T::SinkError, R::Error>;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        let inner_start_send = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Err(Disconnect::Closed),
                Ok(inner) => inner,
            };

            inner.tx.start_send(item)
        };

        match inner_start_send {
            Ok(AsyncSink::Ready) => Ok(AsyncSink::Ready),
            Ok(AsyncSink::NotReady(item)) => Ok(AsyncSink::NotReady(item)),
            Err(e) => {
                self.inner = Err(Disconnect::Sink(e.clone()));
                Err(Disconnect::Sink(e))
            }
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        let inner_poll_complete = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Err(Disconnect::Closed),
                Ok(inner) => inner,
            };

            inner.tx.poll_complete()
        };

        match inner_poll_complete {
            Ok(Async::Ready(())) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                self.inner = Err(Disconnect::Sink(e.clone()));
                Err(Disconnect::Sink(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::{lazy, executor, Future, Stream};

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
    fn stream_and_sink_separate() {
        let (rx, tx, mut c0) = mock_client("client1", 1);
        let mut rx = rx.wait();

        lazy(move || {
            // With nothing to be sent and nothing to be received, the Stream should not be
            // ready and the Sink should be empty.
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            // With a message to be received, the Stream should be ready and the Sink should
            // be empty.
            let msg = TinyMsg::A;
            tx.clone().send(msg.clone()).wait().unwrap();
            assert_eq!(c0.poll(), Ok(Async::Ready(Some(Some(msg)))));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            // With no message left to be received, the Stream should not be ready and the Sink
            // should be empty.
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            // Start to send a message should not stall poll_complete and not affect Stream.
            assert_eq!(c0.start_send(TinyMsg::A), Ok(AsyncSink::Ready));
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            // Start to send a message should not stall poll_complete and not affect Stream.
            assert_eq!(c0.start_send(TinyMsg::A), Ok(AsyncSink::Ready));
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            // Backpressure causes no progress until some items read out.
            assert_eq!(c0.start_send(TinyMsg::A), Ok(AsyncSink::NotReady(TinyMsg::A)));
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));
            // Again.
            assert_eq!(c0.start_send(TinyMsg::A), Ok(AsyncSink::NotReady(TinyMsg::A)));
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            // With a message cleared, we can send another.
            rx.next();
            assert_eq!(c0.start_send(TinyMsg::A), Ok(AsyncSink::Ready));
            assert_eq!(c0.poll(), Ok(Async::NotReady));
            assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

            Ok::<(), ()>(())
        }).wait().unwrap();

        //rx.close();
        //tx.close().unwrap();
    }

    #[test]
    fn can_status() {
        let msg = TinyMsg::B("ABC".to_string());

        let (mut rx_from_client, _, client) = mock_client("client1", 1);
        assert!(client.status().is_ready());
        // Check unfortunate edgecase that a closed channel is not noticed until the next
        // IO action.
        let _ = rx_from_client.close();
        assert!(client.status().is_ready());

        let (_, mut tx_to_client, client) = mock_client("client2", 1);
        assert!(client.status().is_ready());
        let _ = tx_to_client.close();
        // Check unfortunate edgecase that a closed channel is not noticed until the next
        // IO action.
        assert!(client.status().is_ready());

        // Assert that status with dropped channels indicates the client is gone.
        let (_, _, client) = mock_client("client2", 1);
        assert!(client.status().is_ready());
        match client.transmit(msg.clone()).wait() {
            Ok(_) => unreachable!(),
            Err(client) => assert!(client.status().is_gone().is_some()),
        };
    }

    #[test]
    fn can_close() {
        let (_, _, mut client) = mock_client("client1", 1);
        assert!(client.status().is_ready());
        client.close();
        assert!(client.status().is_gone().is_some());
        // @TODO: Check channels are gone.
    }
}
