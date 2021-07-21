use crate::email::parse_message;
use crate::repository::MessageRepository;
use log::{debug, info};
use std::net::IpAddr;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn run_smtp_server<R>(
    tcp_listener: StdTcpListener,
    repository: Arc<Mutex<R>>,
) -> Result<()>
where
    R: 'static + MessageRepository + Send,
{
    info!(
        "Starting SMTP server on {}",
        tcp_listener.local_addr().unwrap()
    );

    tcp_listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::from_std(tcp_listener)?;

    loop {
        let (socket, remote_ip) = listener.accept().await?;
        let session_repository = Arc::clone(&repository);

        tokio::spawn(async move {
            let server_impl = SmtpServer::new(session_repository);
            let mut protocol = SmtpProtocol::new(socket, remote_ip.ip(), server_impl);

            protocol.execute().await;
        });
    }
}

#[derive(PartialEq)]
enum SmtpProtocolState {
    ReceivedEhlo,
    ReceivedSender,
    ReceivedRecipients,
    ReceivedDataCommand,
}

struct SmtpProtocol<I>
where
    I: SmtpServerImplementation,
{
    processing_data: bool,
    server_impl: I,
    remote_addr: IpAddr,
    stream: BufReader<TcpStream>,
    state: Vec<SmtpProtocolState>,
}

impl<I: SmtpServerImplementation> SmtpProtocol<I> {
    fn new(stream: TcpStream, remote_addr: IpAddr, server_impl: I) -> Self {
        SmtpProtocol {
            processing_data: false,
            server_impl,
            remote_addr,
            stream: BufReader::new(stream),
            state: vec![],
        }
    }

    async fn execute(&mut self) {
        let mut line_buf = String::new();

        self.say(format!("220 {}\r\n", self.server_impl.server_greeting()).as_str())
            .await;

        loop {
            line_buf.clear();
            self.stream.read_line(&mut line_buf).await.unwrap();

            let mut line = line_buf.as_bytes();

            if self.processing_data && line == b".\r\n" {
                self.processing_data = false;

                if self.server_impl.data_end().is_ok() {
                    self.say("250 Message accepted\r\n").await;
                } else {
                    self.say("550 Message rejected\r\n").await;
                }
                // Remove these three states
                self.state
                    .retain(|s| s != &SmtpProtocolState::ReceivedDataCommand);
                self.state
                    .retain(|s| s != &SmtpProtocolState::ReceivedSender);
                self.state
                    .retain(|s| s != &SmtpProtocolState::ReceivedRecipients);

                continue;
            }

            if self.processing_data {
                // skip leading "." if it exists
                if line.starts_with(b".") {
                    line = &line[1..];
                }

                self.server_impl.data(line).unwrap();

                continue;
            }

            // slice off trailing \r\n
            if line.ends_with(b"\r\n") {
                line = &line[..(line.len() - 2)];
            }

            // SAFETY: read_line already checks whether line is valid UTF-8
            debug!(">>> {}", std::str::from_utf8(line).unwrap());
            let line_upper = line.to_ascii_uppercase();

            let mut quit = false;
            if line_upper.starts_with(b"EHLO") {
                let domain = line_buf["EHLO".len()..].trim();
                if self.server_impl.helo(self.remote_addr, domain).is_ok() {
                    self.say(format!("250-{}\r\n", self.server_impl.server_domain()).as_str())
                        .await;
                    self.say("250-NO-SOLLICITING\r\n").await;
                    self.say("250 SIZE 20000000\r\n").await;
                    self.reset();
                    self.state.push(SmtpProtocolState::ReceivedEhlo);
                } else {
                    self.say("550 Requested action not taken\r\n").await;
                }
            } else if line_upper.starts_with(b"HELO") {
                let domain = line_buf["HELO".len()..].trim();
                if self.server_impl.helo(self.remote_addr, domain).is_ok() {
                    self.say(format!("250-{}\r\n", self.server_impl.server_domain()).as_str())
                        .await;
                    self.reset();
                    self.state.push(SmtpProtocolState::ReceivedEhlo);
                } else {
                    self.say("550 Requested action not taken\r\n").await;
                }
            } else if line_upper.starts_with(b"QUIT") {
                self.say("220 Ok\r\n").await;
                quit = true;
            } else if line_upper.starts_with(b"MAIL FROM:") {
                if self.state.contains(&SmtpProtocolState::ReceivedSender) {
                    self.say("503 MAIL already given\r\n").await;
                } else {
                    let sender = line_buf["MAIL FROM:".len()..].trim();

                    if self.server_impl.mail(sender).is_ok() {
                        self.say("250 Ok\r\n").await;
                        self.state.push(SmtpProtocolState::ReceivedSender);
                    } else {
                        self.say("550 sender is unacceptable\r\n").await;
                    }
                }
            } else if line_upper.starts_with(b"RCPT TO:") {
                if self.state.contains(&SmtpProtocolState::ReceivedSender) {
                    let recipient = line_buf["RCPT TO:".len()..].trim();

                    if self.server_impl.rcpt(recipient).is_ok() {
                        self.say("250 Ok\r\n").await;
                        self.state.push(SmtpProtocolState::ReceivedRecipients);
                    } else {
                        self.say("550 recipient is unacceptable\r\n").await;
                    }
                } else {
                    self.say("503 MAIL is required before RCPT\r\n").await;
                }
            } else if line_upper.starts_with(b"DATA") {
                if self.state.contains(&SmtpProtocolState::ReceivedRecipients) {
                    self.say("354 Send it\r\n").await;
                    self.state.push(SmtpProtocolState::ReceivedDataCommand);
                    self.processing_data = true;
                } else {
                    self.say("503 Operation sequence error\r\n").await;
                }
            } else if line_upper.starts_with(b"NOOP") {
                self.say("250 Ok\r\n").await;
            } else if line_upper.starts_with(b"RSET") {
                self.reset();
                self.say("250 Ok\r\n").await;
            } else {
                // Unknown command
                self.say("500 Unknown command\r\n").await;
            }

            if quit {
                break;
            }
        }
    }

