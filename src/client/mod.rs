mod receive;
pub use self::receive::*;

use std::hash::Hash;
use std::fmt::{self, Debug};
use futures::{Future, Sink, Stream, Poll, Async, AsyncSink, StartSend};
use futures::sync::mpsc;
use futures::stream::{FromErr, SplitStream, SplitSink};
use futures::sink::SinkFromErr;
use tokio_timer;
use super::*;

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
pub struct Client<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    id: I,
    inner: Result<C, Disconnect<C::SinkError, C::Error>>,
}

impl<I, T, R> Client<I, Unsplit<T, R>>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    /// Create a new client from separate `Sink` and `Stream`.
    ///
    /// Created from the ID, a `Sink` and a `Stream`.
    pub fn new_from_split(id: I, tx: T, rx: R) -> Client<I, Unsplit<T, R>> {
        Client::new(id, Unsplit { tx: tx, rx: rx })
    }
}

// @TODO: Implement `Sink` on `futures::stream::FromErr` to simplify these types.
impl<I, C> Client<I,
                  Unsplit<SinkFromErr<SplitSink<C>, ErrorString>,
                          FromErr<SplitStream<C>, ErrorString>>>
    where I: Clone + Send + Debug + 'static,
          C: Sink<SinkError = io::Error> + Stream<Error = io::Error> + 'static
{
    /// Create a new client from a `Sink + Stream` that uses `io::Error`.
    ///
    /// ```rust,ignore
    /// listener.incoming()
    ///     .for_each(move |(socket, addr)| {
    ///         let client = Client::new_from_io(Uuid::new_v4(), ClientTimeout::None, socket.framed(MsgCodec));
    /// ```
    pub fn new_from_io(id: I,
                       tx_rx: C)
                       -> Client<I,
                                 Unsplit<SinkFromErr<SplitSink<C>, ErrorString>,
                                         FromErr<SplitStream<C>, ErrorString>>> {
        let (tx, rx) = tx_rx.split();
        Client::new_from_split(id, tx.sink_from_err(), rx.from_err())
    }
}

/// A client using mpsc channels.
pub type MpscClient<I, M> = Client<I, Unsplit<mpsc::Sender<M>, mpsc::Receiver<M>>>;
// @TODO: Adopting tokio-io feature flag could allow these handy types. Unsure if worthwhile.
//pub type TokioClient<S, C> = Client<I, Framed<S, C>>;
//pub type TokioTcpClient<C> = Client<I, Framed<TcpStream, C>>;

impl<I, C> Client<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    /// Create a new client from a `Sink + Stream`.
    ///
    /// Created from the ID, a timeout strategy, and a `Sink + Stream`.
    pub fn new(id: I, tx_rx: C) -> Client<I, C> {
        Client {
            id: id,
            inner: Ok(tx_rx),
        }
    }

    /// Get a clone of the client ID.
    pub fn id(&self) -> I {
        self.id.clone()
    }

    /// Change the client's ID. The new ID can be of a different type.
    pub fn rename<J>(self, new_id: J) -> Client<J, C>
        where J: Clone + Send + Debug + 'static
    {
        Client {
            id: new_id,
            inner: self.inner,
        }
    }

    /// Join a `Room` of clients with the same type.
    pub fn join(self, room: &mut Room<I, C>) -> bool
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
    pub fn transmit(self, msg: C::SinkItem) -> Box<Future<Item = Self, Error = Self>> {
        // N.B. This does ensure the inner is set, but it's a bit of a hack.
        let id = self.id();
        Box::new(self.send(msg).map_err(|e| {
            Client {
                id: id,
                inner: Err(e.clone()),
            }
        }))
    }

    /// Future that tries to receive a single message.
    pub fn receive(self) -> Receive<I, C> {
        Receive::new(self)
    }

    /// Get the current status of this Client.
    ///
    /// Check whether a Client is connected with `client.status().is_connected()`.
    ///
    /// If disconnected you can get the cause of disconnection with
    /// `client.status().is_disconnected().unwrap()`.
    pub fn status(&self) -> Status<C::SinkError, C::Error> {
        if let Err(ref e) = self.inner {
            Status::Disconnected(e.clone())
        } else {
            Status::Connected
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
    pub fn update_status(self) -> Box<Future<Item = (Option<C::Item>, Self), Error = Self>> {
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
    pub fn into_inner(self) -> (I, Result<C, Disconnect<C::SinkError, C::Error>>) {
        (self.id, self.inner)
    }
}

impl<I, C> Stream for Client<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = C::Item;
    type Error = Disconnect<C::SinkError, C::Error>;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let inner_poll = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Ok(Async::Ready(None)),
                Ok(inner) => inner,
            };

            inner.poll()
        };

        match inner_poll {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            // If an item is ready, discard the current Sleep and yield it.
            Ok(Async::Ready(Some(item))) => Ok(Async::Ready(Some(item))),
            // If the stream has terminated, discard it. We record the Dropped state for the
            // benefit of Status calls, and pass on the stream termination.
            Ok(Async::Ready(None)) => {
                self.inner = Err(Disconnect::Dropped);
                Ok(Async::Ready(None))
            }
            // If the stream yields an error we discard it. This is a limitation vs the
            // current stream error model but it simplifies an already complex model.
            Err(e) => {
                self.inner = Err(Disconnect::Stream(e.clone()));
                Err(Disconnect::Stream(e))
            }
        }
    }
}

