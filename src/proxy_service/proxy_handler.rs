use hyper::{body::Incoming, Request, Response, StatusCode, Uri};
use hyper_rustls::{ConfigBuilderExt, HttpsConnector};
use hyper_util::{client::legacy::{connect::HttpConnector, Client}, rt::{TokioExecutor, TokioIo}};
use tokio::time::timeout;
use tokio_rustls::rustls;
use std::sync::Arc;

use crate::load_balancer::factory::LoadBalancer;

use super::gateway_body::GatewayBody;

type HttpClient = Client<HttpsConnector<HttpConnector>, GatewayBody>;

pub struct ProxyHandler {
    pub client: HttpClient,
    pub load_balancer: Arc<dyn LoadBalancer>,
}

impl ProxyHandler {
    pub fn new(balancer: Arc<dyn LoadBalancer>) -> Self {
        let c = rustls::ClientConfig::builder()
            .with_native_roots().unwrap()
            .with_no_client_auth();

        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(c)
            .https_or_http()
            .enable_http1()
            .build();

        let client: Client<_, GatewayBody> = Client::builder(TokioExecutor::new()).build(https);

        Self {
            client: client,
            load_balancer: balancer,
        }
    }

    pub async fn handle(&self, mut req: Request<Incoming>) -> Response<GatewayBody> {

        let selected_lb = self.load_balancer.next().await;
        
        match selected_lb {
            Some(backend) => {
                let backend_uri = self.build_backend_uri(&req, &backend.server);
                *req.uri_mut() = backend_uri;

                self.proxy_request(req).await
            },
            None => Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(GatewayBody::Empty)
                .unwrap(),
        }
    }

    fn build_backend_uri(&self, req: &Request<Incoming>, backend: &str) -> Uri {
        let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
        format!("{}{}", backend, path).parse().unwrap()
    }

    async fn proxy_request(&self ,req: Request<Incoming>) -> Response<GatewayBody> {
        let timeout_duration = std::time::Duration::from_secs(5);

        let new_req = Request::builder()
            .method(req.method())
            .uri(req.uri())
            .body(GatewayBody::from(GatewayBody::Incomming(req.into_body())))
            .unwrap();

        match timeout(timeout_duration, self.client.request(new_req)).await {
            Ok(Ok(res)) => {
                let (parts, body) = res.into_parts();
                let body = GatewayBody::from(GatewayBody::Incomming(body));
                Response::from_parts(parts, body)
            },
            Ok(Err(_)) => Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(GatewayBody::Empty)
                .unwrap(),
            Err(_) => Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(GatewayBody::Empty)
                .unwrap(),
        }
    }
}