    async fn say(&mut self, message: &str) {
        debug!("<<< {}", message);
        self.stream.write_all(message.as_bytes()).await.unwrap();
    }

    fn reset(&mut self) {
        self.state.clear();
        self.server_impl.reset();
    }
}

trait SmtpServerImplementation {
    fn reset(&mut self) {}

    fn server_greeting(&self) -> String {
        "Hello from Tiny MailCatcher's simple and dumb SMTP server".to_string()
    }

    fn server_domain(&self) -> String {
        "Tiny MailCatcher - do not expect too much from me please".to_string()
    }

    fn helo(&mut self, _ip: IpAddr, _domain: &str) -> Result<()> {
        Ok(())
    }

    fn mail(&mut self, _sender: &str) -> Result<()> {
        Ok(())
    }

    fn rcpt(&mut self, _recipient: &str) -> Result<()> {
        Ok(())
    }

    fn data(&mut self, _buf: &[u8]) -> Result<()> {
        Ok(())
    }

    fn data_end(&mut self) -> Result<()> {
        Ok(())
    }
}

struct SmtpServer<R: MessageRepository + Send> {
    sender: Option<String>,
    recipients: Vec<String>,
    data: Vec<u8>,
    repository: Arc<Mutex<R>>,
}

impl<R: MessageRepository + Send> SmtpServer<R> {
    fn new(repository: Arc<Mutex<R>>) -> Self {
        SmtpServer {
            sender: None,
            recipients: Vec::new(),
            data: Vec::new(),
            repository,
        }
    }
}

impl<R: MessageRepository + Send> SmtpServerImplementation for SmtpServer<R> {
    fn reset(&mut self) {
        self.sender = None;
        self.recipients = Vec::new();
        self.data = Vec::new();
    }

    fn mail(&mut self, sender: &str) -> Result<()> {
        self.sender = Some(sender.to_string());

        Ok(())
    }

    fn rcpt(&mut self, recipient: &str) -> Result<()> {
        self.recipients.push(recipient.to_string());

        Ok(())
    }

    fn data(&mut self, buf: &[u8]) -> Result<()> {
        self.data.extend_from_slice(buf);

        Ok(())
    }

    fn data_end(&mut self) -> Result<()> {
        // strip size extension
        let size_extension_index = self.sender.as_ref().and_then(|s| s.rfind("SIZE="));

        let sender = if let Some(idx) = size_extension_index {
            self.sender.as_ref().map(|s| s[..idx].to_string())
        } else {
            self.sender.clone()
        };

        let message = parse_message(&sender, &self.recipients, &self.data).unwrap();

        info!(
            "Received message from {}",
            &sender.as_deref().unwrap_or("unknown sender")
        );

        self.repository.lock().unwrap().persist(message);

        Ok(())
    }
}
