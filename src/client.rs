use std::fmt::Debug;
use futures::{sink, stream, Future, Sink, Stream, Poll, Async, AsyncSink, StartSend};
use tokio_timer;
use super::*;

enum TimeoutError<S>
    where S: Stream
{
    TimeoutError(tokio_timer::TimeoutError<S>),
    StreamError(S::Error),
}

impl<S> From<tokio_timer::TimeoutError<S>> for TimeoutError<S>
    where S: Stream
{
    fn from(e: tokio_timer::TimeoutError<S>) -> TimeoutError<S> {
        TimeoutError::TimeoutError(e)
    }
}

pub struct Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    id: I,
    inner: Result<ClientInner<T, R>, ClientError<T, R>>,
}

struct ClientInner<T, R>
    where T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    tx: T,
    rx: R,
    timer: tokio_timer::Timer,
}

impl<I, T, R> Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    pub fn new(id: I, tx: T, rx: R, timer: tokio_timer::Timer) -> Client<I, T, R> {
        Client {
            id: id,
            inner: Ok(ClientInner {
                tx: tx,
                rx: rx,
                timer: timer,
            }),
        }
    }

    pub fn id(&self) -> I {
        self.id.clone()
    }

    pub fn rename<J>(self, new_id: J) -> Client<J, T, R>
        where J: Clone + Send + Debug + 'static
    {
        Client {
            id: new_id,
            inner: self.inner,
        }
    }

    // pub fn join(self, room: &mut Room<I, T, R>) -> bool {
    //     room.insert(self)
    // }

    // pub fn transmit(self, msg: T::SinkItem) -> sink::Send<Self> {
    //     self.send(msg).then(|result| match result {
    //         Ok(client) => future::ok(Ok(client)),
    //         Err(e) => future::ok()
    //     })
    // }

    // We\re mapping everything to Ok.
    // But if no message is provided, ClientTimeout::DisconnectAfter will be Ok but closed.
    // Deliberate aim: no more abstraction; if anything the crate will remove abstraction.

    // Or we map to Ok if the client is still connected. Yes, this.

    // if msg received
    //     Ok((Some(msg), client))
    // elsif ClientTimeout::None
    //     does not terminate
    // elsif ClientTimeout::DisconnectAfter
    //     Err(client_without_inner)
    // elsif ClientTimeout::KeepAliveAfter
    //     Ok((None, client))
    // else
    //     unreachable!()



    pub fn receive
        (self,
         timeout: ClientTimeout)
         -> Box<Future<Item = (Option<R::Item>, Client<I, T, R>), Error = Client<I, T, R>>> {
        let mut rx = Box::new(self.inner.as_mut().unwrap().rx.map_err(TimeoutError::StreamError));
        if let Some(timeout_duration) = timeout.duration() {
            rx = Box::new(self.inner
                .as_mut().unwrap()
                .timer
                .timeout_stream(rx, timeout_duration)
                .map(|(maybe_msg, rx_or_timeout)| (maybe_msg, rx_or_timeout.into_inner())));
        }

        rx.then(|result| match result {
            Ok((maybe_msg, rx)) => {
                self.inner.as_mut().unwrap().rx = rx;
                Ok((maybe_msg, self))
            }
            // If the stream errors, we discard the client.
            // For relative simplicity, if the timer errors, we discard the client.
            // In time it would be best to reduce clients to just IDs if they fail.
            Err(e) => {
                match e {
                    TimeoutError::TimeoutError(tokio_timer::TimeoutError::Timer(_, e)) => {
                        self.inner = Err(ClientError::Timer(e));
                        Err(self)
                    }
                    TimeoutError::TimeoutError(TimeoutError::TimedOut(rx)) => {
                        if timeout == ClientTimeout::DisconnectAfter(..) {
                            self.inner = Err(ClientError::Timeout);
                            Err(self)
                        } else {
                            self.inner.as_mut().unwrap.rx = rx;
                            Ok((None, self))
                        }
                    }
                    TimeoutError::StreamError(e) => {
                        self.inner = Err(ClientError::StreamError(e));
                        Err(self)
                    }
                }
            }
        })
    }

    // -> Box<Future<Item = Client, Error = Client>>
    // pub enum ClientTimeout {
    //     None,                       -> Error=
    //     KeepAliveAfter(Duration),   -> Error=ClientWithInner
    //     DisconnectAfter(Duration),  -> Error=ClientWithoutInner
    // }

    // pub fn status_sync(&self) -> ClientStatus {
    //     if self.inner.is_some() {
    //         ClientStatus::Ready
    //     } else {
    //         ClientStatus::Gone
    //     }
    // }

    // // @TODO: Implement `into_inner` yielding a `(T, R)` tuple?
    // pub fn close(&mut self) {
    //     // Drop inner tx and rx, if present.
    //     self.inner.take();
    // }
}

