use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use futures::{future, Future, Sink, Stream, Poll, Async};
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

    pub fn join(self, room: &mut Room<I, T, R>) -> bool
        where I: PartialEq + Eq + Hash
    {
        room.insert(self)
    }

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
    pub fn receive(self,
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

    pub fn status(&self) -> ClientStatus<T::SinkError, R::Error> {
        if let Err(ref e) = self.inner {
            ClientStatus::Gone(e)
        } else {
            ClientStatus::Ready
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
            Ok(Async::Ready(Some(item))) => {
                return Ok(Async::Ready((Some(item), self.rx.take().unwrap())));
            }
            Ok(Async::Ready(None)) => {
                self.rx.take();
                return Ok(Err(ClientError::Closed));
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
            let receive = client.receive(ClientTimeout::None);

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
