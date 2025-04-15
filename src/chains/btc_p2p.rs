use nakamoto::client::network::Network;
use nakamoto::client::{Client, Config, Error, Handle};
use nakamoto::net::poll::Waker;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::{net, thread};

type Reactor = nakamoto::net::poll::Reactor<net::TcpStream>;

pub fn run_p2p_btc_client(
    network_name: &str,
    database_path: &str,
    seal: Option<Vec<u8>>,
) -> Result<Arc<Handle<Waker>>, Error> {
    // Parse the network type from the provided string
    let network = Network::from_str(network_name).expect("Failed to parse network type");

    // Construct the configuration for the Bitcoin client
    let config = Config::new(network, format!("{database_path}/nak-btc").into(), seal);

    // Log the network and database path for debugging purposes
    tracing::info!(
        target: "spv",
        "Initializing client with network: {:?}, database path: {:?}",
        config.network,
        config.root
    );

    // Create a new Bitcoin client instance
    let client = Client::<Reactor>::new()?;

    // Obtain a handle to the client for external control
    let client_handle = Arc::new(client.handle());

    // Spawn a new thread to run the client in the background
    thread::spawn(move || {
        client.run(config).expect("Client failed to run");
    });

    let r_h = client_handle.clone();
    thread::spawn(move || {
        restart(r_h);
    });

    // Return the handle to the client for further interaction
    Ok(client_handle)
}

fn restart(client_handle: Arc<Handle<Waker>>) {
    use nakamoto::client::handle::Handle;
    std::thread::sleep(std::time::Duration::from_secs(5 * 60));
    loop {
        let result = client_handle.get_tip();
        tracing::info!(target:"restart", "test");

        if result.is_err() {
            tracing::error!(target:"restart", "exit");
            exit(0)
            //panic!("panic");
        }
        std::thread::sleep(std::time::Duration::from_secs(40));
    }
}
