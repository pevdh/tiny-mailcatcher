use crate::repository::MessageRepository;
use hyper::http::HeaderValue;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use log::info;
use regex::Regex;
use serde::Serialize;
use std::convert::Infallible;
use std::net::TcpListener;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

pub async fn run_http_server(
    tcp_listener: TcpListener,
    repository: Arc<Mutex<MessageRepository>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "Starting HTTP server on {}",
        tcp_listener.local_addr().unwrap()
    );

    let server = Server::from_tcp(tcp_listener)
        .unwrap()
        .serve(make_service_fn(move |_conn| {
            let repository = repository.clone();

            async move {
                Ok::<_, Infallible>(service_fn(move |_req| {
                    let repository = repository.clone();

                    async move { Ok::<_, Infallible>(handle_request(repository, _req)) }
                }))
            }
        }));

    server.await?;

    Ok(())
}

pub fn handle_request(
    repository: Arc<Mutex<MessageRepository>>,
    req: Request<Body>,
) -> Response<Body> {
    info!("{} {}", req.method().as_str(), req.uri().path());

    let get_message_regex = Regex::new(r"^/messages/(\d+)(\.[a-z]+)?$").unwrap();
    let caps = get_message_regex.captures(req.uri().path());
    if let Some(caps) = caps {
        let id = usize::from_str(caps.get(1).unwrap().as_str()).unwrap();
        let format = caps.get(2).map(|c| c.as_str());

        return match (req.method(), format) {
            // GET /message/<id>.json
            (&Method::GET, Some(".json")) => get_message_json(repository, id),

            // GET /message/<id>.html
            (&Method::GET, Some(".html")) => get_message_html(repository, id),

            // GET /message/<id>.plain
            (&Method::GET, Some(".plain")) => get_message_plain(repository, id),

            // GET /message/<id>.source
            (&Method::GET, Some(".source")) => get_message_source(repository, id),

            // GET /message/<id>.eml
            (&Method::GET, Some(".eml")) => get_message_eml(repository, id),

            // DELETE /message/<id>
            (&Method::DELETE, None) => delete_message(repository, id),
            _ => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap(),
        };
    }

    let get_message_part_regex = Regex::new(r"^/messages/(\d+)/parts/(.+)").unwrap();
    let caps = get_message_part_regex.captures(req.uri().path());
    if let Some(caps) = caps {
        let id = usize::from_str(caps.get(1).unwrap().as_str()).unwrap();
        let cid = caps.get(2).unwrap().as_str();

        // GET /messages/<id>/parts/<cid>
        return get_message_part(repository, id, cid);
    }

    return match (req.method(), req.uri().path()) {
        // GET /messages
        (&Method::GET, "/messages") => get_messages(repository),

        // DELETE /messages
        (&Method::DELETE, "/messages") => delete_messages(repository),

        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    };
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

fn get_messages(repository: Arc<Mutex<MessageRepository>>) -> Response<Body> {
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

    Response::new(serde_json::to_string(&messages).unwrap().into())
}

fn delete_messages(repository: Arc<Mutex<MessageRepository>>) -> Response<Body> {
    repository.lock().unwrap().delete_all();

    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap()
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

fn get_message_json(repository: Arc<Mutex<MessageRepository>>, id: usize) -> Response<Body> {
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
        Response::new(serde_json::to_string(&message).unwrap().into())
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    }
}

fn get_message_html(repository: Arc<Mutex<MessageRepository>>, id: usize) -> Response<Body> {
    let repository = repository.lock().unwrap();
    let html_part = repository.find(id).and_then(|message| message.html());

    return if let Some(html_part) = html_part {
        Response::builder()
            .header(
                "Content-Type",
                format!("text/html; charset={}", html_part.charset),
            )
            .body(Body::from(html_part.body.clone()))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    };
}

fn get_message_plain(repository: Arc<Mutex<MessageRepository>>, id: usize) -> Response<Body> {
    let repository = repository.lock().unwrap();
    let html_part = repository.find(id).and_then(|message| message.plain());

    return if let Some(html_part) = html_part {
        Response::builder()
            .header(
                "Content-Type",
                format!("text/plain; charset={}", html_part.charset),
            )
            .body(Body::from(html_part.body.clone()))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    };
}

fn get_message_source(repository: Arc<Mutex<MessageRepository>>, id: usize) -> Response<Body> {
    let repository = repository.lock().unwrap();
    let message = repository.find(id);

    return if let Some(message) = message {
        Response::builder()
            .header(
                "Content-Type",
                format!("text/plain; charset={}", message.charset),
            )
            .body(Body::from(message.source.clone()))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    };
}

fn get_message_eml(repository: Arc<Mutex<MessageRepository>>, id: usize) -> Response<Body> {
    let repository = repository.lock().unwrap();
    let message = repository.find(id);

    return if let Some(message) = message {
        Response::builder()
            .header(
                "Content-Type",
                format!("message/rfc822; charset={}", message.charset),
            )
            .body(Body::from(message.source.clone()))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    };
}

fn get_message_part(
    repository: Arc<Mutex<MessageRepository>>,
    id: usize,
    cid: &str,
) -> Response<Body> {
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

        response
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    };
}

fn delete_message(repository: Arc<Mutex<MessageRepository>>, id: usize) -> Response<Body> {
    let deleted_message = repository.lock().unwrap().delete(id);

    if deleted_message.is_some() {
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{Message, MessagePart, MessageRepository};
    use chrono::{TimeZone, Utc};
    use hyper::{body, Body, Request, StatusCode};
    use std::sync::{Arc, Mutex};

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

        let req = Request::get("/messages").body(Body::empty()).unwrap();
        let res = handle_request(repository, req);

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

        let req = Request::delete("/messages").body(Body::empty()).unwrap();
        let res = handle_request(repository.clone(), req);

        let expected_messages: Vec<&Message> = vec![];
        assert_eq!(StatusCode::NO_CONTENT, res.status());
        assert_eq!(expected_messages, repository.lock().unwrap().find_all());
    }

    #[tokio::test]
    async fn test_get_message_json() {
        let repository = Arc::new(Mutex::new(MessageRepository::new()));
        repository.lock().unwrap().persist(create_test_message());

        let req = Request::get("/messages/1.json")
            .body(Body::empty())
            .unwrap();
        let res = handle_request(repository.clone(), req);

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

        let req = Request::get("/messages/1.html")
            .body(Body::empty())
            .unwrap();
        let res = handle_request(repository.clone(), req);

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

        let req = Request::get("/messages/1.plain")
            .body(Body::empty())
            .unwrap();
        let res = handle_request(repository.clone(), req);

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

        let req = Request::get("/messages/1.source")
            .body(Body::empty())
            .unwrap();
        let res = handle_request(repository.clone(), req);

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

        let req = Request::get("/messages/1.eml").body(Body::empty()).unwrap();
        let res = handle_request(repository.clone(), req);

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

        let req = Request::get("/messages/1/parts/some-id")
            .body(Body::empty())
            .unwrap();
        let res = handle_request(repository.clone(), req);

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

        let req = Request::delete("/messages/1").body(Body::empty()).unwrap();
        let res = handle_request(repository.clone(), req);

        assert_eq!(StatusCode::NO_CONTENT, res.status());
        assert_eq!(None, repository.lock().unwrap().find(1));
    }
}
