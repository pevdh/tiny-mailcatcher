use lettre::{AsyncSmtpTransport, AsyncTransport, Message as EmailMessage, Tokio1Executor};
use std::net::{SocketAddr, TcpListener};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tiny_mailcatcher::repository::MessageRepository;
use tiny_mailcatcher::smtp::run_smtp_server;
use tokio::task::JoinHandle;

fn launch_test_smtp_server(repository: Arc<Mutex<MessageRepository>>, port: u16) -> JoinHandle<()> {
    let addr = SocketAddr::from_str(format!("127.0.0.1:{}", port).as_str()).unwrap();
    let tcp_listener = TcpListener::bind(addr).unwrap();

    tokio::spawn(async move {
        run_smtp_server(tcp_listener, repository.clone())
            .await
            .unwrap()
    })
}

async fn send_mail(repository: Arc<Mutex<MessageRepository>>, sender: &str, recipient: &str) {
    let port = 62044;
    let server = launch_test_smtp_server(repository, port);

    let email = EmailMessage::builder()
        .from(sender.parse().unwrap())
        .to(recipient.parse().unwrap())
        .subject("Hello there")
        .body(String::from("Abc123"))
        .unwrap();

    let mailer = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous("127.0.0.1")
        .port(port)
        .build();

    mailer.send(email).await.unwrap();

    server.abort();
}

#[tokio::test]
async fn test_smtp_server_is_reachable() {
    let repository = Arc::new(Mutex::new(MessageRepository::new()));

    send_mail(
        repository.clone(),
        "test@example.com",
        "recipient@example.com",
    )
    .await;

    let repository = repository.lock().unwrap();
    assert_eq!(1, repository.find_all().len());
    assert_eq!(
        Some("<test@example.com>".to_string()),
        repository.find(1).unwrap().sender
    );
}
