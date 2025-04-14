#![warn(missing_debug_implementations)]
#[cfg(not(target_os = "linux"))]
compile_error!("This crate is only compatible with Linux.");

use rustix::fd::OwnedFd;
use rustix::time::{Itimerspec, TimerfdClockId, Timespec};
use std::os::unix::prelude::*;
use rustix::time::{TimerfdFlags, TimerfdTimerFlags};
use std::time::Duration;
use mio::event::Source;
use mio::unix::SourceFd;
use mio::{Interest, Registry, Token};
use std::io;

const TS_NULL: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerFdFlag {
    Default,
    Abstime,
    TimerCancelOnSet,
}

#[derive(Debug)]
pub struct TimerFd(OwnedFd);
impl TimerFd {

    pub fn new_custom(
        clock: TimerfdClockId,
        nonblocking: bool,
        cloexec: bool,
    ) -> std::io::Result<TimerFd> {
        let mut flags = TimerfdFlags::empty();
        if nonblocking {
            flags |= TimerfdFlags::NONBLOCK;
        }
        if cloexec {
            flags |= TimerfdFlags::CLOEXEC;
        }

        let fd = rustix::time::timerfd_create(clock, flags)?;
        Ok(TimerFd(fd))
    }

    pub fn new() -> std::io::Result<TimerFd> {
        TimerFd::new_custom(TimerfdClockId::Monotonic, true, true)
    }

    #[inline]
    fn duration_to_timespec(duration: Duration) -> Timespec {
        Timespec { tv_sec: duration.as_secs() as i64, tv_nsec: duration.subsec_nanos() as i64 }
    }

    #[inline]
    fn timespec_to_duration(timespec: Timespec) -> Duration {
        Duration::new(timespec.tv_sec as u64, timespec.tv_nsec as u32)
    }

    fn convert_timerfd_flags(flags: TimerFdFlag) -> TimerfdTimerFlags {
        match flags {
            TimerFdFlag::Default => TimerfdTimerFlags::empty(),
            TimerFdFlag::Abstime => TimerfdTimerFlags::ABSTIME,
            TimerFdFlag::TimerCancelOnSet => {
                TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET
            }
        }
    }

    pub fn set_timeout_interval_and_flags(&mut self, value: Duration, interval: Duration, sflags: TimerFdFlag) -> std::io::Result<()>  {
        let flags = Self::convert_timerfd_flags(sflags);

        let timer_spec: Itimerspec = Itimerspec {
            it_value: Self::duration_to_timespec(value),
            it_interval: Self::duration_to_timespec(interval),
        };
        rustix::time::timerfd_settime(&self.0, flags, &timer_spec)?;
        Ok(())
    }

    pub fn set_timeout_interval(&mut self, value: Duration, interval: Duration) -> std::io::Result<()> {
        Self::set_timeout_interval_and_flags(self, value, interval, TimerFdFlag::Default)
    }

    pub fn set_timeout_oneshot_and_flags(&mut self, value: Duration, flags: TimerFdFlag) -> std::io::Result<()> {
        let flags = Self::convert_timerfd_flags(flags);
        let timer_spec: Itimerspec = Itimerspec {
            it_value: Self::duration_to_timespec(value),
            it_interval: TS_NULL,
        };
        rustix::time::timerfd_settime(&self.0, flags, &timer_spec)?;
        Ok(())
    }

    pub fn set_timeout_oneshot(&mut self, value: Duration) -> std::io::Result<()> {
        Self::set_timeout_oneshot_and_flags(self, value, TimerFdFlag::Default)
    }

    pub fn disarm(&mut self) {
        let timer_spec: Itimerspec = Itimerspec {
            it_value: TS_NULL,
            it_interval: TS_NULL,
        };
        rustix::time::timerfd_settime(&self.0, TimerfdTimerFlags::empty(), &timer_spec).unwrap();
    }

    pub fn get_remaining_time(&self) -> io::Result<Duration> {
        Ok(Self::timespec_to_duration(rustix::time::timerfd_gettime(&self.0)?.it_value))
    }

    pub fn get_interval(&self) -> io::Result<Duration> {
        Ok(Self::timespec_to_duration(rustix::time::timerfd_gettime(&self.0)?.it_interval))
    }

