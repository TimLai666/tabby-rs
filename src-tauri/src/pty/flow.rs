use std::sync::{Condvar, Mutex};

#[derive(Debug)]
struct FlowState {
    attached: bool,
    closed: bool,
    unacked: usize,
}

#[derive(Debug)]
pub struct FlowControl {
    max_unacked: usize,
    state: Mutex<FlowState>,
    wake: Condvar,
}

impl FlowControl {
    pub fn new(max_unacked: usize) -> Self {
        Self {
            max_unacked,
            state: Mutex::new(FlowState {
                attached: false,
                closed: false,
                unacked: 0,
            }),
            wake: Condvar::new(),
        }
    }

    pub fn attach(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.attached = true;
        state.unacked = 0;
        self.wake.notify_all();
    }

    pub fn detach(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.attached = false;
        state.unacked = 0;
        self.wake.notify_all();
        true
    }

    pub fn reserve(&self, bytes: usize) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while !state.closed
            && (!state.attached || state.unacked.saturating_add(bytes) > self.max_unacked)
        {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        if state.closed {
            return false;
        }
        state.unacked = state.unacked.saturating_add(bytes);
        true
    }

    pub fn ack(&self, bytes: usize) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.unacked = state.unacked.saturating_sub(bytes);
        let drained = state.unacked == 0;
        self.wake.notify_all();
        drained
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        self.wake.notify_all();
    }

    pub fn is_drained(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .unacked
            == 0
    }

    #[cfg(test)]
    fn unacked(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .unacked
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{mpsc, Arc},
        thread,
        time::Duration,
    };

    use super::FlowControl;

    #[test]
    fn output_is_blocked_until_attached_and_acknowledged() {
        let flow = Arc::new(FlowControl::new(10));
        let worker = {
            let flow = Arc::clone(&flow);
            thread::spawn(move || {
                assert!(flow.reserve(8));
                assert!(flow.reserve(3));
            })
        };

        thread::sleep(Duration::from_millis(20));
        assert_eq!(flow.unacked(), 0);
        flow.attach();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(flow.unacked(), 8);
        assert!(flow.ack(8));
        worker.join().unwrap();
        assert_eq!(flow.unacked(), 3);
    }

    #[test]
    fn renderer_disconnect_pauses_new_output_until_reattach() {
        let flow = Arc::new(FlowControl::new(10));
        flow.attach();
        assert!(flow.reserve(10));
        assert!(flow.detach());
        assert_eq!(flow.unacked(), 0);

        let (sent, received) = mpsc::channel();
        let worker = {
            let flow = Arc::clone(&flow);
            thread::spawn(move || sent.send(flow.reserve(1)).unwrap())
        };
        assert!(received.recv_timeout(Duration::from_millis(20)).is_err());
        flow.attach();
        assert!(received.recv_timeout(Duration::from_secs(1)).unwrap());
        worker.join().unwrap();
    }

    #[test]
    fn one_byte_and_exact_window_chunks_are_supported() {
        let flow = FlowControl::new(10);
        flow.attach();
        assert!(flow.reserve(1));
        assert!(flow.ack(1));
        assert!(flow.reserve(10));
        assert_eq!(flow.unacked(), 10);
        assert!(flow.ack(usize::MAX));
        assert_eq!(flow.unacked(), 0);
        assert!(flow.is_drained());
    }

    #[test]
    fn simulated_gibibyte_stream_never_exceeds_window() {
        const CHUNK: usize = 100 * 1024;
        const WINDOW: usize = CHUNK * 5;
        const ONE_GIB: usize = 1024 * 1024 * 1024;

        let flow = FlowControl::new(WINDOW);
        flow.attach();
        let mut processed = 0usize;
        while processed < ONE_GIB {
            assert!(flow.reserve(CHUNK));
            assert!(flow.unacked() <= WINDOW);
            assert!(flow.ack(CHUNK));
            processed = processed.saturating_add(CHUNK);
        }
    }

    #[test]
    fn close_releases_waiters() {
        let flow = Arc::new(FlowControl::new(1));
        let worker = {
            let flow = Arc::clone(&flow);
            thread::spawn(move || flow.reserve(1))
        };
        thread::sleep(Duration::from_millis(20));
        flow.close();
        assert!(!worker.join().unwrap());
    }
}
