use alloy_rpc_types_engine::{PayloadId, PayloadStatus};
use futures_util::TryFutureExt;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Client, Request, Response, Server,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::Infallible;
use tokio::io::{self};
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
    req: Request<Body>,
    is_auth_server: bool,
) -> Result<Response<Body>, hyper::Error> {
    let (parts, body) = req.into_parts();
    let whole_body = hyper::body::to_bytes(body).await?;
    let body_str = String::from_utf8_lossy(&whole_body);

    let mut is_engine_method = false;
    if is_auth_server {
        if let Ok(json_body) = serde_json::from_str::<Value>(&body_str) {
            if let Some(method) = json_body.get("method") {
                info!("method: {}", method);
                if method.as_str().unwrap_or_default().starts_with("engine") {
                    is_engine_method = true;
                }
            }
        }
    }

    let client = Client::new();
    let build_request = |uri: &str| {
        let mut builder = Request::builder().method(parts.method.clone()).uri(uri);
        for (key, value) in parts.headers.iter() {
            builder = builder.header(key, value);
        }
        builder.body(Body::from(whole_body.clone()))
    };

    // Send request to Reth and await its response
    let reth = if is_auth_server { RETH_AUTH } else { RETH_HTTP };
    let reth_req = build_request(reth).unwrap();
    let reth_resp = client.request(reth_req).await?;
    info!("Received from Reth: {:?}", reth_resp);
    let (reth_parts, reth_body) = reth_resp.into_parts();
    let reth_body_bytes = hyper::body::to_bytes(reth_body).await?;
    let is_payload_id = match serde_json::from_slice::<RethPayloadResponse>(&reth_body_bytes) {
        Ok(json_value) => json_value.payload_id,
        Err(_) => None,
    };

    // If it's an engine method, send request to Ress and await its response
    if is_engine_method {
        let ress_req = build_request(RESS_AUTH).unwrap();
        info!("Sending request to Ress: {:?}", ress_req);

        let ress_resp = client.request(ress_req).await?;
        info!("Received response from Ress: {:?}", ress_resp);
        let (ress_parts, ress_body) = ress_resp.into_parts();
        if let Some(reth_payload_id) = is_payload_id {
            info!("reth payload id: {:?}", reth_payload_id);
            let ress_body_bytes = hyper::body::to_bytes(ress_body).await?;
            let mut payload =
                serde_json::from_slice::<RethPayloadResponse>(&ress_body_bytes).unwrap();
            payload.payload_id = Some(reth_payload_id);
            let new_body_bytes = serde_json::to_vec(&payload).unwrap_or_default();
            let new_response = Response::from_parts(ress_parts, Body::from(new_body_bytes));
            return Ok(new_response);
        } else {
            let ress_body_bytes = hyper::body::to_bytes(ress_body).await?;
            let new_body_bytes = serde_json::to_vec(&ress_body_bytes).unwrap_or_default();
            let new_response = Response::from_parts(ress_parts, Body::from(new_body_bytes));
            return Ok(new_response);
        }
    }

    let new_body_bytes = serde_json::to_vec(&reth_body_bytes).unwrap_or_default();
    let new_response = Response::from_parts(reth_parts, Body::from(new_body_bytes));
    Ok(new_response)
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let auth = ([0, 0, 0, 0], 8551).into();
    let http = ([0, 0, 0, 0], 8545).into();

    let auth_server = Server::bind(&auth).serve(make_service_fn(|_conn| async {
        Ok::<_, Infallible>(service_fn(|req| forward_request(req, true)))
    }));

    let http_server = Server::bind(&http).serve(make_service_fn(|_conn| async {
        Ok::<_, Infallible>(service_fn(|req| forward_request(req, false)))
    }));

    info!("Listening on http://{} and http://{}", auth, http);

    tokio::try_join!(
        auth_server.map_err(|e| {
            error!("Auth server error: {}", e);
            io::Error::new(io::ErrorKind::Other, e)
        }),
        http_server.map_err(|e| {
            error!("Http error: {}", e);
            io::Error::new(io::ErrorKind::Other, e)
        })
    )?;

    Ok(())
}
