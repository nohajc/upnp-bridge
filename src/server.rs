use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    time::Duration,
};

use crate::{
    grpc::{
        bridge_server::{self, BridgeServer},
        client_request::ReqOneof,
        server_response::RespOneof,
        ClientRequest, Endpoint, MSearchRequest, MSearchResponse, ServerResponse,
    },
    ssdp::{self, MutlicastType, RequestHeaderMap, ResponseHeaderMap},
};

use futures::Stream;
use tokio::{
    net::UdpSocket,
    sync::mpsc::{self, Sender},
    time::timeout,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Request, Response, Status, Streaming};

pub struct BridgeService {
    multiaddr: SocketAddr,
}

impl BridgeService {
    pub fn new() -> BridgeService {
        let multiaddr = SocketAddr::from((IpAddr::from([239, 255, 255, 250]), 1900));
        BridgeService { multiaddr }
    }
}

type OpenResult = Result<ServerResponse, Status>;
type OpenResultStream = Pin<Box<dyn Stream<Item = OpenResult> + Send>>;

#[tonic::async_trait]
impl bridge_server::Bridge for BridgeService {
    type OpenStream = futures::stream::SelectAll<OpenResultStream>;

    async fn open(
        &self,
        req: Request<Streaming<ClientRequest>>,
    ) -> Result<Response<Self::OpenStream>, Status> {
        let (tx, rx) = mpsc::channel::<OpenResult>(16);
        let rx_list: Vec<OpenResultStream> = vec![Box::pin(ReceiverStream::new(rx))];
        // TODO: add broadcast channel to rx_list (for Notify requests)

        let rx_all = futures::stream::select_all(rx_list);

        let mut stream = req.into_inner();

        let multiaddr = self.multiaddr;
        tokio::spawn(async move {
            while let Some(req) = stream.message().await.transpose() {
                log::info!("next request message");

                let tx = tx.clone();
                tokio::spawn(async move {
                    let bindaddr = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 0));
                    let sock = ssdp::udp_bind_multicast(bindaddr, MutlicastType::Sender).unwrap();

                    match req {
                        Ok(req) => {
                            if let Some(oneof) = req.req_oneof {
                                match oneof {
                                    ReqOneof::MSearch(msearch) => {
                                        handle_msearch(&msearch, &sock, multiaddr, tx).await;
                                    }
                                }
                            }
                        }
                        Err(e) => log::error!("{}", e),
                    }
                });
            }
        });

        Ok(Response::new(rx_all))
    }
}

async fn handle_msearch(
    msearch: &MSearchRequest,
    sock: &UdpSocket,
    multiaddr: SocketAddr,
    tx: Sender<OpenResult>,
) {
    log::info!("received M-SEARCH request: {:?}", &msearch);

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let req_st = headers.get_request_header(&msearch.payload, "ST");

    log::info!("retransmitting to multicast address {}", multiaddr);
    if let Err(e) = sock.send_to(&msearch.payload, multiaddr).await {
        log::error!("send error: {}", e);
    }

    let mut buf = [0; 65535];
    loop {
        let t = Duration::from_secs(30);
        let recv_fut = sock.recv_from(&mut buf);
        let next = match timeout(t, recv_fut).await {
            Ok(next) => next,
            Err(_) => {
                log::info!("M-SEARCH: no respone in {:?}", t);
                break;
            }
        };

        match next {
            Ok((len, _)) => {
                let buf = &buf[0..len];
                let mut headers = [httparse::EMPTY_HEADER; 32];
                let resp_st = headers.get_response_header(buf, "ST");

                if req_st.is_some() && req_st == resp_st {
                    log::info!(
                        "matched header ST: {}, sending back SSDP response",
                        String::from_utf8_lossy(req_st.unwrap())
                    );
                    if let Err(e) = tx
                        .send(Ok(ServerResponse {
                            resp_oneof: Some(RespOneof::MSearch(MSearchResponse {
                                payload: buf.into(),
                                req_source: msearch.source.as_ref().map(|s| Endpoint {
                                    ip: s.ip.clone(),
                                    port: s.port,
                                }),
                            })),
                        }))
                        .await
                    {
                        log::error!("send error: {}", e);
                    }
                    break;
                }
            }
            Err(e) => {
                log::error!("recv error: {}", e);
                break;
            }
        }
    }
}

pub async fn run(addr: SocketAddr) -> anyhow::Result<()> {
    let br = BridgeService::new();
    let svc = BridgeServer::new(br);

    Server::builder().add_service(svc).serve(addr).await?;

    Ok(())
}
