use crate::repository::MessageRepository;
use hyper::http::HeaderValue;
use hyper::{header, Body, Request, Response, Server, StatusCode};
use log::info;
use routerify::ext::RequestExt;
use routerify::{Middleware, Router, RouterService};
use serde::Serialize;
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

async fn add_cors_headers(mut res: Response<Body>) -> Result<Response<Body>, io::Error> {
    let headers = res.headers_mut();

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("*"),
    );

    Ok(res)
}

struct State {
    repository: Arc<Mutex<MessageRepository>>,
}

fn router(repository: Arc<Mutex<MessageRepository>>) -> Router<Body, io::Error> {
    let state = State { repository };

    Router::builder()
        .middleware(Middleware::post(add_cors_headers))
        .data(state)
        .delete("/messages/:id", delete_message)
        .get("/messages/:id.source", get_message_source)
        .get("/messages/:id.html", get_message_html)
        .get("/messages/:id.eml", get_message_eml)
        .get("/messages/:id.plain", get_message_plain)
        .get("/messages/:id/parts/:cid", get_message_part)
        .get("/messages/:id", get_message_json)
        .get("/messages", get_messages)
        .delete("/messages", delete_messages)
        .options("/*", options_handler)
        .any(handler_404)
        .build()
        .unwrap()
}

pub async fn run_http_server(
    tcp_listener: TcpListener,
    repository: Arc<Mutex<MessageRepository>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "Starting HTTP server on {}",
        tcp_listener.local_addr().unwrap()
    );

    let router = router(repository);
    let service = RouterService::new(router).unwrap();

    let server = Server::from_tcp(tcp_listener).unwrap().serve(service);

    server.await?;

    Ok(())
}

#[derive(Serialize)]
struct GetMessagesListItem {
    id: usize,
    sender: Option<String>,
    recipients: Vec<String>,
    subject: Option<String>,
    size: String,
    created_at: String,
}

async fn get_messages(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let repository = &req.data::<State>().unwrap().repository;

    let mut messages = vec![];
    for message in repository.lock().unwrap().find_all() {
        messages.push(GetMessagesListItem {
            id: message.id.unwrap(),
            sender: message.sender.clone(),
            recipients: message.recipients.clone(),
            subject: message.subject.clone(),
            size: message.size.to_string(),
            created_at: message.created_at.to_rfc3339(),
        })
    }

    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&messages).unwrap().into())
        .unwrap())
}

async fn delete_messages(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let repository = &req.data::<State>().unwrap().repository;

    repository.lock().unwrap().delete_all();

    Ok(Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap())
}

#[derive(Serialize)]
struct GetMessage {
    id: usize,
    sender: Option<String>,
    recipients: Vec<String>,
    subject: Option<String>,
    size: String,

    #[serde(rename = "type")]
    ty: String,
    created_at: String,
    formats: Vec<String>,
    attachments: Vec<GetMessageAttachment>,
}

#[derive(Serialize)]
struct GetMessageAttachment {
    pub cid: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub filename: String,
    pub size: usize,
    pub href: String,
}

async fn get_message_json(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let message = repository.lock().unwrap().find(id).map(|message| {
        let mut formats = vec!["source".to_string()];
        if message.html().is_some() {
            formats.push("html".to_string());
        }

        if message.plain().is_some() {
            formats.push("plain".to_string());
        }

        GetMessage {
            id,
            sender: message.sender.clone(),
            recipients: message.recipients.clone(),
            subject: message.subject.clone(),
            size: message.size.to_string(),
            ty: message.typ.clone(),
            created_at: message.created_at.to_rfc3339(),
            formats,
            attachments: message
                .parts
                .iter()
                .filter(|p| p.is_attachment)
                .map(|attachment| GetMessageAttachment {
                    cid: attachment.cid.clone(),
                    typ: attachment.typ.clone(),
                    filename: attachment.filename.clone(),
                    size: attachment.size,
                    href: format!("/messages/{}/parts/{}", id, attachment.cid),
                })
                .collect(),
        }
    });

    if let Some(message) = message {
        Ok(Response::builder()
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&message).unwrap().into())
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    }
}

