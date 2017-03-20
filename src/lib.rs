extern crate futures;
extern crate tokio_timer;

mod client;
//mod room;

pub use self::client::*;
//pub use self::room::*;

use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum ClientError<T, R> {
    Closed,
    Timeout,
    Timer(tokio_timer::TimerError),
    Sink(T),
    Stream(R),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientStatus<'a, T, R>
    where T: 'a,
          R: 'a
{
    Ready,
    Gone(&'a ClientError<T, R>),
}

#[derive(Clone, Copy)]
pub enum ClientTimeout<'a> {
    None,
    KeepAliveAfter(Duration, &'a tokio_timer::Timer),
    DisconnectAfter(Duration, &'a tokio_timer::Timer),
}

impl<'a> ClientTimeout<'a> {
    pub fn keep_alive_after(maybe_duration: Option<Duration>,
                            timer: &'a tokio_timer::Timer)
                            -> ClientTimeout {
        match maybe_duration {
            Some(duration) => ClientTimeout::KeepAliveAfter(duration, timer),
            None => ClientTimeout::None,
        }
    }

    pub fn disconnect_after(maybe_duration: Option<Duration>,
                            timer: &'a tokio_timer::Timer)
                            -> ClientTimeout {
        match maybe_duration {
            Some(duration) => ClientTimeout::DisconnectAfter(duration, timer),
            None => ClientTimeout::None,
        }
    }

    pub fn duration_timer(&self) -> Option<(Duration, &tokio_timer::Timer)> {
        match *self {
            ClientTimeout::None => None,
            ClientTimeout::KeepAliveAfter(d, t) => Some((d, t)),
            ClientTimeout::DisconnectAfter(d, t) => Some((d, t)),
        }
    }
}

// Utilities for testing `Communicator` implementations.
#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Arc;
    use futures::executor;
    use futures::sync::mpsc;

    #[derive(Clone, PartialEq, Debug)]
    pub enum TinyMsg {
        A,
        B(String),
    }

    pub fn unpark_noop() -> Arc<executor::Unpark> {
        struct Foo;

        impl executor::Unpark for Foo {
            fn unpark(&self) {}
        }

        Arc::new(Foo)
    }

    pub fn mock_client(id: &str,
                       buffer_size: usize)
                       -> (mpsc::Receiver<TinyMsg>,
                           mpsc::Sender<TinyMsg>,
                           Client<String, mpsc::Sender<TinyMsg>, mpsc::Receiver<TinyMsg>>) {
        let (tx, rx_from_client) = mpsc::channel(buffer_size);
        let (tx_to_client, rx) = mpsc::channel(buffer_size);
        (rx_from_client, tx_to_client, Client::new(id.to_string(), tx, rx))
    }
}
