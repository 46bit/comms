use std::time::Duration;
use futures::{Future, Sink, Stream, Poll, Async};
use super::*;

pub struct Receive<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    client: Option<Client<I, C>>,
}

impl<I, C> Receive<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>) -> Receive<I, C> {
        Receive { client: Some(client) }
    }

    pub fn with_hard_timeout(mut self,
                             duration: Duration,
                             timer: &tokio_timer::Timer)
                             -> ReceiveWithHardTimeout<I, C> {
        ReceiveWithHardTimeout::new(self.client.take().unwrap(), duration, timer)
    }

    pub fn with_soft_timeout(mut self,
                             duration: Duration,
                             timer: &tokio_timer::Timer)
                             -> ReceiveWithSoftTimeout<I, C> {
        ReceiveWithSoftTimeout::new(self.client.take().unwrap(), duration, timer)
    }

    fn take_client(&mut self) -> Client<I, C> {
        self.client.take().expect("Polled after Async::Ready.")
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for Receive<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (C::Item, Client<I, C>);
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let client_poll = {
            let client = self.client.as_mut().unwrap();
            client.poll()
        };
        match client_poll {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(Some(msg))) => Ok(Async::Ready((msg, self.take_client()))),
            Ok(Async::Ready(None)) |
            Err(_) => Err(self.take_client()),
        }
    }
}

pub struct ReceiveWithHardTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    client: Option<Client<I, C>>,
    sleep: tokio_timer::Sleep,
}

impl<I, C> ReceiveWithHardTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>,
               timeout_duration: Duration,
               timer: &tokio_timer::Timer)
               -> ReceiveWithHardTimeout<I, C> {
        ReceiveWithHardTimeout {
            client: Some(client),
            sleep: timer.sleep(timeout_duration),
        }
    }

    fn take_client(&mut self) -> Client<I, C> {
        self.client.take().expect("Polled after Async::Ready.")
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for ReceiveWithHardTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (C::Item, Client<I, C>);
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let client_poll = {
            let client = self.client.as_mut().unwrap();
            client.poll()
        };
        match client_poll {
            Ok(Async::NotReady) => {}
            Ok(Async::Ready(Some(msg))) => {
                return Ok(Async::Ready((msg, self.client.take().unwrap())));
            }
            Ok(Async::Ready(None)) |
            Err(_) => return Err(self.take_client()),
        }

        match self.sleep.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(_)) => {
                let mut client = self.take_client();
                if client.inner.is_ok() {
                    client.inner = Err(Disconnect::Timeout);
                }
                Err(client)
            }
            Err(e) => {
                let mut client = self.take_client();
                if client.inner.is_ok() {
                    client.inner = Err(Disconnect::Timer(e));
                }
                Err(client)
            }
        }
    }
}

pub struct ReceiveWithSoftTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    client: Option<Client<I, C>>,
    sleep: tokio_timer::Sleep,
}