async fn get_message_html(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let repository = repository.lock().unwrap();
    let html_part = repository.find(id).and_then(|message| message.html());

    return if let Some(html_part) = html_part {
        Ok(Response::builder()
            .header(
                "Content-Type",
                format!("text/html; charset={}", html_part.charset),
            )
            .body(Body::from(html_part.body.clone()))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    };
}

async fn get_message_plain(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let repository = repository.lock().unwrap();
    let html_part = repository.find(id).and_then(|message| message.plain());

    return if let Some(html_part) = html_part {
        Ok(Response::builder()
            .header(
                "Content-Type",
                format!("text/plain; charset={}", html_part.charset),
            )
            .body(Body::from(html_part.body.clone()))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    };
}

async fn get_message_source(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let repository = repository.lock().unwrap();
    let message = repository.find(id);

    return if let Some(message) = message {
        Ok(Response::builder()
            .header(
                "Content-Type",
                format!("text/plain; charset={}", message.charset),
            )
            .body(Body::from(message.source.clone()))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    };
}

async fn get_message_eml(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let repository = repository.lock().unwrap();
    let message = repository.find(id);

    return if let Some(message) = message {
        Ok(Response::builder()
            .header(
                "Content-Type",
                format!("message/rfc822; charset={}", message.charset),
            )
            .body(Body::from(message.source.clone()))
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    };
}

async fn get_message_part(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let cid = req.param("cid").unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let repository = repository.lock().unwrap();
    let part = repository
        .find(id)
        .and_then(|message| message.parts.iter().find(|part| part.cid.as_str() == cid));

    return if let Some(part) = part {
        let mut response = Response::builder()
            .header(
                "Content-Type",
                format!("{}; charset={}", part.typ, part.charset),
            )
            .body(Body::from(part.body.clone()))
            .unwrap();

        if part.is_attachment {
            let content_disposition = HeaderValue::from_str(
                format!("attachment; filename=\"{}\"", part.filename).as_str(),
            )
            .unwrap();
            response
                .headers_mut()
                .insert("Content-Disposition", content_disposition);
        }

        Ok(response)
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    };
}

async fn delete_message(req: Request<Body>) -> Result<Response<Body>, io::Error> {
    let id: usize = req.param("id").unwrap().parse().unwrap();
    let repository = &req.data::<State>().unwrap().repository;

    let deleted_message = repository.lock().unwrap().delete(id);

    if deleted_message.is_some() {
        Ok(Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .unwrap())
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap())
    }
}

async fn handler_404(_: Request<Body>) -> Result<Response<Body>, io::Error> {
    Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Page Not Found"))
        .unwrap())
}

async fn options_handler(_req: Request<Body>) -> Result<Response<Body>, io::Error> {
    Ok(Response::new(Body::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{Message, MessagePart, MessageRepository};
    use chrono::{TimeZone, Utc};
    use hyper::{body, Client, Request, StatusCode};
    use hyper::{body::HttpBody, Server};
    use routerify::{Router, RouterService};
    use std::{
        net::SocketAddr,
        sync::{Arc, Mutex},
    };
    use tokio::sync::oneshot::{self, Sender};

    #[allow(dead_code)]
    pub struct Serve {
        addr: SocketAddr,
        tx: Sender<()>,
    }

    impl Serve {
        pub fn addr(&self) -> SocketAddr {
            self.addr
        }
    }

    pub async fn serve<B, E>(router: Router<B, E>) -> Serve
    where
        B: HttpBody + Send + Sync + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>> + 'static,
        <B as HttpBody>::Data: Send + Sync + 'static,
        <B as HttpBody>::Error: Into<Box<dyn std::error::Error + Send + Sync>> + 'static,
    {
        let service = RouterService::new(router).unwrap();
        let server = Server::bind(&([127, 0, 0, 1], 0).into()).serve(service);
        let addr = server.local_addr();

        let (tx, rx) = oneshot::channel::<()>();

        let graceful_server = server.with_graceful_shutdown(async {
            rx.await.unwrap();
        });

        tokio::spawn(async move {
            graceful_server.await.unwrap();
        });

        Serve { addr, tx }
    }

    async fn body_to_string(body: Body) -> String {
        return String::from_utf8(body::to_bytes(body).await.unwrap().to_vec()).unwrap();
    }

    async fn body_to_json(body: Body) -> serde_json::Value {
        return serde_json::from_str(body_to_string(body).await.as_str()).unwrap();
    }

    fn create_test_message() -> Message {
        Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "text/plain".to_string(),
            parts: vec![],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        }
    }

    #[tokio::test]
    async fn test_get_messages_returns_messages_in_repository() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(create_test_message());

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!("http://{}/{}", serve.addr(), "messages"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let expected = serde_json::json!([
            {
                "created_at": "2015-05-15T00:00:00+00:00",
                "id": 1,
                "recipients": ["recipient@example.com"],
                "sender": "sender@example.com",
                "size": "42",
                "subject": "This is the subject",
            }
        ]);

        assert_eq!(StatusCode::OK, res.status());
        assert_eq!(expected, body_to_json(res.into_body()).await);
    }

    #[tokio::test]
    async fn test_delete_messages_clears_repository() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(create_test_message());

        let router = router(Arc::clone(&repository));

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("http://{}/{}", serve.addr(), "messages"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let repository = Arc::clone(&repository);
        let handle = repository.lock().unwrap();
        let repository_messages = handle.find_all();

        // let repository = &res.data().repository;
        let expected_messages: Vec<&Message> = vec![];
        assert_eq!(StatusCode::NO_CONTENT, res.status());
        assert_eq!(expected_messages, repository_messages);
    }

    #[tokio::test]
    async fn test_get_message_json() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(create_test_message());

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!("http://{}/{}", serve.addr(), "messages/1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(StatusCode::OK, res.status());

        let expected_message = serde_json::json!({
            "created_at": "2015-05-15T00:00:00+00:00",
            "id": 1,
            "recipients": ["recipient@example.com"],
            "sender": "sender@example.com",
            "size": "42",
            "subject": "This is the subject",
            "attachments": [],
            "formats": ["source"],
            "type": "text/plain",
        });
        assert_eq!(expected_message, body_to_json(res.into_body()).await);
    }

    #[tokio::test]
    async fn test_get_message_html() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "text/plain".to_string(),
            parts: vec![MessagePart {
                cid: "some-id".to_string(),
                typ: "text/html".to_string(),
                filename: "some_html_abc123".to_string(),
                size: 422,
                charset: "UTF-8".to_string(),
                body: b"<html>Hello world</html".to_vec(),
                is_attachment: false,
            }],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        });

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!("http://{}/{}", serve.addr(), "messages/1.html"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(StatusCode::OK, res.status());
        assert_eq!(
            "<html>Hello world</html",
            body_to_string(res.into_body()).await
        );
    }

    #[tokio::test]
    async fn test_get_message_plain() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "multipart/mixed".to_string(),
            parts: vec![MessagePart {
                cid: "some-id".to_string(),
                typ: "text/plain".to_string(),
                filename: "some_html_abc123".to_string(),
                size: 422,
                charset: "UTF-8".to_string(),
                body: b"This is some plaintext".to_vec(),
                is_attachment: false,
            }],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        });

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!("http://{}/{}", serve.addr(), "messages/1.plain"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(StatusCode::OK, res.status());
        assert_eq!(
            "This is some plaintext",
            body_to_string(res.into_body()).await
        );
    }

    #[tokio::test]
    async fn test_get_message_source() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "multipart/mixed".to_string(),
            parts: vec![MessagePart {
                cid: "some-id".to_string(),
                typ: "text/plain".to_string(),
                filename: "some_html_abc123".to_string(),
                size: 422,
                charset: "UTF-8".to_string(),
                body: b"This is some plaintext".to_vec(),
                is_attachment: false,
            }],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        });

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!("http://{}/{}", serve.addr(), "messages/1.source"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(StatusCode::OK, res.status());
        assert_eq!(
            "Subject: This is the subject\r\n\r\nHello world!\r\n",
            body_to_string(res.into_body()).await
        );
    }

    #[tokio::test]
    async fn test_get_message_eml() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "multipart/mixed".to_string(),
            parts: vec![MessagePart {
                cid: "some-id".to_string(),
                typ: "text/plain".to_string(),
                filename: "some_html_abc123".to_string(),
                size: 422,
                charset: "UTF-8".to_string(),
                body: b"This is some plaintext".to_vec(),
                is_attachment: false,
            }],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        });

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!("http://{}/{}", serve.addr(), "messages/1.eml"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(StatusCode::OK, res.status());
        assert_eq!(
            "message/rfc822; charset=UTF-8",
            res.headers().get("Content-Type").unwrap().to_str().unwrap()
        );
        assert_eq!(
            "Subject: This is the subject\r\n\r\nHello world!\r\n",
            body_to_string(res.into_body()).await
        );
    }

    #[tokio::test]
    async fn test_get_message_part() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "multipart/mixed".to_string(),
            parts: vec![MessagePart {
                cid: "some-id".to_string(),
                typ: "text/plain".to_string(),
                filename: "some_textfile.txt".to_string(),
                size: 422,
                charset: "UTF-8".to_string(),
                body: b"This is some plaintext as an attachment".to_vec(),
                is_attachment: true,
            }],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        });

        let router = router(repository);

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "http://{}/{}",
                        serve.addr(),
                        "messages/1/parts/some-id"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(StatusCode::OK, res.status());
        assert_eq!(
            "attachment; filename=\"some_textfile.txt\"",
            res.headers()
                .get("Content-Disposition")
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert_eq!(
            "text/plain; charset=UTF-8",
            res.headers().get("Content-Type").unwrap().to_str().unwrap()
        );
        assert_eq!(
            "This is some plaintext as an attachment",
            body_to_string(res.into_body()).await
        );
    }

    #[tokio::test]
    async fn test_delete_message() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(Message {
            id: Some(1),
            size: 42,
            subject: Some("This is the subject".to_string()),
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            created_at: Utc.timestamp(1431648000, 0),
            typ: "multipart/mixed".to_string(),
            parts: vec![],
            charset: "UTF-8".to_string(),
            source: b"Subject: This is the subject\r\n\r\nHello world!\r\n".to_vec(),
        });

        let router = router(Arc::clone(&repository));

        let serve = serve(router).await;
        let res = Client::new()
            .request(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("http://{}/{}", serve.addr(), "messages/1"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let repository = Arc::clone(&repository);
        let handle = repository.lock().unwrap();

        let message = handle.find(1);

        assert_eq!(StatusCode::NO_CONTENT, res.status());
        assert_eq!(None, message);
    }
}
