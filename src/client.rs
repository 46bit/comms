use std::fmt::Debug;
use futures::{sink, stream, Sink, Stream, Poll, Async, AsyncSink, StartSend};
use super::*;

pub struct Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    id: I,
    inner: Option<ClientInner<T, R>>,
}

struct ClientInner<T, R>
    where T: Sink + 'static,
          R: Stream + 'static
{
    tx: T,
    rx: R,
}

impl<I, T, R> Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    pub fn new(id: I, tx: T, rx: R) -> Client<I, T, R> {
        Client {
            id: id,
            inner: Some(ClientInner { tx: tx, rx: rx }),
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

    pub fn join(self, room: &mut Room<I, T, R>) -> bool {
        room.insert(self)
    }

    pub fn transmit(&mut self, msg: T::SinkItem) -> sink::Send<&mut Self> {
        self.send(msg)
    }

    pub fn receive(&mut self, timeout: ClientTimeout) -> stream::StreamFuture<&mut Self> {
        self.into_future()
    }

    pub fn status_sync(&self) -> ClientStatus {
        if self.inner.is_some() {
            ClientStatus::Ready
        } else {
            ClientStatus::Gone
        }
    }

    // @TODO: Implement `into_inner` yielding a `(T, R)` tuple?
    pub fn close(&mut self) {
        // Drop inner tx and rx, if present.
        self.inner.take();
    }
}

impl<I, T, R> Sink for Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    type SinkItem = T::SinkItem;
    type SinkError = T::SinkError;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        match self.inner.as_mut().map(|inner| &mut inner.tx) {
            Some(mut tx) => tx.start_send(item),
            None => Ok(AsyncSink::NotReady(item)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.as_mut().map(|inner| &mut inner.tx) {
            Some(mut tx) => tx.poll_complete(),
            None => Ok(Async::Ready(())),
        }
    }
}

impl<I, T, R> Stream for Client<I, T, R>
    where I: Clone + Send + Debug + 'static,
          T: Sink + 'static,
          R: Stream + 'static
{
    type Item = R::Item;
    type Error = R::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.as_mut().map(|inner| &mut inner.rx) {
            Some(mut rx) => rx.poll(),
            None => Ok(Async::Ready(None)),
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
