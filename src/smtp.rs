use crate::email::parse_message;
use crate::repository::MessageRepository;
use log::{info, warn};
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn run_smtp_server(
    tcp_listener: StdTcpListener,
    repository: Arc<Mutex<MessageRepository>>,
) -> Result<()> {
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
            let server_impl = SmtpServerImplementation::new(session_repository);
            let mut protocol = SmtpProtocol::new(socket, server_impl);

            match protocol.execute().await {
                Ok(_) => {}
                Err(e) => {
                    warn!(
                        "An error occurred while executing the SMTP protocol with {}: {}",
                        remote_ip, e
                    );
                }
            };
        });
    }
}

#[derive(PartialEq)]
enum SmtpProtocolState {
    ReceivedEhlo,
    ReceivedSender,
    ReceivedRecipients,
}

struct SmtpProtocol {
    server_impl: SmtpServerImplementation,
    stream: SmtpStream,
    state: Vec<SmtpProtocolState>,
}

impl SmtpProtocol {
    fn new(stream: TcpStream, server_impl: SmtpServerImplementation) -> Self {
        SmtpProtocol {
            server_impl,
            stream: SmtpStream::new(stream),
            state: vec![],
        }
    }

    async fn execute(&mut self) -> Result<()> {
        self.say("220 Hello from Tiny MailCatcher's simple and dumb SMTP server\r\n")
            .await?;

        loop {
            let line = self.stream.read_line().await?;
            if line.is_none() {
                // EOF
                return Ok(());
            }

            let line = std::str::from_utf8(line.unwrap());
            if line.is_err() {
                self.say("500 Invalid UTF-8 encountered\r\n").await?;

                continue;
            }

            let line = line.unwrap();
            let line_upper = line.to_ascii_uppercase();

            if line_upper.starts_with("EHLO") {
                let message = "\
                    250-Tiny MailCatcher - do not expect too much from me please\r\n\
                    250-NO-SOLLICITING\r\n\
                    250 SIZE 20000000\r\n";

                self.say(message).await?;
                self.reset();
                self.state.push(SmtpProtocolState::ReceivedEhlo);
            } else if line_upper.starts_with("HELO") {
                self.say("250-Tiny MailCatcher - do not expect too much from me please\r\n")
                    .await?;
                self.reset();
                self.state.push(SmtpProtocolState::ReceivedEhlo);
            } else if line_upper.starts_with("QUIT") {
                self.say("220 Ok\r\n").await?;
                return Ok(());
            } else if line_upper.starts_with("MAIL FROM:") {
                if self.state.contains(&SmtpProtocolState::ReceivedSender) {
                    self.say("503 MAIL already given\r\n").await?;
                } else {
                    let sender = line["MAIL FROM:".len()..].trim();
                    self.server_impl.mail(sender);
                    self.say("250 Ok\r\n").await?;
                    self.state.push(SmtpProtocolState::ReceivedSender);
                }
            } else if line_upper.starts_with("RCPT TO:") {
                if self.state.contains(&SmtpProtocolState::ReceivedSender) {
                    let recipient = line["RCPT TO:".len()..].trim();
                    self.server_impl.rcpt(recipient);
                    self.state.push(SmtpProtocolState::ReceivedRecipients);
                    self.say("250 Ok\r\n").await?;
                } else {
                    self.say("503 MAIL is required before RCPT\r\n").await?;
                }
            } else if line_upper.starts_with("DATA") {
                if self.state.contains(&SmtpProtocolState::ReceivedRecipients) {
                    self.say("354 Send it\r\n").await?;

                    let data = self.stream.read_data().await?;
                    if data.is_none() {
                        return Ok(());
                    }

                    self.server_impl.data(data.unwrap())?;

                    self.say("250 Message accepted\r\n").await?;

                    // Remove these two states
                    self.state
                        .retain(|s| s != &SmtpProtocolState::ReceivedSender);
                    self.state
                        .retain(|s| s != &SmtpProtocolState::ReceivedRecipients);
                } else {
                    self.say("503 Operation sequence error\r\n").await?;
                }
            } else if line_upper.starts_with("NOOP") {
                self.say("250 Ok\r\n").await?;
            } else if line_upper.starts_with("RSET") {
                self.reset();
                self.say("250 Ok\r\n").await?;
            } else {
                // Unknown command
                self.say("500 Unknown command\r\n").await?;
            }
        }
    }

