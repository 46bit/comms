use std::fmt::Debug;
use std::marker::PhantomData;
use futures::{future, sink, stream, Future, Sink, Stream, Poll, Async, AsyncSink, StartSend};
use tokio_timer;
use super::*;

#[derive(Debug)]
pub struct Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    id: I,
    inner: Result<ClientInner<T, R>, ClientError<T::SinkError, R::Error>>,
}

#[derive(Debug)]
struct ClientInner<T, R>
    where T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    tx: T,
    rx: R,
}

impl<I, T, R> Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    pub fn new(id: I, tx: T, rx: R) -> Client<I, T, R> {
        Client {
            id: id,
            inner: Ok(ClientInner { tx: tx, rx: rx }),
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

    pub fn transmit(self, msg: T::SinkItem) -> Box<Future<Item = Self, Error = Self>> {
        // Lack of debug and partialeq requirement on error items requires this code.
        if let Ok(inner) = self.inner {
            let id = self.id;
            let id2 = id.clone();
            let rx = inner.rx;
            Box::new(inner.tx
                .send(msg)
                .map(|tx| Client::new(id, tx, rx))
                .map_err(|e| {
                    Client {
                        id: id2,
                        inner: Err(ClientError::Sink(e)),
                    }
                }))
        } else {
            Box::new(future::err(self))
        }
    }

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
    pub fn receive<'a>(self,
                       timeout: ClientTimeout<'static>)
                       -> Box<Future<Item = (Option<R::Item>, Self), Error = Self>> {
        // Lack of debug and partialeq requirement on error items requires this code.
        if let Ok(inner) = self.inner {
            let id = self.id;
            let id2 = id.clone();
            let tx = inner.tx;
            let sleep = timeout.duration_timer()
                .map(|(duration, timer)| timer.sleep(duration));
            Box::new(Receive::<'static, T, R> {
                    timeout: timeout,
                    sleep: sleep,
                    rx: Some(inner.rx),
                    ph: PhantomData,
                }
                .map(move |(maybe_msg, rx)| (maybe_msg, Self::new(id, tx, rx)))
                .map_err(move |e| {
                    Client {
                        id: id2,
                        inner: Err(e),
                    }
                }))
        } else {
            Box::new(future::err(self))
        }
    }

    pub fn status<'a>(&'a self) -> ClientStatus<'a, T::SinkError, R::Error> {
        if let Err(ref e) = self.inner {
            ClientStatus::Gone(&e)
        } else {
            ClientStatus::Ready
        }
    }

    pub fn close(&mut self) {
        // N.B., Erases any existing error. This seems the least-odd thing to do.
        self.inner = Err(ClientError::Closed);
    }

    pub fn into_inner(self) -> (I, Result<(T, R), ClientError<T::SinkError, R::Error>>) {
        (self.id, self.inner.map(|inner| (inner.tx, inner.rx)))
    }
}

struct Receive<'a, T, R>
    where T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    timeout: ClientTimeout<'a>,
    sleep: Option<tokio_timer::Sleep>,
    rx: Option<R>,
    ph: PhantomData<T>,
}

impl<'a, T, R> Future for Receive<'a, T, R>
    where T: Sink + Debug + 'static,
          R: Stream + Debug + 'static
{
    type Item = (Option<R::Item>, R);
    type Error = ClientError<T::SinkError, R::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // First, try polling the future
        let polled = self.rx.as_mut().unwrap().poll();
        match polled {
            Ok(Async::NotReady) => {}
            Ok(Async::Ready(maybe_item)) => {
                return Ok(Async::Ready((maybe_item, self.rx.take().unwrap())));
            }
            Err(e) => return Err(ClientError::Stream(e)),
        }

        // Now check the timer
        if let Some(sleep) = self.sleep.as_mut() {
            match sleep.poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(_)) => {
                    match self.timeout {
                        ClientTimeout::DisconnectAfter(..) => Err(ClientError::Timeout),
                        _ => Ok(Async::Ready((None, self.rx.take().unwrap()))),
                    }
                }
                Err(e) => Err(ClientError::Timer(e)),
            }
        } else {
            Ok(Async::NotReady)
        }
    }
}

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
    fn can_status() {
        let msg = TinyMsg::B("ABC".to_string());

        let (mut rx_from_client, _, client) = mock_client("client1", 1);
        assert!(client.status().ready());
        let _ = rx_from_client.close();
        assert!(client.status().ready());

        let (_, mut tx_to_client, client) = mock_client("client2", 1);
        assert!(client.status().ready());
        let _ = tx_to_client.close();
        assert!(client.status().ready());

        let (_, _, client) = mock_client("client2", 1);
        assert!(client.status().ready());
        assert!(client.transmit(msg.clone()).wait().is_err());
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
