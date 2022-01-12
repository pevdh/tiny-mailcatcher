# Tiny MailCatcher

Tiny MailCatcher is a _tiny_ (<6 MB) drop-in [MailCatcher](https://mailcatcher.me/) replacement 
optimized for size and speed. It's meant to be run in a resource constrained environment, such 
as a CI system, or as part of an automated test suite.

Some features that the original MailCatcher has were omitted: Tiny MailCatcher does not have 
an HTML + JavaScript UI and it does not publish a WebSocket endpoint at /messages (yet). 
If you need either of these features I strongly recommend installing the original MailCatcher.

When Tiny MailCatcher is running any emails sent to port 1025 (configurable) can be accessed
and analysed via a REST API. This is useful in a test suite when asserting that certain emails
are or are not sent.

## Documentation

The documentation at [https://mailcatcher.me/](https://mailcatcher.me/) also applies to Tiny 
MailCatcher.

In short:

1. Tiny MailCatcher has an SMTP server which runs on port 1025 by default (configurable)
2. There is a REST API that runs on port 1080 (configurable).
    - `GET/DELETE http://localhost:1080/messages` retrieves/deletes all messages
    - `GET http://localhost:1080/messages/:id.json` retrieve a single message in JSON format
    - `GET http://localhost:1080/messages/:id.source` retrieve the message source
    - `GET http://localhost:1080/messages/:id.html` retrieve the HTML version of this message
    - `GET http://localhost:1080/messages/:id.eml` retrieve the EML version of this message
    - `GET http://localhost:1080/messages/:id.plain` retrieve the text/plain version of this message
    - `DELETE http://localhost:1080/messages/:id` delete a message
    - `GET http://localhost:1080/messages/:id/parts/:cid` retrieve attachments by content ID

## Installation

Using Cargo:

```bash
cargo install tiny-mailcatcher
```

There is also a tiny (~5MB) Docker image available:

```bash
docker run -d -p 1080:80 -p 1025:25 pevdh/tiny-mailcatcher:latest
```

## Usage

```
USAGE:
    tiny-mailcatcher [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --http-port <http-port>     [default: 1080]
        --ip <ip>                   [default: 127.0.0.1]
        --smtp-port <smtp-port>     [default: 1025]
```