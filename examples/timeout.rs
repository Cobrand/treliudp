
use treliudp::{Treliudp, CommStatus, TerminateKind};

fn main() -> Result<(), Box<dyn ::std::error::Error>> {
    env_logger::init();

    // we have to assign a type to Treliudp, since the "parsing" is done automatically. A vec will be more than enough
    // to retrieve a buffer.
    let mut treliudp = Treliudp::<String, String>::connect("127.0.0.1:61244").expect("failed to start UDP socket");
    println!("starting a client without a remote");

    for _ in 0..10 {
        'm: loop {
            match treliudp.next_incoming() {
                None => break 'm,
                Some(Ok(_)) => panic!("there shouldn't be a server connected to {}", treliudp.remote_addr()),
                Some(Err(_)) => panic!("there shouldn't be a server connected to {}", treliudp.remote_addr()),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    std::thread::sleep(std::time::Duration::from_secs(1));

    assert_eq!(treliudp.next_incoming(), Some(Err(TerminateKind::Timeout)));
    assert_eq!(treliudp.status(), CommStatus::Terminated(TerminateKind::Timeout));

    Ok(())
}