impl<I, C> Sink for Client<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type SinkItem = C::SinkItem;
    type SinkError = Disconnect<C::SinkError, C::Error>;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        let inner_start_send = {
            let inner = match self.inner.as_mut() {
                Err(_) => return Err(Disconnect::Closed),
                Ok(inner) => inner,
            };

            inner.start_send(item)
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

            inner.poll_complete()
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

// @TODO: When breaking `Client` into separate futures, try defining:
// type Connection = Result<Connection, Disconnection>;
// N.B., Would need a better name for the type.
pub struct Unsplit<T, R>
    where T: Sink + 'static,
          R: Stream + 'static
{
    tx: T,
    rx: R,
}

impl<T, R> Debug for Unsplit<T, R>
    where T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "Unsplit {{ tx: Sink, rx: Stream, sleep: tokio_timer::Sleep }}")
    }
}

impl<T, R> Sink for Unsplit<T, R>
    where T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type SinkItem = T::SinkItem;
    type SinkError = T::SinkError;

    fn start_send(&mut self, item: T::SinkItem) -> StartSend<T::SinkItem, T::SinkError> {
        self.tx.start_send(item)
    }

    fn poll_complete(&mut self) -> Poll<(), T::SinkError> {
        self.tx.poll_complete()
    }
}

impl<T, R> Stream for Unsplit<T, R>
    where T: Sink + 'static,
          R: Stream + 'static,
          T::SinkError: Clone,
          R::Error: Clone
{
    type Item = R::Item;
    type Error = R::Error;

    fn poll(&mut self) -> Poll<Option<R::Item>, R::Error> {
        self.rx.poll()
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
                Ok((msg, client_new)) => {
                    client = client_new;
                    assert_eq!("client1", client.id());
                    assert_eq!(msg, msg);
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
                assert_eq!(c0.poll(), Ok(Async::Ready(Some(msg))));
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
                assert_eq!(c0.start_send(TinyMsg::A),
                           Ok(AsyncSink::NotReady(TinyMsg::A)));
                assert_eq!(c0.poll(), Ok(Async::NotReady));
                assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));
                // Again.
                assert_eq!(c0.start_send(TinyMsg::A),
                           Ok(AsyncSink::NotReady(TinyMsg::A)));
                assert_eq!(c0.poll(), Ok(Async::NotReady));
                assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

                // With a message cleared, we can send another.
                rx.next();
                assert_eq!(c0.start_send(TinyMsg::A), Ok(AsyncSink::Ready));
                assert_eq!(c0.poll(), Ok(Async::NotReady));
                assert_eq!(c0.poll_complete(), Ok(Async::Ready(())));

                Ok::<(), ()>(())
            })
            .wait()
            .unwrap();

        //rx.close();
        //tx.close().unwrap();
    }

    #[test]
    fn can_status() {
        let msg = TinyMsg::B("ABC".to_string());

        let (mut rx_from_client, _, client) = mock_client("client1", 1);
        assert!(client.status().is_connected());
        // Check unfortunate edgecase that a closed channel is not noticed until the next
        // IO action.
        let _ = rx_from_client.close();
        assert!(client.status().is_connected());

        let (_, mut tx_to_client, client) = mock_client("client2", 1);
        assert!(client.status().is_connected());
        let _ = tx_to_client.close();
        // Check unfortunate edgecase that a closed channel is not noticed until the next
        // IO action.
        assert!(client.status().is_connected());

        // Assert that status with dropped channels indicates the client is gone.
        let (_, _, client) = mock_client("client2", 1);
        assert!(client.status().is_connected());
        match client.transmit(msg.clone()).wait() {
            Ok(_) => unreachable!(),
            Err(client) => assert!(client.status().is_disconnected().is_some()),
        };
    }

    #[test]
    fn can_close() {
        let (_, _, mut client) = mock_client("client1", 1);
        assert!(client.status().is_connected());
        client.close();
        assert!(client.status().is_disconnected().is_some());
        // @TODO: Check channels are gone.
    }
}
