use std::future::poll_fn;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use quinn::crypto::rustls::QuicClientConfig;
use quinn::{ClientConfig, Connection, Endpoint};

use super::*;

/// How long a setup may take when nothing holds it up (the old serial accept took 10 s).
const PROMPT: Duration = Duration::from_secs(2);

fn server() -> WebTransportServer {
    WebTransportServer::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap()
}

/// A QUIC client that speaks HTTP/3 and accepts any certificate, like a browser with a pin.
fn client() -> Endpoint {
    let mut tls = quic_tls::client_config().unwrap();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let tls = QuicClientConfig::try_from(tls).unwrap();
    let mut endpoint = Endpoint::client(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    endpoint.set_default_client_config(ClientConfig::new(Arc::new(tls)));
    endpoint
}

async fn connect(endpoint: &Endpoint, server: &WebTransportServer) -> Result<Connection> {
    let addr = server.local_addr()?;
    endpoint
        .connect(addr, quic_tls::SERVER_NAME)
        .map_err(network)?
        .await
        .map_err(network)
}

/// Sends a plain HTTP/3 request, which the server answers with 404 once it runs this
/// connection's setup.
async fn status_of_plain_request(connection: Connection) -> u16 {
    let (mut driver, mut requests) = h3::client::new(h3_quinn::Connection::new(connection))
        .await
        .unwrap();
    let driving = tokio::spawn(async move { poll_fn(|cx| driver.poll_close(cx)).await });
    let request = http::Request::get("https://beam/other").body(()).unwrap();
    let mut stream = requests.send_request(request).await.unwrap();
    stream.finish().await.unwrap();
    let status = stream.recv_response().await.unwrap().status().as_u16();
    driving.abort();
    status
}

#[tokio::test]
async fn a_stalled_browser_does_not_hold_up_the_next() {
    let server = server();
    let endpoint = client();
    let _stalled = connect(&endpoint, &server).await.unwrap(); // connects, then does nothing

    let started = Instant::now();
    let next = connect(&endpoint, &server).await.unwrap();
    let status = timeout(PROMPT, status_of_plain_request(next)).await;

    assert_eq!(
        status.ok(),
        Some(404),
        "the second setup waited for the first"
    );
    assert!(started.elapsed() < PROMPT);
}

#[tokio::test]
async fn one_address_gets_a_limited_number_of_setups() {
    let server = server();
    let endpoint = client();
    let mut stalled = Vec::new();
    for _ in 0..MAX_SETTING_UP_PER_ADDRESS {
        stalled.push(connect(&endpoint, &server).await.unwrap());
    }

    let refused = timeout(PROMPT, connect(&endpoint, &server)).await;
    assert!(
        matches!(refused, Ok(Err(_))),
        "a setup beyond the limit was let in"
    );

    drop(stalled.pop()); // a slot frees once its connection is gone...
    tokio::time::sleep(Duration::from_millis(200)).await;
    let again = timeout(PROMPT, connect(&endpoint, &server)).await;
    assert!(matches!(again, Ok(Ok(_))), "the freed slot was not reused"); // ...and is reused
}
