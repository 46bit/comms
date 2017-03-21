extern crate futures;
extern crate tokio_timer;

mod client;
mod room;

pub use self::client::*;
pub use self::room::*;

use std::fmt::{self, Debug};
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

impl<'a, T, R> ClientStatus<'a, T, R>
    where T: 'a,
          R: 'a
{
    pub fn ready(&self) -> bool {
        if let ClientStatus::Ready = *self {
            true
        } else {
            false
        }
    }

    pub fn gone(&self) -> Option<&'a ClientError<T, R>> {
        if let ClientStatus::Gone(e) = *self {
            Some(e)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub enum ClientTimeout {
    None,
    KeepAliveAfter(Duration, tokio_timer::Timer),
    DisconnectAfter(Duration, tokio_timer::Timer),
}

impl Debug for ClientTimeout {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use ClientTimeout::*;
        match *self {
            None => write!(f, "ClientTimeout::None"),
            KeepAliveAfter(d, _) => {
                write!(f,
                       "ClientTimeout::KeepAliveAfter({:?}, tokio_timer::Timer)",
                       d)
            }
            DisconnectAfter(d, _) => {
                write!(f,
                       "ClientTimeout::DisconnectAfter({:?}, tokio_timer::Timer)",
                       d)
            }
        }
    }
}

impl ClientTimeout {
    pub fn keep_alive_after(maybe_duration: Option<Duration>,
                            timer: tokio_timer::Timer)
                            -> ClientTimeout {
        match maybe_duration {
            Some(duration) => ClientTimeout::KeepAliveAfter(duration, timer),
            None => ClientTimeout::None,
        }
    }

    pub fn disconnect_after(maybe_duration: Option<Duration>,
                            timer: tokio_timer::Timer)
                            -> ClientTimeout {
        match maybe_duration {
            Some(duration) => ClientTimeout::DisconnectAfter(duration, timer),
            None => ClientTimeout::None,
        }
    }

    pub fn to_sleep(&self) -> Option<tokio_timer::Sleep> {
        match *self {
            ClientTimeout::None => None,
            ClientTimeout::KeepAliveAfter(duration, ref timer) |
            ClientTimeout::DisconnectAfter(duration, ref timer) => Some(timer.sleep(duration)),
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
        (rx_from_client, tx_to_client, Client::new(id.to_string(), ClientTimeout::None, tx, rx))
    }
}
