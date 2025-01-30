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
            if let Some(method_value) = json_body.get("method") {
                let method_str = method_value.as_str().unwrap_or_default();
                // Mark `is_engine_method` as true if:
                //  1) method starts with "engine"
                //  2) method does NOT start with "engine_get"
                if method_str.starts_with("engine") && !method_str.starts_with("engine_get") {
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

    // If it's an engine method, send request to Ress and await its response
    if is_engine_method {
        let (_reth_parts, reth_body) = reth_resp.into_parts();
        let reth_body_bytes = hyper::body::to_bytes(reth_body).await?;
        let top_level: Value = serde_json::from_slice(&reth_body_bytes).unwrap();
        let result_value = top_level.get("result").unwrap();
        let is_payload_id =
            match serde_json::from_value::<RethPayloadResponse>(result_value.clone()) {
                Ok(json_value) => json_value.payload_id,
                Err(_) => None,
            };

        let ress_req = build_request(RESS_AUTH).unwrap();
        info!("Sending request to Ress: {:?}", ress_req);

        let ress_resp = client.request(ress_req).await?;
        if let Some(reth_payload_id) = is_payload_id {
            let (mut ress_parts, ress_body) = ress_resp.into_parts();
            info!("reth payload id: {:?}", reth_payload_id);
            let ress_body_bytes = hyper::body::to_bytes(ress_body).await?;
            let mut top_level: Value = serde_json::from_slice(&ress_body_bytes).unwrap();
            if let Some(result_obj) = top_level.get_mut("result") {
                if let Some(result_map) = result_obj.as_object_mut() {
                    result_map.insert(
                        "payloadId".to_string(),
                        serde_json::Value::String(reth_payload_id.to_string()),
                    );
                }
            }

            let new_body_bytes = serde_json::to_vec(&top_level).unwrap();

            // Remove or recalc content-length
            ress_parts.headers.remove("content-length");
            ress_parts.headers.insert(
                hyper::header::CONTENT_LENGTH,
                hyper::header::HeaderValue::from_str(&new_body_bytes.len().to_string()).unwrap(),
            );
            let new_response = Response::from_parts(ress_parts, Body::from(new_body_bytes));
            Ok(new_response)
        } else {
            info!("Received from Ress, not payload: {:?}", ress_resp);
            Ok(ress_resp)
        }
    } else {
        info!("Received from Reth, not engine: {:?}", reth_resp);
        Ok(reth_resp)
    }
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
