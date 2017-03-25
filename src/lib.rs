extern crate futures;
extern crate tokio_timer;
#[cfg(test)]
extern crate quickcheck;

/// A single client connection.
pub mod client;
/// Rooms (groups) of client connections.
pub mod room;

pub use self::client::Client;
pub use self::room::Room;

// Utilities for testing `Communicator` implementations.
#[cfg(test)]
mod test {
    use super::*;
    use client::MpscClient;
    use std::sync::Arc;
    use futures::{executor, Stream, Async, Poll};
    use futures::sync::mpsc;
    use quickcheck::{Gen, Arbitrary};

    #[derive(Clone, PartialEq, Eq, Debug)]
    pub enum TinyMsg {
        A,
        B(String),
    }

    impl Arbitrary for TinyMsg {
        fn arbitrary<G: Gen>(g: &mut G) -> TinyMsg {
            if g.gen() {
                TinyMsg::A
            } else {
                TinyMsg::B(String::arbitrary(g))
            }
        }
    }

    #[derive(Copy, Clone, PartialEq, Eq, Debug)]
    pub enum CopyMsg {
        A,
        B(usize),
    }

    impl Arbitrary for CopyMsg {
        fn arbitrary<G: Gen>(g: &mut G) -> CopyMsg {
            if g.gen() {
                CopyMsg::A
            } else {
                CopyMsg::B(usize::arbitrary(g))
            }
        }
    }

    pub fn unpark_noop() -> Arc<executor::Unpark> {
        struct Foo;

        impl executor::Unpark for Foo {
            fn unpark(&self) {}
        }

        Arc::new(Foo)
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct NeverReady;

    impl Stream for NeverReady {
        type Item = NeverReady;
        type Error = NeverReady;

        fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
            Ok(Async::NotReady)
        }
    }

    pub fn mock_client
        (id: &str,
         buffer_size: usize)
         -> (mpsc::Receiver<TinyMsg>, mpsc::Sender<TinyMsg>, MpscClient<String, TinyMsg>) {
        let (tx, rx_from_client) = mpsc::channel(buffer_size);
        let (tx_to_client, rx) = mpsc::channel(buffer_size);
        let client = Client::new_from_split(id.to_string(), tx, rx);
        (rx_from_client, tx_to_client, client)
    }

    pub fn mock_client_copy
        (id: &str,
         buffer_size: usize)
         -> (mpsc::Receiver<CopyMsg>, mpsc::Sender<CopyMsg>, MpscClient<String, CopyMsg>) {
        let (tx, rx_from_client) = mpsc::channel(buffer_size);
        let (tx_to_client, rx) = mpsc::channel(buffer_size);
        let client = Client::new_from_split(id.to_string(), tx, rx);
        (rx_from_client, tx_to_client, client)
    }
}
