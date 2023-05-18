use std::{
    net::Ipv4Addr,
    net::{IpAddr, SocketAddr},
};

use anyhow::anyhow;
use default_net::{get_default_interface, ip::Ipv4Net};
use httparse::Status;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::{net::UdpSocket, signal};

enum MutlicastType {
    Listener,
    Sender,
}

fn get_first_iface_addr(addrs: &Vec<Ipv4Net>) -> anyhow::Result<Ipv4Addr> {
    match addrs.len() > 0 {
        true => Ok(addrs[0].addr),
        false => Err(anyhow!("failed to detect local IP address")),
    }
}

fn udp_bind_multicast(
    addr: impl Into<SockAddr>,
    multiaddr: impl Into<IpAddr>,
    mc_type: MutlicastType,
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
    match mc_type {
        MutlicastType::Listener => match &multiaddr {
            IpAddr::V4(addr) => {
                let ifaceaddr = get_first_iface_addr(&iface.ipv4)?;
                sock.join_multicast_v4(addr, &ifaceaddr)?;
            }
            IpAddr::V6(addr) => {
                sock.join_multicast_v6(addr, iface.index)?;
            }
        },
        MutlicastType::Sender => match &multiaddr {
            IpAddr::V4(_) => {
                let ifaceaddr = get_first_iface_addr(&iface.ipv4)?;
                sock.set_multicast_if_v4(&ifaceaddr)?;
            }
            IpAddr::V6(_) => {
                sock.set_multicast_if_v6(iface.index)?;
            }
        },
    }

    let udp_sock = UdpSocket::from_std(sock.into())?;
    Ok(udp_sock)
}

fn handle_request(addr: &SocketAddr, buf: &[u8]) -> anyhow::Result<()> {
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

fn handle_response(addr: &SocketAddr, buf: &[u8]) -> anyhow::Result<()> {
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
    let sock = udp_bind_multicast(bindaddr, multiaddr, MutlicastType::Listener)?;

    let mut buf = [0; 65535];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        if let Err(e) = handle_request(&addr, &buf[0..len]) {
            println!("failed to parse message as HTTP request: {}", e);
        }
    }
}

async fn test_multicast_sender() -> anyhow::Result<()> {
    let bindaddr = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 0));
    let multiaddr = Ipv4Addr::new(239, 255, 255, 250);
    let sock = udp_bind_multicast(bindaddr, multiaddr, MutlicastType::Sender)?;

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
        if let Err(e) = handle_response(&addr, &buf[0..len]) {
            println!("failed to parse message as HTTP response: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::spawn(async {
        signal::ctrl_c().await.unwrap();
        std::process::exit(0);
    });

    test_multicast_sender().await?;
    test_multicast_listener().await?;

    Ok(())
}
