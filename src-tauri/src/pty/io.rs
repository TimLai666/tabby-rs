#[cfg(unix)]
mod unix {
    use std::{
        fs::File,
        io::{self, Read, Write},
        os::fd::{AsRawFd, FromRawFd, RawFd},
        os::unix::net::UnixStream,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
    };

    #[derive(Clone, Copy, Debug)]
    enum IoState {
        Running,
        Cancelled,
    }

    pub(crate) struct StopHandle {
        state: Arc<Mutex<IoState>>,
        reader: Arc<File>,
        disconnected: Arc<AtomicBool>,
        writer: Mutex<Option<UnixStream>>,
    }

    #[derive(Clone)]
    pub(crate) struct StopToken {
        state: Arc<Mutex<IoState>>,
        reader: Arc<File>,
        disconnected: Arc<AtomicBool>,
    }

    pub(crate) struct Reader<R> {
        inner: R,
        master: File,
        stop: StopToken,
    }

    pub(crate) struct Writer<W> {
        inner: W,
        master: File,
        stop: StopToken,
    }

    pub(crate) fn setup(master_fd: RawFd) -> io::Result<(StopHandle, File)> {
        let flags = unsafe { libc::fcntl(master_fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let poll_fd = unsafe { libc::fcntl(master_fd, libc::F_DUPFD_CLOEXEC, 0) };
        if poll_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let poll_master = unsafe { File::from_raw_fd(poll_fd) };
        let (stop_reader, stop_writer) = UnixStream::pair()?;
        set_cloexec(stop_reader.as_raw_fd())?;
        set_cloexec(stop_writer.as_raw_fd())?;
        let reader = Arc::new(stop_reader_to_file(stop_reader));
        let disconnected = Arc::new(AtomicBool::new(false));
        Ok((
            StopHandle {
                state: Arc::new(Mutex::new(IoState::Running)),
                reader,
                disconnected: Arc::clone(&disconnected),
                writer: Mutex::new(Some(stop_writer)),
            },
            poll_master,
        ))
    }

    fn stop_reader_to_file(stream: UnixStream) -> File {
        let fd = stream.into_raw_fd();
        unsafe { File::from_raw_fd(fd) }
    }

    use std::os::fd::IntoRawFd;

    fn set_cloexec(fd: RawFd) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    impl StopHandle {
        pub(crate) fn token(&self) -> StopToken {
            StopToken {
                state: Arc::clone(&self.state),
                reader: Arc::clone(&self.reader),
                disconnected: Arc::clone(&self.disconnected),
            }
        }

        pub(crate) fn cancel(&self) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            *state = IoState::Cancelled;
            drop(state);
            let writer = self
                .writer
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            drop(writer);
        }
    }

    impl StopToken {
        fn state(&self) -> IoState {
            *self.state.lock().unwrap_or_else(|error| error.into_inner())
        }

        fn cancelled() -> io::Error {
            io::Error::new(io::ErrorKind::ConnectionAborted, "PTY I/O cancelled")
        }

        fn check_writer(&self) -> io::Result<()> {
            match self.state() {
                IoState::Running => Ok(()),
                IoState::Cancelled => Err(Self::cancelled()),
            }
        }

        fn wait(&self, master: RawFd, events: i16) -> io::Result<()> {
            loop {
                if matches!(self.state(), IoState::Cancelled) {
                    return Err(Self::cancelled());
                }
                let observe_stop = !self.disconnected.load(Ordering::Acquire);
                let mut fds = [
                    libc::pollfd {
                        fd: master,
                        events,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: if observe_stop {
                            self.reader.as_raw_fd()
                        } else {
                            -1
                        },
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if matches!(self.state(), IoState::Cancelled) {
                    return Err(Self::cancelled());
                }
                if fds[0].revents != 0 {
                    return Ok(());
                }
                if observe_stop
                    && fds[1].revents
                        & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
                        != 0
                {
                    self.disconnected.store(true, Ordering::Release);
                }
            }
        }
    }

    impl<R: Read> Read for Reader<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return self.inner.read(buf);
            }
            loop {
                let mut state = self
                    .stop
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                match *state {
                    IoState::Running => {}
                    IoState::Cancelled => return Err(StopToken::cancelled()),
                }
                match self.inner.read(buf) {
                    Ok(0) => {
                        *state = IoState::Cancelled;
                        return Ok(0);
                    }
                    Ok(length) => {
                        drop(state);
                        return Ok(length);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        drop(state);
                        self.stop.wait(self.master.as_raw_fd(), libc::POLLIN)?;
                    }
                    result => return result,
                }
            }
        }
    }

    impl<W: Write> Write for Writer<W> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return self.inner.write(buf);
            }
            self.stop.check_writer()?;
            loop {
                match self.inner.write(buf) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        self.stop.wait(self.master.as_raw_fd(), libc::POLLOUT)?;
                        self.stop.check_writer()?;
                    }
                    result => return result,
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    pub(crate) fn wrap_reader<R: Read>(inner: R, master: File, token: StopToken) -> Reader<R> {
        Reader {
            inner,
            master,
            stop: token,
        }
    }
    pub(crate) fn wrap_writer<W: Write>(inner: W, master: File, token: StopToken) -> Writer<W> {
        Writer {
            inner,
            master,
            stop: token,
        }
    }
}

#[cfg(unix)]
pub(crate) use unix::{StopHandle as UnixStopHandle, *};
