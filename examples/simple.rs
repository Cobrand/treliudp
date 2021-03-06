use treliudp::{Treliudp, TerminateKind, TreliudpMessage, CommStatus};
use treliudp::reliudp::{self, SocketEvent};

use treliudp::bincode::{Options};

use std::sync::Arc;

fn generate_message(i: i32) -> String {
    let x = i * (256i32 - i);
    format!("my message is : {0} x (256 - {0}) = {1}", i, x)
}

fn deser_message(data: impl AsRef<[u8]>) -> String {
    treliudp::treliudp_bincode_options().deserialize::<String>(data.as_ref()).unwrap()
}

fn ser_message(message: &str) -> Arc<[u8]> {
    Arc::from(treliudp::treliudp_bincode_options().serialize(&message).unwrap())
}

fn main() -> Result<(), Box<dyn ::std::error::Error>> {
    env_logger::init();

    let mut server = reliudp::RUdpServer::new("0.0.0.0:61244").expect("Failed to create reliudp server");

    // we have to assign a type to Treliudp, since the "parsing" is done automatically. A vec will be more than enough
    // to retrieve a buffer.
    let mut treliudp = Treliudp::<String, String>::connect("127.0.0.1:61244").expect("failed to start UDP socket");

    for n in 0i32..900 {
        // SERVER PART
        server.next_tick()?;
        // for the server, since it's not threaded, we have to process the "next_tick" manually, otherwise packets will never be
        // processed and no events will appear
        for (_socket, ref server_event) in server.drain_events() {
            match server_event {
                SocketEvent::Data(d) => {
                    println!("local server: Incoming message \"{}\"", deser_message(&d));
                },
                _ => {
                    println!("local server: Incoming event {:?}", server_event);
                }
            }
        }

        if n % 120 == 0 {
            let message_to_send = generate_message(n);
            println!("local server (n={:?}): Sending message \"{}\" to all {:?} remotes", n, message_to_send, server.remotes_len());
            server.send_data(&ser_message(&message_to_send), reliudp::MessageType::KeyMessage, reliudp::MessagePriority::Normal);
        }

        // CLIENT PART
        // for the client part, next_tick is done automatically, so we don't really have to care about that
        while let Some(m) = treliudp.next_incoming() {
            match m {
                TreliudpMessage::Msg(message) => {
                    println!("client (n={:?}): Received message \"{}\"", n, message);
                },
                TreliudpMessage::StatusChange(CommStatus::Terminated(_)) => {
                    println!("client (n={:?}): remote server has disconnected unexpectedly", n);
                    break;
                },
                _ => {}
            }
        }
        if n % 180 == 0 {
            let message_to_send = generate_message(n);
            println!("client (n={:?}): Sending message \"{}\" to server", n, message_to_send);
            treliudp.send_data(Box::new(message_to_send), reliudp::MessageType::KeyMessage, reliudp::MessagePriority::Normal);
        }

        ::std::thread::sleep(::std::time::Duration::from_millis(16));
    }

    ::std::thread::sleep(::std::time::Duration::from_millis(1000));

    while let Some(message) = treliudp.next_incoming() {
        match message {
            TreliudpMessage::Msg(message) => {
                println!("client: Received (n=final) \"{}\"", message);
            },
            TreliudpMessage::StatusChange(CommStatus::Terminated(TerminateKind::Ended)) => {
                println!("client: remote server has disconnected expectedly");
                return Ok(());
            },
            TreliudpMessage::StatusChange(CommStatus::Terminated(e)) => {
                panic!("unexpected error {:?} received, expected a peacefull shutdown", e);
            },
            _ => {},
        }
    }

    drop(server);

    Ok(())
}