use chrono::{TimeZone, Utc};
use reqwest::{Method, Response};
use std::net::{SocketAddr, TcpListener};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tiny_mailcatcher::http::run_http_server;
use tiny_mailcatcher::repository::{InMemoryRepository, Message, MessageRepository};
use tokio::task::JoinHandle;

fn create_repository_with_messages(messages: Vec<Message>) -> Arc<Mutex<InMemoryRepository>> {
    let mut repository = InMemoryRepository::new();

    for message in messages {
        repository.persist(message);
    }

    Arc::new(Mutex::new(repository))
}

fn launch_test_http_server(
    repository: Arc<Mutex<InMemoryRepository>>,
    port: u16,
) -> JoinHandle<()> {
    let addr = SocketAddr::from_str(format!("127.0.0.1:{}", port).as_str()).unwrap();
    let tcp_listener = TcpListener::bind(addr).unwrap();

    tokio::spawn(async move {
        run_http_server(tcp_listener, repository.clone())
            .await
            .unwrap()
    })
}

async fn do_request(
    repository: Arc<Mutex<InMemoryRepository>>,
    method: Method,
    uri: &str,
) -> Response {
    let server = launch_test_http_server(repository, 62043);
    let client = reqwest::Client::new();
    let req = client
        .request(method, "http://127.0.0.1:62043".to_string() + uri)
        .build()
        .unwrap();
    let resp = client.execute(req).await.unwrap();

    server.abort();

    resp
}

#[tokio::test]
async fn test_http_server_is_reachable() {
    let repository = create_repository_with_messages(vec![Message {
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
    }]);

    let res = do_request(repository, Method::GET, "/messages").await;
    let json = res.json::<serde_json::Value>().await.unwrap();

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
    assert_eq!(expected, json);
}
