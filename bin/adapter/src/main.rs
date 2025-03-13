use alloy_rpc_types_engine::{PayloadId, PayloadStatus};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{body::Incoming, server::conn::http1, service::service_fn, Request, Response};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::{
    client::legacy::Client,
    rt::{TokioExecutor, TokioIo},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::SocketAddr;
use tokio::{self, net::TcpListener};
use tracing::{error, info, Level};

const RETH_AUTH: &str = "http://127.0.0.1:8651";
const RETH_HTTP: &str = "http://127.0.0.1:8544";

const RESS_AUTH: &str = "http://127.0.0.1:8552";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RethPayloadResponse {
    #[serde(rename = "payloadStatus")]
    pub payload_status: PayloadStatus,
    #[serde(rename = "payloadId")]
    pub payload_id: Option<PayloadId>,
}

async fn forward_request(
    req: Request<Incoming>,
    is_auth_server: bool,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let reth_uri = if is_auth_server { RETH_AUTH } else { RETH_HTTP };
    let req_method = req.method().clone();
    let req_headers = req.headers().clone();
    let whole_body = req.collect().await?.to_bytes();
    let request_body: serde_json::Value = serde_json::from_slice(&whole_body).unwrap();

    // `is_engine_method` as true if:
    //  1) method starts with "engine"
    //  2) method does NOT start with "engine_get"
    let is_engine_method = request_body["method"]
        .as_str()
        .map(|method| method.starts_with("engine") && !method.starts_with("engine_get"))
        .unwrap_or(false);

    let https =
        HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
    let client = Client::builder(TokioExecutor::new()).build(https);
    let build_request = |uri: &str| {
        let mut builder = Request::builder().method(req_method.clone()).uri(uri);
        for (key, value) in req_headers.iter() {
            builder = builder.header(key, value);
        }
        builder.body(Full::new(whole_body.clone()).boxed()).unwrap()
    };

    info!(target: "adapter", "Sending request to reth");
    let reth_req = build_request(reth_uri);
    let reth_res = client.request(reth_req).await.unwrap();
    let (parts, body) = reth_res.into_parts();
    // if it's not engine method, return reth response
    if !is_engine_method {
        let boxed_body = BoxBody::new(body);
        info!(target: "adapter", "Sending response from reth");
        return Ok(Response::from_parts(parts, boxed_body));
    }

    let reth_body_bytes = body.collect().await?.to_bytes();
    info!(target: "adapter", "Sending request to ress");
    let ress_req = build_request(RESS_AUTH);
    let ress_res = client.request(ress_req).await.unwrap();
    let (mut ress_parts, ress_body) = ress_res.into_parts();
    let ress_body_bytes = ress_body.collect().await?.to_bytes();

    // Process payload ID replacement
    let top_level: Value = serde_json::from_slice(&reth_body_bytes).unwrap();
    if let Some(result_value) = top_level.get("result") {
        if let Ok(reth_response) =
            serde_json::from_value::<RethPayloadResponse>(result_value.clone())
        {
            if let Some(reth_payload_id) = reth_response.payload_id {
                let mut ress_top_level: Value = serde_json::from_slice(&ress_body_bytes).unwrap();
                if let Some(result_obj) = ress_top_level.get_mut("result") {
                    if let Some(result_map) = result_obj.as_object_mut() {
                        result_map.insert(
                            "payloadId".to_string(),
                            serde_json::Value::String(reth_payload_id.to_string()),
                        );
                    }
                }
                let new_body = serde_json::to_vec(&ress_top_level).unwrap();
                ress_parts.headers.remove(hyper::header::CONTENT_LENGTH);
                ress_parts.headers.insert(
                    hyper::header::CONTENT_LENGTH,
                    new_body.len().to_string().parse().unwrap(),
                );
                info!(target: "adapter", "Sending response from ress");
                return Ok(Response::from_parts(ress_parts, full(new_body)));
            }
        }
    }
    info!(target: "adapter", "Sending response from ress");
    Ok(Response::from_parts(ress_parts, full(ress_body_bytes)))
}

pub(crate) fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into()).map_err(|never| match never {}).boxed()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let auth_addr: SocketAddr = ([0, 0, 0, 0], 8551).into();
    let http_addr: SocketAddr = ([0, 0, 0, 0], 8545).into();

    let auth_listener = TcpListener::bind(auth_addr).await?;
    let http_listener = TcpListener::bind(http_addr).await?;
    info!("Listening on http://{} and http://{}", auth_addr, http_addr);

    let auth_task = tokio::spawn(async move {
        loop {
            let (auth_tcp, _) = auth_listener.accept().await.unwrap();
            let auth_io = TokioIo::new(auth_tcp);
            tokio::spawn(async move {
                if let Err(err) = http1::Builder::new()
                    .serve_connection(auth_io, service_fn(|req| forward_request(req, true)))
                    .await
                {
                    error!("Error serving auth connection: {:?}", err);
                }
            });
        }
    });
    let http_task = tokio::spawn(async move {
        loop {
            let (http_tcp, _) = http_listener.accept().await.unwrap();
            let http_io = TokioIo::new(http_tcp);
            tokio::spawn(async move {
                if let Err(err) = http1::Builder::new()
                    .serve_connection(http_io, service_fn(|req| forward_request(req, false)))
                    .await
                {
                    error!("Error serving http connection: {:?}", err);
                }
            });
        }
    });
    tokio::try_join!(auth_task, http_task)?;
    Ok(())
}
