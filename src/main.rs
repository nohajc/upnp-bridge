use std::{
    net::Ipv4Addr,
    net::{IpAddr, SocketAddr},
};

use anyhow::anyhow;
use clap::{Parser, Subcommand};
use httparse::Status;
use ssdp::MutlicastType;
use tokio::signal;

mod grpc;

mod client;
mod server;
mod ssdp;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Client {
        #[clap(value_parser)]
        /// host of the server
        host: String,
        #[arg(short, long, default_value = "1907")]
        /// port of the server
        port: u16,
    },
    Server {
        #[arg(short, long, default_value = "1907")]
        /// port of the server
        port: u16,
    },
}

fn handle_request(addr: SocketAddr, buf: &[u8]) -> anyhow::Result<()> {
    println!("{:?} bytes received from {:?}", buf.len(), addr);
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);
    if let Status::Partial = req.parse(buf)? {
        return Err(anyhow!("incomplete message"));
    }
    println!(
        "method: {:?}, path: {:?}, version: {:?}, headers: {:?}, raw:",
        req.method, req.path, req.version, req.headers
    );
    println!("{}-----", String::from_utf8_lossy(buf));

    Ok(())
}

fn handle_response(addr: SocketAddr, buf: &[u8]) -> anyhow::Result<()> {
    println!("{:?} bytes received from {:?}", buf.len(), addr);
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut resp = httparse::Response::new(&mut headers);
    if let Status::Partial = resp.parse(buf)? {
        return Err(anyhow!("incomplete message"));
    }

    if addr.ip() == IpAddr::from(Ipv4Addr::new(192, 168, 1, 112)) {
        println!(
            "{:?} {:?}, version: {:?}, headers: {:?}, raw:",
            resp.code, resp.reason, resp.version, resp.headers
        );
        println!("{}-----", String::from_utf8_lossy(buf));
    }

    Ok(())
}

async fn test_multicast_listener() -> anyhow::Result<()> {
    let bindaddr = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 1900));
    let multiaddr = Ipv4Addr::new(239, 255, 255, 250);
    let sock = ssdp::udp_bind_multicast(bindaddr, multiaddr, MutlicastType::Listener)?;

    let mut buf = [0; 65535];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        if let Err(e) = handle_request(addr, &buf[0..len]) {
            println!("failed to parse message as HTTP request: {}", e);
        }
    }
}

async fn test_multicast_sender() -> anyhow::Result<()> {
    let bindaddr = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 0));
    let multiaddr = Ipv4Addr::new(239, 255, 255, 250);
    let sock = ssdp::udp_bind_multicast(bindaddr, multiaddr, MutlicastType::Sender)?;

    let req = b"M-SEARCH * HTTP/1.1\r
HOST: 239.255.255.250:1900\r
MAN: \"ssdp:discover\"\r
MX: 5\r
ST: urn:schemas-upnp-org:device:MediaServer:1\r
\r
";

    sock.send_to(req, "239.255.255.250:1900").await?;

    let mut buf = [0; 65535];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        if let Err(e) = handle_response(addr, &buf[0..len]) {
            println!("failed to parse message as HTTP response: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    tokio::spawn(async {
        signal::ctrl_c().await.unwrap();
        std::process::exit(0);
    });

    match cli.command {
        Commands::Client { host, port } => {
            client::run(SocketAddr::from((host.parse::<IpAddr>()?, port))).await?;
        }
        Commands::Server { port } => {
            server::run(SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), port))).await?;
        }
    }

    test_multicast_sender().await?;
    test_multicast_listener().await?;

    Ok(())
}
