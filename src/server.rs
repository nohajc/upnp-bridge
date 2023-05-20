use std::{net::SocketAddr, pin::Pin};

use crate::grpc::{
    bridge_server::{self, BridgeServer},
    server_response::RespOneof,
    ClientRequest, Endpoint, MSearchResponse, ServerResponse,
};
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Request, Response, Status, Streaming};

pub struct BridgeService {}

impl BridgeService {
    pub fn new() -> BridgeService {
        BridgeService {}
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

        tokio::spawn(async move {
            while let Some(req) = stream.message().await.ok().flatten() {
                log::info!("received request: {:?}", &req);
                _ = tx
                    .send(Ok(ServerResponse {
                        // TODO: Retransmit the m-search request received by client,
                        // wait for response, retransmit the response back to the client.
                        resp_oneof: Some(RespOneof::MSearch(MSearchResponse {
                            payload: vec![],
                            req_source: Some(Endpoint {
                                ip: [].into(),
                                port: 0,
                            }),
                        })),
                    }))
                    .await;
            }
        });

        Ok(Response::new(rx_all))
    }
}

pub async fn run(addr: SocketAddr) -> anyhow::Result<()> {
    let br = BridgeService::new();
    let svc = BridgeServer::new(br);

    Server::builder().add_service(svc).serve(addr).await?;

    Ok(())
}
