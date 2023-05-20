use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::anyhow;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use crate::grpc::client_request::ReqOneof;
use crate::grpc::{bridge_client::BridgeClient, ClientRequest, ServerResponse};
use crate::grpc::{Endpoint, MSearchRequest};
use crate::ssdp::{self, HeaderMap, MutlicastType};

async fn open_stream(
    client: &mut BridgeClient<Channel>,
    req_rx: mpsc::Receiver<ClientRequest>,
    resp_tx: mpsc::Sender<ServerResponse>,
) -> anyhow::Result<()> {
    let req = tonic::Request::new(ReceiverStream::new(req_rx));
    let mut resp = client.open(req).await?.into_inner();

    tokio::spawn(async move {
        while let Some(resp) = resp.message().await.ok().flatten() {
            _ = resp_tx.send(resp).await;
        }
    });

    Ok(())
}

pub async fn run(addr: SocketAddr) -> anyhow::Result<()> {
    let endpoint = format!("http://{}:{}", addr.ip(), addr.port());
    let mut client = BridgeClient::connect(endpoint).await?;

    let (req_tx, req_rx) = mpsc::channel(16);
    let (resp_tx, mut resp_rx) = mpsc::channel(16);

    open_stream(&mut client, req_rx, resp_tx).await?;

    tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            log::info!("received response: {:?}", &resp);
        }
    });

    let bindaddr = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 1900));
    let multiaddr = IpAddr::from([239, 255, 255, 250]);
    let sock = ssdp::udp_bind_multicast(bindaddr, MutlicastType::Listener(multiaddr))?;

    let mut buf = [0; 65535];
    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
        process_request(addr, &buf[0..len], &req_tx).await?;
    }
}

trait OctetsVec {
    fn octets(&self) -> Vec<u8>;
}

impl OctetsVec for IpAddr {
    fn octets(&self) -> Vec<u8> {
        match self {
            IpAddr::V4(ip) => ip.octets().to_vec(),
            IpAddr::V6(ip) => ip.octets().to_vec(),
        }
    }
}

async fn process_request(
    addr: SocketAddr,
    buf: &[u8],
    req_tx: &mpsc::Sender<ClientRequest>,
) -> anyhow::Result<()> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    if let httparse::Status::Partial = req.parse(buf)? {
        return Err(anyhow!("incomplete message"));
    }
    if req.method == Some("M-SEARCH") {
        log::info!(
            "sending M-SEARCH request; ST: '{}'",
            String::from_utf8_lossy(req.headers.get_header("ST").unwrap_or(&[]))
        );
        req_tx
            .send(ClientRequest {
                req_oneof: Some(ReqOneof::MSearch(MSearchRequest {
                    payload: buf.into(),
                    source: Some(Endpoint {
                        ip: addr.ip().octets(),
                        port: addr.port().into(),
                    }),
                })),
            })
            .await?;
    }
    Ok(())
}
