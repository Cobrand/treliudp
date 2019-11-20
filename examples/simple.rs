use treliudp::Treliudp;
use treliudp::reliudp::{self, SocketEvent};

use treliudp::bincode;

use std::sync::Arc;

fn generate_message(i: i32) -> String {
    let x = i * (256i32 - i);
    format!("my message is : {0} x (256 - {0}) = {1}", i, x)
}

fn deser_message(bincode_config: &bincode::Config, data: impl AsRef<[u8]>) -> String {
    bincode_config.deserialize::<String>(data.as_ref()).unwrap()
}

fn ser_message(bincode_config: &bincode::Config, message: &str) -> Arc<[u8]> {
    Arc::from(bincode_config.serialize(&message).unwrap())
}

fn main() -> Result<(), Box<dyn ::std::error::Error>> {
    env_logger::init();

    let server_bincode_config = bincode::config();

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
                    println!("local server: Incoming message \"{}\"", deser_message(&server_bincode_config, &d));
                },
                _ => {
                    println!("local server: Incoming event {:?}", server_event);
                }
            }
        }

        if n % 120 == 0 {
            let message_to_send = generate_message(n);
            println!("local server (n={:?}): Sending message \"{}\" to all {:?} remotes", n, message_to_send, server.remotes_len());
            server.send_data(&ser_message(&server_bincode_config, &message_to_send), reliudp::MessageType::KeyMessage);
        }

        // CLIENT PART
        // for the client part, next_tick is done automatically, so we don't really have to care about that
        while let Some(m) = treliudp.next_incoming() {
            match m {
                Ok(message) => {
                    println!("client (n={:?}): Received message \"{}\"", n, message);
                },
                Err(_) => {
                    println!("client (n={:?}): remote server has disconnected unexpectedly", n);
                    break;
                }
            }
        }
        if n % 180 == 0 {
            let message_to_send = generate_message(n);
            println!("client (n={:?}): Sending message \"{}\" to server", n, message_to_send);
            treliudp.send_data(Box::new(message_to_send), reliudp::MessageType::KeyMessage);
        }

        ::std::thread::sleep(::std::time::Duration::from_millis(16));
    }

    drop(server);

    ::std::thread::sleep(::std::time::Duration::from_millis(1000));


    while let Some(message) = treliudp.next_incoming() {
        match message {
            Ok(message) => {
                println!("client: Received (n=final) \"{}\"", message);
            },
            Err(treliudp::TerminateType::Ended) => {
                println!("client: remote server has disconnected expectedly");
                return Ok(());
            },
            Err(e) => {
                panic!("unexpected error {:?} received, expected a peacefull shutdown", e);
            }
        }
    }

    Ok(())
}