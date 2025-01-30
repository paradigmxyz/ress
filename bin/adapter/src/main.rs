use futures_util::TryFutureExt;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Client, Request, Response, Server,
};
use serde_json::Value;
use std::convert::Infallible;
use tokio::io::{self};
use tracing::{error, info, Level};

const RETH_AUTH: &str = "http://127.0.0.1:8651";
const RETH_HTTP: &str = "http://127.0.0.1:8544";

const RESS_AUTH: &str = "http://127.0.0.1:8552";

async fn forward_request(
    req: Request<Body>,
    is_auth_server: bool,
) -> Result<Response<Body>, hyper::Error> {
    let (parts, body) = req.into_parts();
    let whole_body = hyper::body::to_bytes(body).await?;
    let body_str = String::from_utf8_lossy(&whole_body);

    // todo: handle it as enum
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

    // If it's an engine method, send request to Ress and await its response
    if is_engine_method {
        let ress_req = build_request(RESS_AUTH).unwrap();
        info!("Sending request to Ress: {:?}", ress_req);

        let ress_resp = client.request(ress_req).await?;
        info!("Received response from Ress: {:?}", ress_resp);

        let body = hyper::body::to_bytes(ress_resp.into_body()).await.unwrap();
        info!(
            "Ress Response: Status:, Body: {} ",
            String::from_utf8_lossy(&body)
        );
    }

    Ok(reth_resp)
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