impl<I, C> ReceiveWithSoftTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>,
               timeout_duration: Duration,
               timer: &tokio_timer::Timer)
               -> ReceiveWithSoftTimeout<I, C> {
        ReceiveWithSoftTimeout {
            client: Some(client),
            sleep: timer.sleep(timeout_duration),
        }
    }

    fn take_client(&mut self) -> Client<I, C> {
        self.client.take().expect("Polled after Async::Ready.")
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for ReceiveWithSoftTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (Option<C::Item>, Client<I, C>);
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let client_poll = {
            let client = self.client.as_mut().unwrap();
            client.poll()
        };
        match client_poll {
            Ok(Async::NotReady) => {}
            Ok(Async::Ready(Some(msg))) => return Ok(Async::Ready((Some(msg), self.take_client()))),
            Ok(Async::Ready(None)) |
            Err(_) => return Err(self.take_client()),
        }

        match self.sleep.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(_)) => Ok(Async::Ready((None, self.take_client()))),
            Err(e) => {
                let mut client = self.take_client();
                if client.inner.is_ok() {
                    client.inner = Err(Disconnect::Timer(e));
                }
                Err(client)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test::*;
    use futures::{lazy, Future};
    use std::mem::drop;
    use std::time::Instant;

    #[test]
    fn can_receive() {
        let f = |f| f;
        let g = |g: Vec<_>| g.into_iter().map(|i| (i, i)).collect();
        subtest_well_behaved(&f, vec![]);
        subtest_well_behaved(&f, g(vec![CopyMsg::B(5)]));
        subtest_well_behaved(&f, g(vec![CopyMsg::B(98), CopyMsg::A]));
        subtest_well_behaved(&f, g(vec![CopyMsg::B(98), CopyMsg::A, CopyMsg::B(543)]));
    }

    #[test]
    fn can_receive_with_hard_timeout() {
        let timer = tokio_timer::Timer::default();
        // @TODO: Warn that timeouts below 101 millis seem to trigger immediately.
        let mut millis = 200;
        while millis < 1200 {
            let duration = Duration::from_millis(millis);
            let f = |f: Receive<_, _>| f.with_hard_timeout(duration, &timer);

            // Check that ReceiveWithHardTimeout is well behaved for reading the stream.
            let g = |g: Vec<_>| g.into_iter().map(|i| (i, i)).collect();
            subtest_well_behaved(&f, vec![]);
            subtest_well_behaved(&f, g(vec![CopyMsg::B(5)]));
            subtest_well_behaved(&f, g(vec![CopyMsg::B(98), CopyMsg::A]));
            subtest_well_behaved(&f, g(vec![CopyMsg::B(98), CopyMsg::A, CopyMsg::B(543)]));

            // Check that ReceiveWithHardTimeout is well behaved for timing out.
            let timeouted_client = subtest_timeout_happens(&f, duration);
            // Ensure a timeout results in connection closure.
            assert_eq!(timeouted_client.status().is_connected(), false);
            assert_eq!(timeouted_client.status().is_disconnected(),
                       Some(&Disconnect::Timeout));

            millis += 50;
        }
    }

    #[test]
    fn can_receive_with_soft_timeout() {
        let timer = tokio_timer::Timer::default();
        // @TODO: Warn that timeouts below 101 millis seem to trigger immediately.
        let mut millis = 200;
        while millis < 1200 {
            let duration = Duration::from_millis(millis);
            let f = |f: Receive<_, _>| f.with_soft_timeout(duration, &timer);

            // Check that ReceiveWithSoftTimeout is well behaved for reading the stream.
            let g = |g: Vec<_>| g.into_iter().map(|i| (i, Some(i))).collect();
            subtest_well_behaved(&f, vec![]);
            subtest_well_behaved(&f, g(vec![CopyMsg::B(5)]));
            subtest_well_behaved(&f, g(vec![CopyMsg::B(98), CopyMsg::A]));
            subtest_well_behaved(&f, g(vec![CopyMsg::B(98), CopyMsg::A, CopyMsg::B(543)]));

            // Check that ReceiveWithSoftTimeout is well behaved for timing out.
            let timeouted_client = subtest_timeout_happens(&f, duration);
            // Ensure a timeout does not result in connection closure.
            assert!(timeouted_client.status().is_connected());
            assert_eq!(timeouted_client.status().is_disconnected(), None);

            millis += 50;
        }
    }

    fn subtest_well_behaved<R, F, G>(f: F, msgs: Vec<(CopyMsg, R)>)
        where F: Fn(Receive<String, Unsplit<mpsc::Sender<CopyMsg>, mpsc::Receiver<CopyMsg>>>) -> G,
              R: PartialEq + Eq + Debug,
              G: Future<Item = (R, MpscClient<String, CopyMsg>),
                        Error = MpscClient<String, CopyMsg>>
    {
        lazy(move || {
                let (_, mut tx, mut client) = mock_client_copy("client1", 1);

                // Check a Receive with nothing to read is not ready.
                let mut fut = f(client.receive());
                assert_eq!(fut.poll(), Ok(Async::NotReady));

                for (msg, expectation) in msgs {
                    // Check previously sinked msg was only provided once.
                    assert_eq!(fut.poll(), Ok(Async::NotReady));

                    // Verify an sinked msg is provided.
                    tx = tx.send(msg).wait().unwrap();
                    if let Ok(Async::Ready((msg2, c))) = fut.poll() {
                        assert_eq!(msg2, expectation);
                        client = c;
                    } else {
                        unreachable!();
                    }

                    fut = f(client.receive());
                }

                // Verify that dropping the channel being received on correctly closes the
                // client sink.
                drop(tx);
                if let Err(client) = fut.poll() {
                    assert_eq!(client.status().is_disconnected(),
                               Some(&Disconnect::Dropped));
                } else {
                    unreachable!();
                }

                // // Verify that after `Dropped` it clamps output to `Ready(None)`.
                // for _ in 0..5 {
                //     assert_eq!(fut.poll(), Ok(Async::Ready(None)));
                // }

                // // @TODO: Needs `into_inner` to be constrained by the type.
                // // Verify the internal client `Disconnect` reason is still `Dropped`.
                // //client = fut.into_inner();

                Ok::<(), ()>(())
            })
            .wait()
            .unwrap();
    }

    fn subtest_timeout_happens<R, F, G>(f: F, duration: Duration) -> MpscClient<String, CopyMsg>
        where F: Fn(Receive<String, Unsplit<mpsc::Sender<CopyMsg>, mpsc::Receiver<CopyMsg>>>) -> G,
              R: PartialEq + Eq + Debug,
              G: Future<Item = (R, MpscClient<String, CopyMsg>),
                        Error = MpscClient<String, CopyMsg>>
    {
        lazy(move || {
                let (_, tx, mut client) = mock_client_copy("client1", 1);

                let start = Instant::now();
                let mut fut = f(client.receive());
                // Check it isn't immediately ready.
                assert_eq!(fut.poll(), Ok(Async::NotReady));

                loop {
                    match fut.poll() {
                        Ok(Async::NotReady) => continue,
                        Ok(Async::Ready((_, c))) |
                        Err(c) => {
                            client = c;
                            break;
                        }
                    }
                }
                // Ensure the channel is kept open until here, to force the timeout to happen.
                drop(tx);

                let elapsed = start.elapsed();
                // Check we waited at least half of the correct duration.
                assert!(duration < elapsed * 2);
                // Check we waited no more than twice the correct duration.
                assert!(elapsed < duration * 2);

                Ok::<_, ()>(client)
            })
            .wait()
            .unwrap()
    }
}
