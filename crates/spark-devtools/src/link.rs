//! Transport: newline-delimited JSON over a TCP stream, with I/O on background threads.

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::marker::PhantomData;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Messages waiting to be written. When the peer stops reading, newer messages are dropped instead
/// of growing memory (stats / scenes are snapshots: losing one is harmless).
const OUTBOX: usize = 256;

/// One end of a devtools connection: sends `Out`, receives `In`. Never blocks the caller.
pub struct Link<Out, In> {
    outbox: SyncSender<Out>,
    inbox: Receiver<In>,
    alive: Arc<AtomicBool>,
    peer: SocketAddr,
    _types: PhantomData<fn(Out) -> In>,
}

impl<Out, In> Link<Out, In>
where
    Out: Serialize + Send + 'static,
    In: DeserializeOwned + Send + 'static,
{
    /// Connects to a listening [`Server`] (the game side).
    pub fn connect(addr: &str, timeout: Duration) -> io::Result<Self> {
        let target = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("bad devtools address '{addr}'")))?;
        Self::from_stream(TcpStream::connect_timeout(&target, timeout)?)
    }

    /// Wraps an accepted / connected stream and starts the reader and writer threads.
    pub fn from_stream(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(false)?;
        let _ = stream.set_nodelay(true);
        let peer = stream.peer_addr()?;
        let read_half = stream.try_clone()?;
        let alive = Arc::new(AtomicBool::new(true));
        let (outbox, out_rx) = mpsc::sync_channel::<Out>(OUTBOX);
        let (in_tx, inbox) = mpsc::channel::<In>();

        let writer_alive = alive.clone();
        std::thread::Builder::new().name("spark-devtools-write".into()).spawn(move || {
            let mut w = BufWriter::new(stream);
            while let Ok(first) = out_rx.recv() {
                // Write everything queued, then flush once.
                let mut next = Some(first);
                while let Some(msg) = next.take() {
                    if write_line(&mut w, &msg).is_err() {
                        writer_alive.store(false, Ordering::SeqCst);
                        return;
                    }
                    next = out_rx.try_recv().ok();
                }
                if w.flush().is_err() {
                    writer_alive.store(false, Ordering::SeqCst);
                    return;
                }
            }
            // Our side was dropped: tell the peer we are done.
            let _ = w.flush();
            let _ = w.get_ref().shutdown(std::net::Shutdown::Write);
        })?;

        let reader_alive = alive.clone();
        std::thread::Builder::new().name("spark-devtools-read".into()).spawn(move || {
            for line in BufReader::new(read_half).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<In>(&line) {
                    Ok(msg) => {
                        if in_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    // A newer peer may send messages this version does not know.
                    Err(e) => log::debug!("devtools: skipped a message ({e})"),
                }
            }
            reader_alive.store(false, Ordering::SeqCst);
        })?;

        Ok(Self { outbox, inbox, alive, peer, _types: PhantomData })
    }

    /// Queues a message. `false` when the link is closed or the outbox is full (message dropped).
    pub fn send(&self, msg: Out) -> bool {
        if !self.is_alive() {
            return false;
        }
        match self.outbox.try_send(msg) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                self.alive.store(false, Ordering::SeqCst);
                false
            }
        }
    }

    /// Next received message, if any.
    pub fn try_recv(&self) -> Option<In> {
        self.inbox.try_recv().ok()
    }

    /// All messages received so far.
    pub fn drain(&self) -> impl Iterator<Item = In> + '_ {
        self.inbox.try_iter()
    }

    /// False once the peer closed the connection or an I/O error happened.
    /// Messages received before that can still be drained.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }
}

fn write_line<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *w, msg).map_err(io::Error::other)?;
    w.write_all(b"\n")
}

/// The editor side: listens on a free localhost port, games connect to it.
pub struct Server {
    listener: TcpListener,
    addr: SocketAddr,
}

impl Server {
    /// Binds `127.0.0.1` on a free port (never reachable from other machines).
    pub fn bind_local() -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        Ok(Self { listener, addr })
    }

    /// The address to pass to the game: `--devtools <addr>`.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Non-blocking: a game that connected since the last call.
    pub fn accept<Out, In>(&self) -> Option<io::Result<Link<Out, In>>>
    where
        Out: Serialize + Send + 'static,
        In: DeserializeOwned + Send + 'static,
    {
        match self.listener.accept() {
            Ok((stream, _)) => Some(Link::from_stream(stream)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorCommand, GameMessage};
    use std::time::Instant;

    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn editor_and_game_talk() {
        let server = Server::bind_local().unwrap();
        let game: Link<GameMessage, EditorCommand> =
            Link::connect(&server.addr().to_string(), Duration::from_secs(2)).unwrap();
        let editor: Link<EditorCommand, GameMessage> = wait(|| server.accept()).unwrap();

        assert!(game.send(GameMessage::Paused { paused: true }));
        assert_eq!(wait(|| editor.try_recv()), GameMessage::Paused { paused: true });
        assert!(editor.send(EditorCommand::Reload));
        assert_eq!(wait(|| game.try_recv()), EditorCommand::Reload);

        drop(game);
        wait(|| (!editor.is_alive()).then_some(()));
        assert!(!editor.send(EditorCommand::Quit));
    }
}