// impl<I, T, R> Sink for Client<I, T, R>
//     where I: Clone + Send + Debug + 'static,
//           T: Sink + 'static,
//           R: Stream + 'static
// {
//     type SinkItem = T::SinkItem;
//     type SinkError = T::SinkError;

//     fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
//         match self.inner.as_mut().map(|inner| &mut inner.tx) {
//             Some(mut tx) => tx.start_send(item),
//             None => Ok(AsyncSink::NotReady(item)),
//         }
//     }

//     fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
//         match self.inner.as_mut().map(|inner| &mut inner.tx) {
//             Some(mut tx) => tx.poll_complete(),
//             None => Ok(Async::Ready(())),
//         }
//     }
// }

// impl<I, T, R> Stream for Client<I, T, R>
//     where I: Clone + Send + Debug + 'static,
//           T: Sink + 'static,
//           R: Stream + 'static
// {
//     type Item = R::Item;
//     type Error = R::Error;

//     fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
//         match self.inner.as_mut().map(|inner| &mut inner.rx) {
//             Some(mut rx) => rx.poll(),
//             None => Ok(Async::Ready(None)),
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::{Future, Stream, executor};

    // #[test]
    // fn can_join_room() {
    //     let (_, client0) = mock_client_channelled();
    //     let (_, client1) = mock_client_channelled();

    //     // Adding of a `Client` to a `Room` returns `true`.
    //     let mut room = Room::default();
    //     assert_eq!(room.client_ids().len(), 0);
    //     assert!(client0.clone().join(&mut room));
    //     assert_eq!(room.client_ids(), vec![client0.id]);

    //     // Adding a `Client` whose ID was already present returns `false` and doesn't
    //     // add a duplicate.
    //     let client0_id = client0.id;
    //     assert!(!client0.join(&mut room));
    //     assert_eq!(room.client_ids(), vec![client0_id]);

    //     // Adding a different-IDed `Client` to a `Room` works.
    //     let client1_id = client1.id;
    //     assert!(client1.join(&mut room));
    //     // Extended comparison necessary because ordering not preserved.
    //     let client_ids = room.client_ids();
    //     assert!(client_ids.len() == 2);
    //     assert!(client_ids.contains(&client0_id));
    //     assert!(client_ids.contains(&client1_id));
    // }

    #[test]
    fn can_transmit() {
        let (rx_from_client, _, mut client) = mock_client("client1", 1);
        let mut rx_stream = rx_from_client.wait().peekable();

        for _ in 0..10 {
            let msg = TinyMsg::A;
            client.transmit(msg.clone()).wait().unwrap();

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
            let receive = client.receive(ClientTimeout::None);

            let mut future = executor::spawn(receive.fuse());

            //assert!(future.poll_future(unpark_noop()) == Ok(AsyncSink::NotReady));

            tx_to_client = tx_to_client.send(msg.clone()).wait().unwrap();

            match future.wait_future() {
                Ok((maybe_msg, client)) => {
                    assert_eq!("client1", client.id());
                    assert_eq!(msg, maybe_msg.unwrap());
                }
                _ => assert!(false),
            }
        }
    }

    #[test]
    fn can_status_sync() {
        let msg = TinyMsg::B("ABC".to_string());

        let (mut rx_from_client, _, client) = mock_client("client1", 1);
        assert_eq!(client.status_sync(), ClientStatus::Ready);
        let _ = rx_from_client.close();
        assert_eq!(client.status_sync(), ClientStatus::Ready);

        let (_, mut tx_to_client, client) = mock_client("client2", 1);
        assert_eq!(client.status_sync(), ClientStatus::Ready);
        let _ = tx_to_client.close();
        assert_eq!(client.status_sync(), ClientStatus::Ready);

        let (_, _, client) = mock_client("client2", 1);
        assert_eq!(client.status_sync(), ClientStatus::Ready);
        assert!(client.send(msg.clone()).wait().is_err());
        //assert_eq!(client.status_sync(), ClientStatus::Gone);
    }

    // #[test]
    // fn can_close() {
    //     let (tx, rx) = mpsc::channel(1);
    //     let mut rx_stream = rx.wait().peekable();
    //     let mut client = mock_client(tx);

    //     for _ in 0..10 {
    //         let msg = TinyMsg::B("test".to_string());
    //         client.transmit(msg.clone()).wait().unwrap();
    //         match rx_stream.next() {
    //             Some(Ok(Command::Transmit(client_id, msg2))) => {
    //                 assert_eq!(client_id, client.id());
    //                 assert_eq!(msg, msg2);
    //             }
    //             _ => assert!(false),
    //         }
    //     }
    // }
}
