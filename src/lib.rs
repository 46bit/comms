extern crate futures;
extern crate tokio_timer;

pub mod client;
pub mod room;

pub use self::client::Client;
pub use self::room::Room;

use std::fmt::{self, Debug};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Error<T, R> {
    Closed,
    Timeout,
    Timer(tokio_timer::TimerError),
    Sink(T),
    Stream(R),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status<T, R> {
    Ready,
    Gone(Error<T, R>),
}

impl<T, R> Status<T, R> {
    pub fn ready(&self) -> bool {
        if let Status::Ready = *self {
            true
        } else {
            false
        }
    }

    pub fn gone(&self) -> Option<&Error<T, R>> {
        if let Status::Gone(ref e) = *self {
            Some(e)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub enum Timeout {
    None,
    KeepAliveAfter(Duration, tokio_timer::Timer),
    DisconnectAfter(Duration, tokio_timer::Timer),
}

impl Debug for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Timeout::*;
        match *self {
            None => write!(f, "Timeout::None"),
            KeepAliveAfter(d, _) => {
                write!(f, "Timeout::KeepAliveAfter({:?}, tokio_timer::Timer)", d)
            }
            DisconnectAfter(d, _) => {
                write!(f, "Timeout::DisconnectAfter({:?}, tokio_timer::Timer)", d)
            }
        }
    }
}

impl Timeout {
    pub fn keep_alive_after(maybe_duration: Option<Duration>,
                            timer: tokio_timer::Timer)
                            -> Timeout {
        match maybe_duration {
            Some(duration) => Timeout::KeepAliveAfter(duration, timer),
            None => Timeout::None,
        }
    }

    pub fn disconnect_after(maybe_duration: Option<Duration>,
                            timer: tokio_timer::Timer)
                            -> Timeout {
        match maybe_duration {
            Some(duration) => Timeout::DisconnectAfter(duration, timer),
            None => Timeout::None,
        }
    }

    pub fn to_sleep(&self) -> Option<tokio_timer::Sleep> {
        match *self {
            Timeout::None => None,
            Timeout::KeepAliveAfter(duration, ref timer) |
            Timeout::DisconnectAfter(duration, ref timer) => Some(timer.sleep(duration)),
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
        (rx_from_client, tx_to_client, Client::new(id.to_string(), Timeout::None, tx, rx))
    }
}
