//! Threaded Reliudp, or Threaded Reliable UDP

pub use reliudp;
pub use serde;
pub use bincode;

use std::net::ToSocketAddrs;
use std::thread;
use std::time::Duration;

use reliudp::{RUdpSocket, SocketEvent, MessageType};
use bincode::{config as bincode_config, Config as BincodeConfig};

use std::sync::mpsc::{Sender, Receiver, channel, TryRecvError};
use std::sync::Arc;
use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug)]
pub enum CommStatus {
    Connecting,
    Connected,
    Terminated,
}

use std::io::{Error as IoError, Result as IoResult};

pub struct Treliudp<R: DeserializeOwned + Send, S: Serialize + Send> {
    pub (crate) status: CommStatus,
    pub (crate) errors: Vec<IoError>,
    pub (crate) receiver: Receiver<T2LMessage<R>>,
    pub (crate) sender: Sender<L2TMessage<S>>,
}

impl<R: DeserializeOwned + Send + 'static, S: Serialize + Send + 'static> Treliudp<R, S> {
    /// Creates a threaded Socket and connects to the remote instantly.
    /// 
    /// This will fail ONLY if there is something wrong with the network, preventing it to create a UDP Socket.
    pub fn connect<A: ToSocketAddrs>(remote_addr: A) -> IoResult<Treliudp<R, S>> {
        let rudp = RUdpSocket::connect(remote_addr)?;

        let (l2t_sender, l2t_receiver) = channel::<L2TMessage<S>>();
        let (t2l_sender, t2l_receiver) = channel::<T2LMessage<R>>();

        let mut threaded_socket = ThreadedSocket::new(rudp, l2t_receiver, t2l_sender);

        thread::spawn(move || threaded_socket.init());

        Ok(Treliudp {
            status: CommStatus::Connecting,
            errors: vec!(),
            receiver: t2l_receiver,
            sender: l2t_sender,
        })
    }

    /// Receive incoming messages.
    ///
    /// * `None` means that no more messages are expected. You should wait before polling again.
    /// * `Some(Ok(message))` means `message` has been received.
    /// * `Some(Err(()))` means that the remote client has disconnected
    pub fn next_incoming(&mut self) -> Option<Result<Box<R>, ()>> {
        'receiver: loop {
            match self.receiver.try_recv() {
                Ok(T2LMessage::Error(io_err)) => {
                    self.errors.push(io_err);
                },
                Ok(T2LMessage::StatusChange(new_status)) => {
                    self.status = new_status;
                }
                Ok(T2LMessage::Message(boxed_message)) => {
                    break 'receiver Some(Ok(boxed_message))
                },
                Err(TryRecvError::Empty) => {
                    break 'receiver None
                },
                Err(TryRecvError::Disconnected) => {
                    break 'receiver Some(Err(()))
                }
            }
        }
    }

    pub fn send_data(&mut self, message: Box<S>, kind: MessageType) {
        let _i = self.sender.send(L2TMessage::Message(message, kind));
        // ignore the result: if the channel has hung up, then it doesn't matter anyway
    }

    pub fn disconnect(&mut self) {
        let _i = self.sender.send(L2TMessage::Stop);
    }

    pub fn drain_errors(&mut self) -> impl Iterator<Item=IoError> + '_ {
        self.errors.drain(..)
    }
}

pub (crate) struct ThreadedSocket<R: DeserializeOwned + Send, S: Serialize + Send> {
    pub (crate) socket: RUdpSocket,
    pub (crate) serde_config: BincodeConfig,

    pub (crate) receiver: Receiver<L2TMessage<S>>,
    pub (crate) sender: Sender<T2LMessage<R>>,

    pub (crate) should_stop: bool,
}

impl<R: DeserializeOwned + Send, S: Serialize + Send> ThreadedSocket<R, S> {
    pub (crate) fn new(socket: RUdpSocket, receiver: Receiver<L2TMessage<S>>, sender: Sender<T2LMessage<R>>) -> ThreadedSocket<R, S> {
        let mut serde_config = bincode_config();
        serde_config.limit(256 * 1220); // 305KB, reliudp can't hold messages that big 
        ThreadedSocket {
            socket,
            serde_config,
            receiver,
            sender,
            should_stop: false,
        }
    }

    pub (crate) fn init(&mut self) {
        self.socket.set_timeout_delay(500 * 10); // equivalent to 10s 
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
                    log::warn!("socket matching {:?} timeout-ed", self.socket.remote_addr());
                    let _x = self.sender.send(T2LMessage::StatusChange(CommStatus::Terminated));
                    self.should_stop = true;
                },
                SocketEvent::Aborted | SocketEvent::Ended => {
                    log::warn!("received termination from remote {:?}", self.socket.remote_addr());
                    let _x = self.sender.send(T2LMessage::StatusChange(CommStatus::Terminated));
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
        self.serde_config.deserialize::<Box<R>>(&data)
    }

    fn process_outgoing(&mut self) {
        'outgoing: loop {
            match self.receiver.try_recv() {
                Ok(L2TMessage::Stop) => {
                    self.should_stop = true;
                },
                Ok(L2TMessage::Message(m, t)) => {
                    let r = self.serde_config.serialize::<Box<S>>(&m);
                    match r {
                        Ok(d) => {
                            let d: Arc<_> = Arc::from(d.into_boxed_slice());
                            self.socket.send_data(d, t);
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
    Message(Box<S>, MessageType),
}

/// Threaded to local message
#[derive(Debug)]
pub (crate) enum T2LMessage<R: DeserializeOwned + Send> {
    Error(IoError),
    StatusChange(CommStatus),
    Message(Box<R>),
}