    pub fn read(&self) -> io::Result<u64> {
        let mut buffer = [0_u8; 8];
        loop {
            match rustix::io::read(&self.0, &mut buffer) {

                // Read number of timer expirations
                Ok(8) => { 
                    let value = u64::from_ne_bytes(buffer);
                    assert_ne!(value, 0);
                    return Ok(value);
                }

                // non-blocking mode : return no timer expirations
                Err(rustix::io::Errno::WOULDBLOCK) => return Ok(0), 

                // Interrupted by a signal, read again
                Err(rustix::io::Errno::INTR) => (), 

                // Error handling
                Err(e) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Unexpected rustix::io::read error: {}", e),
                    ));
                }
                _ => unreachable!(),
            }
        }
    }

    pub fn read_and_check_overrun(&self) -> io::Result<bool> {
        let res = self.read()?;
        match res {
            0 => Ok(false), // No timer expirations
            1 => Ok(true),  // One timer expiration
            x =>
            // More than one timer expiration
            {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Timer overrun: missed {} timer expirations", x - 1),
                ))
            }
        }
    }
}

impl AsRawFd for TimerFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl AsFd for TimerFd {
    fn as_fd(&self) -> rustix::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Source for TimerFd {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        SourceFd(&self.as_raw_fd()).register(registry, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        SourceFd(&self.as_raw_fd()).reregister(registry, token, interests)
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        SourceFd(&self.as_raw_fd()).deregister(registry)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mio::{Events, Poll};
    use std::time::{Duration, Instant};

    fn is_fd_open(fd: RawFd) -> bool {
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
        rustix::io::fcntl_getfd(borrowed_fd).is_ok()
    }

    #[test]
    fn test_new_timerfd() {
        let timer_fd = TimerFd::new().expect("Failed to create TimerFd");
        assert!(timer_fd.as_raw_fd() > 0);
    }

    #[test]
    fn test_duration_to_timespec_and_back() {
        let duration = Duration::new(5, 500_000_000); // 5.5 seconds
        let timespec = TimerFd::duration_to_timespec(duration);
        assert_eq!(timespec.tv_sec, 5);
        assert_eq!(timespec.tv_nsec, 500_000_000);

        let converted_duration = TimerFd::timespec_to_duration(timespec);
        assert_eq!(duration, converted_duration);
    }

    #[test]
    fn test_set_timeout_oneshot() {
        let mut timer_fd = TimerFd::new().unwrap();
        let timeout = Duration::from_secs(1);
        timer_fd.set_timeout_oneshot(timeout).unwrap();

        let remaining_time = timer_fd.get_remaining_time().unwrap();
        assert!(remaining_time <= timeout);
    }

    #[test]
    fn test_set_timeout_interval() {
        let mut timer_fd = TimerFd::new().unwrap();
        let timeout = Duration::from_secs(1);
        let interval = Duration::from_secs(2);

        timer_fd.set_timeout_interval(timeout, interval).unwrap();

        let remaining_time = timer_fd.get_remaining_time().unwrap();
        let configured_interval = timer_fd.get_interval().unwrap();

        assert!(remaining_time <= timeout);
        assert_eq!(configured_interval, interval);
    }

    #[test]
    fn test_disarm_timer() {
        let mut timer_fd = TimerFd::new().unwrap();
        let timeout = Duration::from_secs(1);
        timer_fd.set_timeout_oneshot(timeout).unwrap();

        timer_fd.disarm();

        let remaining_time = timer_fd.get_remaining_time().unwrap();
        assert_eq!(remaining_time, Duration::ZERO);
    }

    #[test]
    fn test_read_expiration_count() {
        let mut timer_fd = TimerFd::new().unwrap();
        let timeout = Duration::from_millis(100);
        timer_fd.set_timeout_oneshot(timeout).unwrap();

        // Wait for the timer to expire
        std::thread::sleep(timeout + Duration::from_millis(50));

        let expirations = timer_fd.read().unwrap();
        assert_eq!(expirations, 1);
    }

    #[test]
    fn test_read_and_check_overrun() {
        let mut timer_fd = TimerFd::new().unwrap();
        let timeout = Duration::from_millis(100);

        // Set a repeating timer
        timer_fd.set_timeout_interval(timeout, timeout).unwrap();

        // Wait long enough for multiple expirations
        std::thread::sleep(timeout * 5);

        // Verify that an overrun is detected
        match timer_fd.read_and_check_overrun() {
            Err(err) => assert!(err.to_string().contains("Timer overrun")),
            _ => panic!("Expected timer expirations"),
        }
    }

    #[test]
    fn test_register_with_mio() {
        use mio::{Events, Poll};

        let mut timer_fd = TimerFd::new().unwrap();
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(128);

        poll.registry()
            .register(&mut timer_fd, Token(0), Interest::READABLE)
            .unwrap();

        let timeout = Duration::from_millis(100);
        timer_fd.set_timeout_oneshot(timeout).unwrap();

        poll.poll(&mut events, Some(timeout * 2)).unwrap();

        let event = events.iter().next().unwrap();
        assert_eq!(event.token(), Token(0));
        assert!(event.is_readable());
    }

    #[test]
    fn oneshot_timeout() {
        const TOKEN: Token = Token(0);
        const TIMEOUT: Duration = Duration::from_millis(100);

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        let mut timer = TimerFd::new().unwrap();

        timer.set_timeout_oneshot(TIMEOUT).unwrap();
        poll.registry().register(&mut timer, TOKEN, Interest::READABLE).unwrap();

        // timer should not elapse before the timeout
        poll.poll(&mut events, Some(TIMEOUT / 2)).unwrap();
        assert!(events.is_empty());
        assert!(timer.read().unwrap() == 0);

        // timer should elapse after its timeout has passed
        poll.poll(&mut events, Some(TIMEOUT)).unwrap();
        assert!(!events.is_empty());
        assert!(timer.read().unwrap() == 1);

        // timer should not elapse again without a rearm
        poll.poll(&mut events, Some(TIMEOUT)).unwrap();
        assert!(events.is_empty());
        assert!(timer.read().unwrap() == 0);

        // timer should elapse after the rearmed timeout
        timer.set_timeout_oneshot(TIMEOUT).unwrap();
        poll.poll(&mut events, Some(TIMEOUT * 2)).unwrap();
        assert!(!events.is_empty());
        assert!(timer.read().unwrap() == 1);
    }

    #[test]
    fn test_timerfd_closes_on_drop() {
        let rawfd: i32;

        // Test with scope
        {
            let timer_fd = TimerFd::new().expect("Failed to create TimerFd");
            rawfd = timer_fd.as_raw_fd();
            assert!(is_fd_open(rawfd));

        }
        assert!(!is_fd_open(rawfd));

        // Test with drop()
        let timer_fd = TimerFd::new().expect("Failed to create TimerFd");
        let rawfd = timer_fd.as_raw_fd();
        assert!(is_fd_open(rawfd));
        drop(timer_fd);
        assert!(!is_fd_open(rawfd));
    }

    #[test]
    fn test_multiple_timers() {
        let mut timer1 = TimerFd::new().expect("Failed to create TimerFd 1");
        let mut timer2 = TimerFd::new().expect("Failed to create TimerFd 2");
        let mut timer3 = TimerFd::new().expect("Failed to create TimerFd 3");

        let duration1 = Duration::from_millis(100);
        let duration2 = Duration::from_millis(200);
        let duration3 = Duration::from_millis(300);

        timer1.set_timeout_oneshot(duration1).expect("Failed to set timer 1");
        timer2.set_timeout_oneshot(duration2).expect("Failed to set timer 2");
        timer3.set_timeout_oneshot(duration3).expect("Failed to set timer 3");

        let start = Instant::now();

        while timer1.read().unwrap() == 0 {
            assert!(start.elapsed() < duration1 + Duration::from_millis(50));
        }
        println!("Timer 1 expired!");

        while timer2.read().unwrap() == 0 {
            assert!(start.elapsed() < duration2 + Duration::from_millis(50));
        }
        println!("Timer 2 expired!");

        while timer3.read().unwrap() == 0 {
            assert!(start.elapsed() < duration3 + Duration::from_millis(50));
        }
        println!("Timer 3 expired!");
    }
}