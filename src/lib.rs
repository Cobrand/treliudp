//! Threaded Reliudp, or Threaded Reliable UDP

pub use reliudp;
pub use serde;
pub use bincode;

use std::net::{ToSocketAddrs, SocketAddr};
use std::thread;
use std::time::Duration;

use reliudp::{RUdpSocket, SocketEvent, MessageType, MessagePriority};
use bincode::{options as bincode_options, config::{Options as BincodeOptions}};

use std::sync::mpsc::{Sender, Receiver, channel, TryRecvError};
use std::sync::Arc;
use serde::{de::DeserializeOwned, Serialize};

pub fn treliudp_bincode_options() -> impl BincodeOptions {
    bincode_options()
        .with_limit(256 * 1220) // reliudp can't hold messages that big
        .reject_trailing_bytes()
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TerminateKind {
    /// Timeout: We didn't receive any message from the remote for too long. 
    Timeout,
    /// The remote ended the connection unexpectedly.
    Aborted,
    /// The remote ended the connection expectedly.
    Ended,
}

impl TerminateKind {
    pub fn is_timeout(&self) -> bool {
        *self == TerminateKind::Timeout
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TreliudpMessage<R> {
    StatusChange(CommStatus),
    Msg(Box<R>),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CommStatus {
    Connecting,
    Connected,
    Terminated(TerminateKind),
}

impl CommStatus {
    pub fn is_connected(&self) -> bool {
        *self == CommStatus::Connected
    }
    
    pub fn is_connecting(&self) -> bool {
        *self == CommStatus::Connecting
    }

    pub fn is_terminated(&self) -> bool {
        if let CommStatus::Terminated(_) = *self {
            true
        } else {
            false
        }
    }
}

use std::io::{Error as IoError, Result as IoResult};

pub struct Treliudp<R: DeserializeOwned + Send, S: Serialize + Send> {
    pub (crate) status: CommStatus,
    pub (crate) remote_addr: SocketAddr,
    pub (crate) errors: Vec<IoError>,
    pub (crate) receiver: Receiver<T2LMessage<R>>,
    pub (crate) sender: Sender<L2TMessage<S>>,
}

impl<R: DeserializeOwned + Send, S: Serialize + Send> ::std::fmt::Debug for Treliudp<R, S> {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        f.debug_struct("Treliudp")
            .field("status", &self.status)
            .field("remote_addr", &self.remote_addr)
            .finish()
    }
}

impl<R: DeserializeOwned + Send + 'static, S: Serialize + Send + 'static> Treliudp<R, S> {
    /// Creates a threaded Socket and connects to the remote instantly.
    /// 
    /// This will fail ONLY if there is something wrong with the network, preventing it to create a UDP Socket.
    pub fn connect<A: ToSocketAddrs>(remote_addr: A) -> IoResult<Treliudp<R, S>> {
        let rudp = RUdpSocket::connect(remote_addr)?;
        Ok(Self::from_rudp(rudp))
    }

    /// Creates a threaded Socket given "connecting" rudp instance.
    ///
    /// Useful if you want to "customize" your instance before hand, for instance for the delay timeouts...
    pub fn from_rudp(rudp: RUdpSocket) -> Treliudp<R, S> {
        let remote_addr = rudp.remote_addr();

        let (l2t_sender, l2t_receiver) = channel::<L2TMessage<S>>();
        let (t2l_sender, t2l_receiver) = channel::<T2LMessage<R>>();

        let mut threaded_socket = ThreadedSocket::new(rudp, l2t_receiver, t2l_sender);

        thread::spawn(move || threaded_socket.init());

        Treliudp {
            status: CommStatus::Connecting,
            remote_addr,
            errors: vec!(),
            receiver: t2l_receiver,
            sender: l2t_sender,
        }
    }

    /// Receive incoming messages.
    ///
    /// * `None` means that no more messages are expected. You should wait before polling again.
    /// * `Some(Ok(message))` means `message` has been received.
    /// * `Some(Err(e))` means that the remote client has disconnected. `e` contains info about how.
    ///
    /// You must call this even if you don't use the data, otherwise `status` will never be
    /// updated.
    pub fn next_incoming(&mut self) -> Option<TreliudpMessage<R>> {
        'receiver: loop {
            match self.receiver.try_recv() {
                Ok(T2LMessage::Error(io_err)) => {
                    self.errors.push(io_err);
                },
                Ok(T2LMessage::StatusChange(new_status)) => {
                    self.status = new_status;
                    break 'receiver Some(TreliudpMessage::StatusChange(new_status))
                }
                Ok(T2LMessage::Message(boxed_message)) => {
                    break 'receiver Some(TreliudpMessage::Msg(boxed_message))
                },
                Err(TryRecvError::Empty) => {
                    break 'receiver None
                },
                Err(TryRecvError::Disconnected) => {
                    if let CommStatus::Terminated(t) = self.status() {
                        break 'receiver Some(TreliudpMessage::StatusChange(CommStatus::Terminated(t)))
                    } else {
                        log::error!("remote to local thread connection broken for {}, but status isn't Terminated!", self.remote_addr());
                        break 'receiver Some(TreliudpMessage::StatusChange(CommStatus::Terminated(TerminateKind::Aborted)))
                    }
                }
            }
        }
    }

    pub fn send_data(&mut self, message: Box<S>, kind: MessageType, priority: MessagePriority) {
        let _i = self.sender.send(L2TMessage::Message(message, kind, priority));
        // ignore the result: if the channel has hung up, then it doesn't matter anyway
    }

    pub fn disconnect(&mut self) {
        let _i = self.sender.send(L2TMessage::Stop);
    }

    pub fn drain_errors(&mut self) -> impl Iterator<Item=IoError> + '_ {
        self.errors.drain(..)
    }

    pub fn status(&self) -> CommStatus {
        self.status
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
}

pub (crate) struct ThreadedSocket<R: DeserializeOwned + Send, S: Serialize + Send> {
    pub (crate) socket: RUdpSocket,

    pub (crate) receiver: Receiver<L2TMessage<S>>,
    pub (crate) sender: Sender<T2LMessage<R>>,

    pub (crate) should_stop: bool,
}

impl<R: DeserializeOwned + Send, S: Serialize + Send> ThreadedSocket<R, S> {
    pub (crate) fn new(socket: RUdpSocket, receiver: Receiver<L2TMessage<S>>, sender: Sender<T2LMessage<R>>) -> ThreadedSocket<R, S> {
        ThreadedSocket {
            socket,
            receiver,
            sender,
            should_stop: false,
        }
    }

    pub (crate) fn init(&mut self) {
        self.main_loop();
    }

    pub (crate) fn main_loop(&mut self) {
        while !self.should_stop {
            if let Err(e) = self.socket.next_tick() {
                log::warn!("error {} was experienced while ticking", e);
                self.add_error(e);
            }
            
            self.process_incoming();
            self.process_outgoing();
            thread::sleep(Duration::from_millis(2));
        }
        log::info!("treliudp thread shutting down");
    }

    fn process_incoming(&mut self) {
        while let Some(event) = self.socket.next_event() {
            match event {
                SocketEvent::Timeout => {
                    log::warn!("socket matching {:?} timed out", self.socket.remote_addr());
                    let _x = self.sender.send(T2LMessage::StatusChange(CommStatus::Terminated(TerminateKind::Timeout)));
                    self.should_stop = true;
                },
                SocketEvent::Ended => {
                    log::warn!("received termination \"ended\" from remote {:?}", self.socket.remote_addr());
                    let _x = self.sender.send(T2LMessage::StatusChange(CommStatus::Terminated(TerminateKind::Ended)));
                    self.should_stop = true;
                },
                SocketEvent::Aborted => {
                    log::warn!("received termination \"aborted\" from remote {:?}", self.socket.remote_addr());
                    let _x = self.sender.send(T2LMessage::StatusChange(CommStatus::Terminated(TerminateKind::Aborted)));
                    self.should_stop = true;
                },
                SocketEvent::Connected => {
                    let _x = self.sender.send(T2LMessage::StatusChange(CommStatus::Connected));
                },
                SocketEvent::Data(d) => {
                    match self.deserialize_message(&d) {
                        Ok(received_message) => {
                            let _x = self.sender.send(T2LMessage::Message(received_message));
                        },
                        Err(bincode_error) => {
                            let remote_addr = self.socket.remote_addr();
                            log::warn!("received deserialization error {} from remote {}", bincode_error, remote_addr);
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn deserialize_message(&self, data: &[u8]) -> Result<Box<R>, bincode::Error> {
        treliudp_bincode_options().deserialize::<Box<R>>(&data)
    }

    fn process_outgoing(&mut self) {
        'outgoing: loop {
            match self.receiver.try_recv() {
                Ok(L2TMessage::Stop) => {
                    self.should_stop = true;
                },
                Ok(L2TMessage::Message(m, t, priority)) => {
                    let r = treliudp_bincode_options().serialize::<Box<S>>(&m);
                    match r {
                        Ok(d) => {
                            let d: Arc<_> = Arc::from(d.into_boxed_slice());
                            self.socket.send_data(d, t, priority);
                        },
                        Err(e) => {
                            let remote_addr = self.socket.remote_addr();
                            log::error!("serialization error {} from remote {}", e, remote_addr);
                        }
                    }
                },
                Err(TryRecvError::Disconnected) => {
                    log::warn!("threaded socket has no matching local thread (probably dropped), remote {:?} is shutting down", self.socket.remote_addr());
                    self.should_stop = true;
                    break 'outgoing;
                },
                Err(TryRecvError::Empty) => {
                    break 'outgoing;
                }
            }
        }
    }

    fn add_error(&mut self, error: IoError) {
        if let Err(e) = self.sender.send(T2LMessage::Error(error)) {
            log::warn!("threaded socket has no matching local thread (probably dropped) to send error {}, remote {:?} is shutting down", e, self.socket.remote_addr());
            self.should_stop = true;
        }
    }
}

/// Local to threaded message
#[derive(Debug)]
pub (crate) enum L2TMessage<S: Serialize + Send> {
    Stop,
    Message(Box<S>, MessageType, MessagePriority),
}

/// Threaded to local message
#[derive(Debug)]
pub (crate) enum T2LMessage<R: DeserializeOwned + Send> {
    Error(IoError),
    StatusChange(CommStatus),
    Message(Box<R>),
}