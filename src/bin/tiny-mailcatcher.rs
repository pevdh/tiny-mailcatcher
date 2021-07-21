use log::info;
use std::error::Error;
use std::net::{SocketAddr, TcpListener};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use structopt::StructOpt;
use tiny_mailcatcher::repository::InMemoryRepository;
use tiny_mailcatcher::{http, smtp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    env_logger::init();

    let args: Options = Options::from_args();

    let repository = Arc::new(Mutex::new(InMemoryRepository::new()));

    info!("Tiny MailCatcher is starting");
    let http_addr =
        SocketAddr::from_str(format!("{}:{}", &args.ip, args.http_port).as_str()).unwrap();
    let http_listener = TcpListener::bind(http_addr).unwrap();
    let http_fut = http::run_http_server(http_listener, repository.clone());

    let smtp_addr =
        SocketAddr::from_str(format!("{}:{}", &args.ip, args.smtp_port).as_str()).unwrap();
    let smtp_listener = TcpListener::bind(smtp_addr).unwrap();
    let smtp_fut = smtp::run_smtp_server(smtp_listener, repository.clone());

    tokio::try_join!(http_fut, smtp_fut)?;

    Ok(())
}

#[derive(Debug, StructOpt)]
#[structopt(name = "tiny-mailcatcher")]
struct Options {
    #[structopt(long, default_value = "127.0.0.1")]
    ip: String,

    #[structopt(long, name = "smtp-port", default_value = "1025")]
    smtp_port: u16,

    #[structopt(long, name = "http-port", default_value = "1080")]
    http_port: u16,
}
