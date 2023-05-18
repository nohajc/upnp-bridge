use std::{
    net::Ipv4Addr,
    net::{IpAddr, SocketAddr},
};

use anyhow::anyhow;
use default_net::get_default_interface;
use httparse::Status;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::{net::UdpSocket, signal};

fn udp_bind_multicast(
    addr: impl Into<SockAddr>,
    multiaddr: impl Into<IpAddr>,
) -> anyhow::Result<UdpSocket> {
    let multiaddr = multiaddr.into();
    let domain = match &multiaddr {
        IpAddr::V4(_) => Domain::IPV4,
        IpAddr::V6(_) => Domain::IPV6,
    };
    let sock = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    sock.bind(&addr.into())?;

    let iface = get_default_interface().map_err(|e| anyhow!(e))?;
    match &multiaddr {
        IpAddr::V4(addr) => {
            let ifaceaddr = match iface.ipv4.len() > 0 {
                true => Ok(iface.ipv4[0].addr),
                false => Err(anyhow!("failed to detect local IP address")),
            }?;
            sock.join_multicast_v4(addr, &ifaceaddr)?;
        }
        IpAddr::V6(addr) => {
            sock.join_multicast_v6(addr, iface.index)?;
        }
    }

    let udp_sock = UdpSocket::from_std(sock.into())?;
    Ok(udp_sock)
}

fn handle_message(addr: &SocketAddr, buf: &[u8]) -> anyhow::Result<()> {
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
    println!("{}\n", String::from_utf8_lossy(buf));

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::spawn(async {
        signal::ctrl_c().await.unwrap();
        std::process::exit(0);
    });

    let bindaddr = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 1900));
    let multiaddr = Ipv4Addr::new(239, 255, 255, 250);
    let sock = udp_bind_multicast(bindaddr, multiaddr)?;

    let mut buf = [0; 65535];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        if let Err(e) = handle_message(&addr, &buf[0..len]) {
            println!("failed to parse message as HTTP request: {}", e);
        }
    }
}
