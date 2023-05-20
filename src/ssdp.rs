use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::anyhow;
use default_net::{get_default_interface, ip::Ipv4Net};
use httparse::Header;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

pub enum MutlicastType {
    Listener(IpAddr),
    Sender,
}

fn get_first_iface_addr(addrs: &Vec<Ipv4Net>) -> anyhow::Result<Ipv4Addr> {
    match addrs.len() > 0 {
        true => Ok(addrs[0].addr),
        false => Err(anyhow!("failed to detect local IP address")),
    }
}

pub fn udp_bind_multicast(addr: SocketAddr, mc_type: MutlicastType) -> anyhow::Result<UdpSocket> {
    let domain = match &addr.ip() {
        IpAddr::V4(_) => Domain::IPV4,
        IpAddr::V6(_) => Domain::IPV6,
    };
    let sock = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    sock.bind(&addr.into())?;

    let iface = get_default_interface().map_err(|e| anyhow!(e))?;
    match mc_type {
        MutlicastType::Listener(multiaddr) => match &multiaddr {
            IpAddr::V4(addr) => {
                let ifaceaddr = get_first_iface_addr(&iface.ipv4)?;
                sock.join_multicast_v4(addr, &ifaceaddr)?;
            }
            IpAddr::V6(addr) => {
                sock.join_multicast_v6(addr, iface.index)?;
            }
        },
        MutlicastType::Sender => match &addr.ip() {
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

pub trait HeaderMap<'b> {
    fn get_header(self, name: &str) -> Option<&'b [u8]>;
}

impl<'h, 'b> HeaderMap<'b> for &'h mut [Header<'b>] {
    fn get_header(self, name: &str) -> Option<&'b [u8]> {
        for f in self {
            if f.name == name {
                return Some(f.value);
            }
        }
        None
    }
}

pub trait RequestHeaderMap<'b> {
    fn get_request_header(self, buf: &'b [u8], name: &str) -> Option<&'b [u8]>;
}

impl<'h, 'b> RequestHeaderMap<'b> for &'h mut [Header<'b>] {
    fn get_request_header(self, buf: &'b [u8], name: &str) -> Option<&'b [u8]> {
        let mut req = httparse::Request::new(self);
        if let Ok(httparse::Status::Complete(_)) = req.parse(buf) {
            req.headers.get_header(name)
        } else {
            None
        }
    }
}

pub trait ResponseHeaderMap<'b> {
    fn get_response_header(self, buf: &'b [u8], name: &str) -> Option<&'b [u8]>;
}

impl<'h, 'b> ResponseHeaderMap<'b> for &'h mut [Header<'b>] {
    fn get_response_header(self, buf: &'b [u8], name: &str) -> Option<&'b [u8]> {
        let mut resp = httparse::Response::new(self);
        if let Ok(httparse::Status::Complete(_)) = resp.parse(buf) {
            resp.headers.get_header(name)
        } else {
            None
        }
    }
}
