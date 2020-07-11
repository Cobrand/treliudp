use treliudp::{Treliudp, TreliudpMessage, CommStatus};
use treliudp::reliudp::{self, SocketEvent};

use treliudp::bincode;
use bincode::config::Options;

fn deser_message(data: impl AsRef<[u8]>) -> String {
    treliudp::treliudp_bincode_options().deserialize::<String>(data.as_ref()).unwrap()
}

// fn ser_message(message: &str) -> Arc<[u8]> {
//     Arc::from(treliudp::treliudp_bincode_options().serialize(&message).unwrap())
// }

fn main() -> Result<(), Box<dyn ::std::error::Error>> {
    env_logger::init();

    let mut server = reliudp::RUdpServer::new("0.0.0.0:61200").expect("Failed to create reliudp server");

    // we have to assign a type to Treliudp, since the "parsing" is done automatically. A vec will be more than enough
    // to retrieve a buffer.
    let mut treliudp = Treliudp::<String, String>::connect("127.0.0.1:61200").expect("failed to start UDP socket");

    for n in 0i32..1200 {
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

        ::std::thread::sleep(::std::time::Duration::from_millis(16));
    }

    Ok(())
}