use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use super::upnp::{bridge_client::BridgeClient, ClientRequest, ServerResponse};

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
