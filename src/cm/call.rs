use std::{
    pin::Pin,
    task::{Context, Poll},
};

use crate::{options::FetchOptions, Error, ResponseBody};
use bytes::Bytes;
use futures_util::stream::Stream;
use http::{Request, Response};
use http_body_util::BodyExt;
use tonic::body::Body;
use wstd::{
    http::{body::IncomingBody, IntoBody as _},
    io::AsyncRead as _,
};


// The core async function that handles the request/response logic using wstd.
pub async fn call(
    base_url: String,
    request: Request<Body>,
    _options: Option<FetchOptions>,
) -> Result<Response<ResponseBody>, Error> {
    let url = format!("{}{}", base_url, request.uri().path());
    let (mut parts, body) = request.into_parts();

    assert!(parts.headers.remove("te").is_some()); // FIXME: Remove

    // 2. Aggregate the entire request body. This works well for unary calls.
    let body_bytes = body.collect().await?.to_bytes();

    // 3. Build the wstd HTTP request
    let mut wstd_request_builder = wstd::http::Request::builder()
        .uri(url)
        .method(wstd::http::Method::POST);
    for (key, value) in &parts.headers {
        wstd_request_builder = wstd_request_builder.header(key.as_str(), value.as_bytes());
    }
    let wstd_request = wstd_request_builder.body(body_bytes.into_body())?;

    // 4. Send the request using the wstd client
    let wstd_client = wstd::http::Client::new();
    let wstd_response = wstd_client
        .send(wstd_request)
        .await
        .map_err(Error::WstdHttp)?;

    // 5. Process the wstd response
    let status = wstd_response.status();
    let wstd_headers = wstd_response.headers().clone();

    // 6. Aggregate the entire response body.
    let response_body_bytes = wstd_response
        .into_body()
        .bytes()
        .await
        .map_err(Error::WstdHttp)?;

    // 7. Build the tonic-compatible HTTP response
    let mut response_builder = Response::builder()
        .status(status.as_u16())
        .version(http::Version::HTTP_2);
    for (key, value) in wstd_headers.iter() {
        if let Ok(h_value) = http::HeaderValue::from_bytes(value.as_bytes()) {
            response_builder = response_builder.header(key.as_str(), h_value);
        }
    }

    // 8. Wrap the response bytes in a `Full` body, which implements `http_body::Body`, and box it.
    let response_body = http_body_util::Full::new(bytes::Bytes::from(response_body_bytes)).boxed();
    response_builder
        .body(response_body)
        .map_err(Error::HttpError)
}
