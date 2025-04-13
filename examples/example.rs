use std::time::Duration;

fn main() {
    let mut poll = mio::Poll::new().unwrap();
    let mut events = mio::Events::with_capacity(2);

    /* Timer */
    let mut timer1 = timerfd_mio::TimerFd::new().unwrap();
    timer1.set_timeout_interval(Duration::from_millis(600), Duration::from_millis(300)).unwrap();
    poll.registry().register(&mut timer1, mio::Token(1), mio::Interest::READABLE).unwrap();

    let mut timer2 = timerfd_mio::TimerFd::new().unwrap();
    timer2.set_timeout_interval(Duration::from_millis(1000), Duration::from_millis(1000)).unwrap();
    poll.registry().register(&mut timer2, mio::Token(2), mio::Interest::READABLE).unwrap();

    loop {
        poll.poll(&mut events, None).unwrap();

        for event in &events {
            if event.token() == mio::Token(1) {
                timer1.read().unwrap();
                println!("Timer 1 event");
            }
            if event.token() == mio::Token(2) {
                // this function check timer overrun
                timer2.read_and_check_overrun().unwrap();
                println!("Timer 2 event");
            }
        }
    }
}
