use log::info;
use std::error::Error;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use structopt::StructOpt;
use tiny_mailcatcher::repository::MessageRepository;
use tiny_mailcatcher::{http, smtp};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    env_logger::init();

    let args: Options = Options::from_args();

    let repository = Arc::new(Mutex::new(MessageRepository::new()));

    info!("Tiny MailCatcher is starting");
    let http_addr = format!("{}:{}", &args.ip, args.http_port);
    let http_listener = TcpListener::bind(http_addr).unwrap();
    let http_handle = tokio::spawn(http::run_http_server(http_listener, repository.clone()));

    let smtp_addr = format!("{}:{}", &args.ip, args.smtp_port);
    let smtp_listener = TcpListener::bind(smtp_addr).unwrap();
    let smtp_handle = tokio::spawn(smtp::run_smtp_server(smtp_listener, repository.clone()));

    let (http_res, smtp_res) = tokio::try_join!(http_handle, smtp_handle)?;

    http_res.and(smtp_res)
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