    async fn say(&mut self, message: &str) -> Result<()> {
        self.stream.write_all(message.as_bytes()).await?;

        Ok(())
    }

    fn reset(&mut self) {
        self.state.clear();
        self.server_impl.reset();
    }
}

struct SmtpStream {
    inner: TcpStream,
    line_buffer: Vec<u8>,
    data_buffer: Vec<u8>,
}

impl SmtpStream {
    fn new(stream: TcpStream) -> Self {
        SmtpStream {
            inner: stream,
            line_buffer: Vec::with_capacity(8 * 1024),
            data_buffer: Vec::with_capacity(64 * 1024),
        }
    }

    async fn write_all(&mut self, message: &[u8]) -> Result<()> {
        self.inner.write_all(message).await?;

        Ok(())
    }

    async fn read_line(&mut self) -> Result<Option<&[u8]>> {
        self.line_buffer.clear();

        loop {
            let mut read_buffer = [0u8; 8 * 1024];
            let bytes_read = self.inner.read(&mut read_buffer).await?;

            if bytes_read == 0 {
                return Ok(None);
            }

            if let Some(i) = memchr::memchr(b'\n', &read_buffer[..bytes_read]) {
                self.line_buffer.extend_from_slice(&read_buffer[0..=i]);

                // slice off \r\n
                return Ok(Some(&self.line_buffer[..self.line_buffer.len() - 2]));
            } else {
                self.line_buffer
                    .extend_from_slice(&read_buffer[..bytes_read]);
            }
        }
    }

    async fn read_data(&mut self) -> Result<Option<&[u8]>> {
        self.data_buffer.clear();

        loop {
            let mut read_buffer = [0u8; 16 * 1024];
            let bytes_read = self.inner.read(&mut read_buffer).await?;

            if bytes_read == 0 {
                return Ok(None);
            }

            self.data_buffer
                .extend_from_slice(&read_buffer[..bytes_read]);

            let data_end_delimiter = b"\r\n.\r\n";
            if self.data_buffer.len() >= data_end_delimiter.len()
                && &self.data_buffer[self.data_buffer.len() - data_end_delimiter.len()..]
                    == data_end_delimiter
            {
                return Ok(Some(
                    &self.data_buffer[..self.data_buffer.len() - data_end_delimiter.len() + 2],
                ));
            }
        }
    }
}

struct SmtpServerImplementation {
    sender: Option<String>,
    recipients: Vec<String>,
    repository: Arc<Mutex<MessageRepository>>,
}

impl SmtpServerImplementation {
    fn new(repository: Arc<Mutex<MessageRepository>>) -> Self {
        SmtpServerImplementation {
            sender: None,
            recipients: Vec::new(),
            repository,
        }
    }

    fn reset(&mut self) {
        self.sender = None;
        self.recipients = Vec::new();
    }

    fn mail(&mut self, sender: &str) {
        self.sender = Some(sender.to_string());
    }

    fn rcpt(&mut self, recipient: &str) {
        self.recipients.push(recipient.to_string());
    }

    fn data(&mut self, buf: &[u8]) -> Result<()> {
        // strip size extension
        let size_extension_index = self.sender.as_ref().and_then(|s| s.rfind("SIZE="));

        let sender = if let Some(idx) = size_extension_index {
            self.sender.as_ref().map(|s| s[..idx].to_string())
        } else {
            self.sender.clone()
        };

        let message = parse_message(&sender, &self.recipients, buf);
        if message.is_err() {
            return Err(Box::new(message.err().unwrap()));
        }
        let message = message.unwrap();

        info!(
            "Received message from {} ({} bytes)",
            sender.as_deref().unwrap_or("unknown sender"),
            buf.len(),
        );

        self.repository.lock().unwrap().persist(message);

        Ok(())
    }
}
